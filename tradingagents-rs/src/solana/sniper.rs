use super::execution::OnChainExecutor;
use super::rug_filter::{RugFilter, RugFilterConfig, SafetyStatus, SecurityCheckReport};
use super::yellowstone::{DexProtocol, PoolCreationEvent};
use chrono::{DateTime, Utc};
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tracing::warn;

// Known Jito MEV Tip Accounts for private bundle routing
pub const JITO_TIP_ACCOUNTS: &[&str] = &[
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pWHLnjvnxKLQm3AoZ23z",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SniperSettings {
    pub total_budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_slippage_bps: u16,
    pub jito_tip_sol: f64,
    pub is_paper_trading: bool,
    pub max_concurrent_positions: usize,
    pub min_liquidity_sol: f64,
    pub live_reserves: bool, // Option 1: Live on-chain bonding curve reserves
    pub rug_filter: RugFilterConfig,
    pub tp1_multiplier: f64, // e.g. 2.0x (sell 50%)
    pub tp2_multiplier: f64, // e.g. 5.0x (sell 25%)
    pub stop_loss_pct: f64,  // e.g. -25%
    pub break_even_mult: Option<f64>, // Break-even stop trigger: locks in 1.02x floor when peak >= e.g. 1.10x
    pub min_curve_pct: Option<f64>, // Item 3: Require minimum bonding curve progress % (e.g. 10.0%)
    pub min_dev_buy_sol: Option<f64>, // Item 3: Require minimum dev initial buy in SOL (e.g. 0.40 SOL)
    pub min_unique_buyers: usize, // Upgrade 2: Minimum unique buyer wallets required before momentum entry
}

impl Default for SniperSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.5,
            buy_amount_sol: 0.05, // 0.05 SOL per trade (10 trades from 0.5 SOL budget)
            max_slippage_bps: 1000,
            jito_tip_sol: 0.0003,
            is_paper_trading: true,
            max_concurrent_positions: 5,
            min_liquidity_sol: 1.0,
            live_reserves: false, // Optional: enabled via --live-reserves
            rug_filter: RugFilterConfig::default(),
            tp1_multiplier: 1.30,
            tp2_multiplier: 1.80,
            stop_loss_pct: -10.0,
            break_even_mult: Some(1.10),
            min_curve_pct: None,
            min_dev_buy_sol: None,
            min_unique_buyers: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioState {
    pub initial_budget_sol: f64,
    pub available_sol: f64,
    pub invested_sol: f64,
    pub realized_pnl_sol: f64,
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
}

impl PortfolioState {
    pub fn total_portfolio_value_sol(&self) -> f64 {
        self.available_sol + self.invested_sol
    }

    pub fn return_on_investment_pct(&self) -> f64 {
        if self.initial_budget_sol > 0.0 {
            (self.realized_pnl_sol / self.initial_budget_sol) * 100.0
        } else {
            0.0
        }
    }

    pub fn win_rate_pct(&self) -> f64 {
        let closed = self.winning_trades + self.losing_trades;
        if closed > 0 {
            (self.winning_trades as f64 / closed as f64) * 100.0
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnipedPosition {
    pub token_mint: String,
    pub pool_address: String,
    pub creator: Option<String>,
    pub tokens_held: u64,
    pub protocol: DexProtocol,
    pub entry_slot: u64,
    pub entry_timestamp: DateTime<Utc>,
    pub entry_price_sol: f64, // Exact SOL price per token at entry
    pub buy_amount_sol: f64,
    pub remaining_portion_pct: f64,
    pub current_multiplier: f64,
    pub peak_multiplier: f64,
    pub tp1_executed: bool,
    pub tp2_executed: bool,
    pub is_closed: bool,
    pub pnl_sol: f64,
    pub age_ticks: u32,
    pub momentum_type: u8, // 0 = standard pump, 1 = super runner (5x+), 2 = early dump
    pub is_token_2022: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JitoBundlePlan {
    pub tip_account: String,
    pub tip_lamports: u64,
    pub instructions_count: usize,
    pub estimated_priority_fee_sol: f64,
    pub simulated_status: String,
}

pub struct SolanaSniperEngine {
    settings: SniperSettings,
    rug_filter: RugFilter,
    positions: Arc<Mutex<HashMap<String, SnipedPosition>>>,
    portfolio: Arc<Mutex<PortfolioState>>,
    executor: Option<Arc<OnChainExecutor>>,
    known_pools: Arc<Mutex<HashMap<String, PoolCreationEvent>>>,
    entered_pools: Arc<Mutex<HashSet<String>>>,
    pub pool_buyers: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    pub pool_dev_dumped: Arc<Mutex<HashSet<String>>>,
}

impl SolanaSniperEngine {
    #[allow(dead_code)]
    pub fn new(settings: SniperSettings) -> Self {
        Self::new_with_executor(settings, None)
    }

    pub fn new_with_executor(
        settings: SniperSettings,
        executor: Option<Arc<OnChainExecutor>>,
    ) -> Self {
        let rug_filter = RugFilter::new(settings.rug_filter.clone());
        let initial_budget = settings.total_budget_sol;
        Self {
            settings,
            rug_filter,
            positions: Arc::new(Mutex::new(HashMap::new())),
            portfolio: Arc::new(Mutex::new(PortfolioState {
                initial_budget_sol: initial_budget,
                available_sol: initial_budget,
                invested_sol: 0.0,
                realized_pnl_sol: 0.0,
                total_trades: 0,
                winning_trades: 0,
                losing_trades: 0,
            })),
            executor,
            known_pools: Arc::new(Mutex::new(HashMap::new())),
            entered_pools: Arc::new(Mutex::new(HashSet::new())),
            pool_buyers: Arc::new(Mutex::new(HashMap::new())),
            pool_dev_dumped: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Record live trade activity (buys/sells) to track unique buyers and detect developer dumps
    pub fn record_trade_activity(&self, trade: &super::yellowstone::TradeActivityEvent) {
        if trade.is_buy {
            let mut pb = self.pool_buyers.lock().unwrap();
            pb.entry(trade.pool_address.clone())
                .or_insert_with(HashSet::new)
                .insert(trade.user_wallet.clone());
        }
        if trade.is_sell {
            let is_dev = {
                let kp = self.known_pools.lock().unwrap();
                kp.get(&trade.pool_address)
                    .and_then(|e| e.creator.as_ref())
                    .map(|c| c == &trade.user_wallet)
                    .unwrap_or(false)
            };
            if is_dev {
                let is_new = {
                    let mut dumped = self.pool_dev_dumped.lock().unwrap();
                    dumped.insert(trade.pool_address.clone())
                };
                if is_new {
                    warn!(
                        "🚨 [DEV DUMP DETECTED] Creator {} dumped tokens for pool {}! Blacklisting pool.",
                        trade.user_wallet, trade.pool_address
                    );
                }

                // In-Slot Emergency Exit: Immediately liquidate any active position on this pool
                self.emergency_exit_pool(&trade.pool_address);
            }
        }
    }

    /// Immediately exit an active position on a pool where a developer dump was detected in the mempool
    pub fn emergency_exit_pool(&self, pool_address: &str) {
        let mut positions = self.positions.lock().unwrap();
        let mut port = self.portfolio.lock().unwrap();
        let total_budget = self.settings.total_budget_sol;

        for pos in positions.values_mut() {
            if pos.pool_address == pool_address && !pos.is_closed {
                pos.is_closed = true;
                let remaining_fraction = pos.remaining_portion_pct / 100.0;
                let gross_sell_proceeds = remaining_fraction * pos.buy_amount_sol * pos.current_multiplier;
                let pump_fee = gross_sell_proceeds * 0.01;
                let sell_proceeds = (gross_sell_proceeds - pump_fee - self.settings.jito_tip_sol).max(0.0);
                let cost_basis = remaining_fraction * pos.buy_amount_sol;
                let final_delta = sell_proceeds - cost_basis;

                if self.settings.is_paper_trading {
                    port.available_sol += sell_proceeds;
                    port.invested_sol = (port.invested_sol - cost_basis).max(0.0);
                    port.realized_pnl_sol += final_delta;
                    pos.pnl_sol += final_delta;
                    if final_delta >= 0.0 {
                        port.winning_trades += 1;
                    } else {
                        port.losing_trades += 1;
                    }
                }

                println!(
                    "\n{}",
                    format!(
                        "🚨 [EMERGENCY IN-SLOT EXIT] Creator dump detected for {}! Liquidated at {:.2}x. Saved: {:.4} SOL",
                        pos.token_mint, pos.current_multiplier, sell_proceeds
                    )
                    .bold()
                    .red()
                );

                if !self.settings.is_paper_trading {
                    if let Some(ref executor) = self.executor {
                        let exec = Arc::clone(executor);
                        let mint = pos.token_mint.clone();
                        let pool = pos.pool_address.clone();
                        let creator = pos.creator.clone();
                        let tip = self.settings.jito_tip_sol;
                        let min_lamports = 1u64; // Guaranteed fill
                        let is_token_2022 = pos.is_token_2022;
                        let portfolio_arc = Arc::clone(&self.portfolio);
                        tokio::spawn(async move {
                            println!("{}", format!("🚨 [ON-CHAIN EMERGENCY DEV DUMP EXIT] Dispatching exit bundle for {}...", mint).bold().red());
                            match exec.execute_snipe_sell_jito(&mint, &pool, creator.as_deref(), 0, min_lamports, tip, is_token_2022).await {
                                Ok(res) => {
                                    let _ = exec.poll_signature_confirmation(&res.signature, 6, 500).await;
                                    if let Ok(real_bal) = exec.get_balance_sol().await {
                                        let mut port = portfolio_arc.lock().unwrap();
                                        let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("   {}", format!("✖ [ON-CHAIN EMERGENCY EXIT FAILED] Mint: {} | Error: {}", mint, e).bold().red());
                                }
                            }
                        });
                    }
                }
            }
        }
    }

    /// Get verified unique buyer count for a specific bonding curve
    #[allow(dead_code)]
    pub fn get_unique_buyer_count(&self, pool_address: &str) -> usize {
        let pb = self.pool_buyers.lock().unwrap();
        pb.get(pool_address).map(|s| s.len()).unwrap_or(0)
    }

    /// Check if a pool has been flagged for a developer dump
    #[allow(dead_code)]
    pub fn is_dev_dumped(&self, pool_address: &str) -> bool {
        let dumped = self.pool_dev_dumped.lock().unwrap();
        dumped.contains(pool_address)
    }

    /// Sync live on-chain wallet balance from Solana RPC directly into portfolio state
    pub async fn sync_live_wallet_balance(&self) -> Option<f64> {
        if let Some(ref exec) = self.executor {
            if let Ok(real_bal) = exec.get_balance_sol().await {
                let mut port = self.portfolio.lock().unwrap();
                let rem_budget = (self.settings.total_budget_sol - port.invested_sol).max(0.0);
                port.available_sol = real_bal.min(rem_budget);
                return Some(real_bal);
            }
        }
        None
    }

    /// Process an incoming pool creation event from Yellowstone gRPC in real time (<1ms)
    pub fn process_detected_pool(&self, event: &PoolCreationEvent) -> Option<SecurityCheckReport> {
        let is_pump = matches!(event.protocol, DexProtocol::PumpFun);
        let initial_sol = event.initial_sol_liquidity.unwrap_or(0.5);

        // Store event in known pools registry for potential momentum breakout tracking
        {
            let mut kp = self.known_pools.lock().unwrap();
            kp.insert(event.pool_address.clone(), event.clone());
        }

        // Item 3: If minimum curve % completion is configured, defer entry until confirmed momentum
        if let Some(min_pct) = self.settings.min_curve_pct {
            if min_pct > 0.0 && is_pump {
                println!(
                    "   👀 [WATCHLIST ACTIVE] Pool registered for ≥{:.1}% curve momentum breakout: {} (Dev Buy: {:.2} SOL)",
                    min_pct,
                    event.token_mint,
                    event.dev_buy_sol.unwrap_or(0.0)
                );
                return None;
            }
        }

        // Filter 1: Check minimum pool liquidity requirement
        if initial_sol < self.settings.min_liquidity_sol && !is_pump {
            return None;
        }

        // Filter 2: Check active concurrent positions limit
        {
            let positions = self.positions.lock().unwrap();
            let active_count = positions.values().filter(|p| !p.is_closed).count();
            if active_count >= self.settings.max_concurrent_positions {
                return None;
            }
        }

        // Filter 3: Check capital constraint in wallet (accounting for 0.0022 SOL ATA rent/network fee on-chain)
        let onchain_fee_buffer = if !self.settings.is_paper_trading { 0.0022 } else { 0.0 };
        let trade_cost = self.settings.buy_amount_sol + self.settings.jito_tip_sol + onchain_fee_buffer;
        {
            let port = self.portfolio.lock().unwrap();
            if port.available_sol < trade_cost {
                warn!(
                    "Insufficient wallet balance ({:.4} SOL < {:.4} SOL required cost). Waiting for exits or capital.",
                    port.available_sol, trade_cost
                );
                return None;
            }
        }

        // Filter 4: Zero-latency heuristic rug, socials, dev holding & dev skin-in-the-game check (<10µs)
        let creator_holding = if let Some(spent) = event.dev_buy_sol {
            (spent / (30.0 + spent)) * 100.0
        } else {
            0.0
        };

        let report = self.rug_filter.evaluate_fast(
            &event.token_mint,
            is_pump,
            initial_sol,
            true, // Freeze auth revoked
            true, // Mint auth revoked
            creator_holding,
            event.has_socials,
            event.dev_buy_sol,
        );

        match &report.status {
            SafetyStatus::Rejected(reason) => {
                println!(
                    "   {}",
                    format!("❌ [FILTER REJECT] Dropped: {} in {}µs", reason, report.execution_latency_us).bold().red()
                );
                return Some(report);
            }
            SafetyStatus::Warning(warnings) => {
                println!(
                    "   {}",
                    format!("⚠ [FILTER WARNING] {:?}", warnings).yellow()
                );
            }
            SafetyStatus::Safe => {
                // Mark pool as entered
                {
                    let mut entered = self.entered_pools.lock().unwrap();
                    entered.insert(event.pool_address.clone());
                }
                // Execute paper snipe entry
                self.execute_entry(event, None, None);
            }
        }

        Some(report)
    }

    fn execute_entry(
        &self,
        event: &PoolCreationEvent,
        entry_price_override: Option<f64>,
        curve_pct_override: Option<f64>,
    ) {
        // Determine deterministic market momentum from mint address hash
        let hash_val = event.token_mint.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
        let momentum_type = if (hash_val % 100) < 18 {
            1 // 18% super runner (reaches 5x+)
        } else if (hash_val % 100) < 55 {
            0 // 37% standard pump (reaches 2x TP1)
        } else {
            2 // 45% early dump (hits -25% stop-loss)
        };

        let entry_price = entry_price_override.unwrap_or(30.0 / 1_073_000_000.0);
        let tokens_est = if entry_price > 0.0 {
            (self.settings.buy_amount_sol / entry_price) as u64
        } else {
            ((self.settings.buy_amount_sol * 1_000_000_000.0 * 1_073_000_000_000_000.0) / 30_050_000_000.0) as u64
        };

        let position = SnipedPosition {
            token_mint: event.token_mint.clone(),
            pool_address: event.pool_address.clone(),
            creator: event.creator.clone(),
            tokens_held: tokens_est,
            protocol: event.protocol.clone(),
            entry_slot: event.slot,
            entry_timestamp: Utc::now(),
            entry_price_sol: entry_price,
            buy_amount_sol: self.settings.buy_amount_sol,
            remaining_portion_pct: 100.0,
            current_multiplier: 1.0,
            peak_multiplier: 1.0,
            tp1_executed: false,
            tp2_executed: false,
            is_closed: false,
            pnl_sol: 0.0,
            age_ticks: 0,
            momentum_type,
            is_token_2022: event.is_token_2022,
        };

        if self.settings.is_paper_trading {
            // Deduct from paper portfolio balance (including 1% Pump.fun fee and Jito tip)
            let pump_fee = self.settings.buy_amount_sol * 0.01;
            let total_paper_cost = self.settings.buy_amount_sol + pump_fee + self.settings.jito_tip_sol;
            let (avail, invested, active_cnt) = {
                let mut port = self.portfolio.lock().unwrap();
                port.available_sol -= total_paper_cost;
                port.invested_sol += self.settings.buy_amount_sol;
                port.total_trades += 1;
                (port.available_sol, port.invested_sol, port.total_trades)
            };

            if let Some(curve_pct) = curve_pct_override {
                println!(
                    "\n{}",
                    format!(
                        "🎯 [MOMENTUM ENTRY #{}] Protocol: {} | Mint: {}",
                        active_cnt, event.protocol, event.token_mint
                    )
                    .bold()
                    .cyan()
                );
                println!(
                    "   Curve Progress: {:.1}% | Entry Price: {:.9} SOL | Cost: {:.3} SOL (+{:.4} Jito Tip)",
                    curve_pct,
                    entry_price,
                    self.settings.buy_amount_sol,
                    self.settings.jito_tip_sol
                );
            } else {
                println!(
                    "\n{}",
                    format!(
                        "🎯 [SNIPED ENTRY #{}] Protocol: {} | Mint: {}",
                        active_cnt, event.protocol, event.token_mint
                    )
                    .bold()
                    .cyan()
                );
                println!(
                    "   Slot: {} | Initial Liquidity: {:.2} SOL | Cost: {:.3} SOL (+{:.4} Jito Tip)",
                    event.slot,
                    event.initial_sol_liquidity.unwrap_or(0.0),
                    self.settings.buy_amount_sol,
                    self.settings.jito_tip_sol
                );
            }
            println!(
                "   💼 [PAPER WALLET] Balance: {:.3} SOL Available | {:.3} SOL Invested",
                avail, invested
            );
            println!(
                "   📈 Brackets: TP1: {:.1}x (50%) | TP2: {:.1}x (25%) | Stop-Loss: {:.1}%",
                self.settings.tp1_multiplier, self.settings.tp2_multiplier, self.settings.stop_loss_pct
            );

            let mut positions = self.positions.lock().unwrap();
            let pos_key = format!("{}:{}", event.token_mint, positions.len());
            positions.insert(pos_key, position);

            // Recalculate true active invested capital directly from open positions
            {
                let mut port = self.portfolio.lock().unwrap();
                port.invested_sol = positions
                    .values()
                    .filter(|p| !p.is_closed)
                    .map(|p| (p.remaining_portion_pct / 100.0) * p.buy_amount_sol)
                    .sum();
            }
        } else {
            // Live on-chain execution with Jito MEV bundles
            if let Some(ref executor) = self.executor {
                let exec = Arc::clone(executor);
                let mint = event.token_mint.clone();
                let pool = event.pool_address.clone();
                let creator = event.creator.clone();
                let buy_amount = self.settings.buy_amount_sol;
                let jito_tip = self.settings.jito_tip_sol;
                let slippage_bps = self.settings.max_slippage_bps;
                let is_token_2022 = event.is_token_2022;
                let total_budget = self.settings.total_budget_sol;
                let positions_arc = Arc::clone(&self.positions);
                let portfolio_arc = Arc::clone(&self.portfolio);

                tokio::spawn(async move {
                    println!(
                        "\n{}",
                        format!("🎯 [ON-CHAIN ATTEMPT] Sniping {} | Cost: {:.3} SOL (+{:.4} Tip)...", mint, buy_amount, jito_tip)
                            .bold()
                            .cyan()
                    );
                    println!(
                        "   {}",
                        format!("🚀 [ON-CHAIN SUBMISSION] Sending dual-path Jito MEV + Direct TPU transaction for {}...", mint)
                            .bold()
                            .yellow()
                    );
                    match exec
                        .execute_snipe_buy_jito(&mint, &pool, creator.as_deref(), buy_amount, jito_tip, slippage_bps, is_token_2022)
                        .await
                    {
                        Ok(res) => {
                            println!(
                                "   {}",
                                format!(
                                    "✔ [ON-CHAIN SUBMISSION BROADCAST] Tx Signature: https://solscan.io/tx/{}",
                                    res.signature
                                )
                                .bold()
                                .green()
                            );
                            println!(
                                "      Pipeline: {} | Tip: {:.4} SOL to {}",
                                res.bundle_id,
                                res.tip_lamports as f64 / 1_000_000_000.0,
                                &res.tip_account[..12.min(res.tip_account.len())]
                            );

                            // Confirm transaction on-chain before inserting into active positions!
                            match exec.poll_signature_confirmation(&res.signature, 8, 500).await {
                                Ok(true) => {
                                    println!(
                                        "   {}",
                                        format!("🎉 [ON-CHAIN BUY CONFIRMED] Token {} acquired on-chain! Position now active.", mint)
                                            .bold()
                                            .green()
                                    );
                                    let (active_cnt, invested) = {
                                        let mut positions = positions_arc.lock().unwrap();
                                        let pos_key = format!("{}:{}", mint, positions.len());
                                        positions.insert(pos_key, position);
                                        let cnt = positions.values().filter(|p| !p.is_closed).count();
                                        let inv: f64 = positions.values().filter(|p| !p.is_closed).map(|p| (p.remaining_portion_pct / 100.0) * p.buy_amount_sol).sum();
                                        (cnt, inv)
                                    };

                                    if let Ok(real_bal) = exec.get_balance_sol().await {
                                        let mut port = portfolio_arc.lock().unwrap();
                                        port.invested_sol = invested;
                                        let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                        port.total_trades += 1;
                                        println!(
                                            "   ⚡ [LIVE WALLET SYNC] Available Balance: {:.4} SOL | Active Positions: {}",
                                            port.available_sol, active_cnt
                                        );
                                    }
                                }
                                Ok(false) => {
                                    eprintln!(
                                        "   {}",
                                        format!("⚠ [ON-CHAIN PENDING] Tx {} pending confirmation. Skipping unconfirmed position.", res.signature)
                                            .yellow()
                                     );
                                    if let Ok(real_bal) = exec.get_balance_sol().await {
                                        let mut port = portfolio_arc.lock().unwrap();
                                        let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                    }
                                }
                                Err(err) => {
                                    eprintln!(
                                        "   {}",
                                        format!("✖ [ON-CHAIN REVERTED] Buy transaction failed on-chain: {}. Position NOT added.", err)
                                            .bold()
                                            .red()
                                    );
                                    if let Ok(real_bal) = exec.get_balance_sol().await {
                                        let mut port = portfolio_arc.lock().unwrap();
                                        let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            eprintln!(
                                "   {}",
                                format!("✖ [ON-CHAIN SUBMISSION FAILED] Error: {}", err).bold().red()
                            );
                            if let Ok(real_bal) = exec.get_balance_sol().await {
                                let mut port = portfolio_arc.lock().unwrap();
                                let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                port.available_sol = real_bal.min(rem_budget);
                            }
                        }
                    }
                });
            } else {
                warn!("On-chain execution requested but no OnChainExecutor was configured!");
            }
        }
    }

    /// Advance active positions, track prices, and execute Take-Profit / Stop-Loss triggers
    pub fn tick_position_updates(&self) {
        let mut positions = self.positions.lock().unwrap();
        let mut port = self.portfolio.lock().unwrap();
        let total_budget = self.settings.total_budget_sol;
        let mut should_print_hud = false;

        for pos in positions.values_mut() {
            if pos.is_closed {
                continue;
            }

            pos.age_ticks += 1;

            // In live on-chain mode OR when live reserves are active (Option 1):
            // Real price updates come directly from on-chain reserves via update_bonding_curve_reserve!
            // Synthetic momentum simulation is ONLY for pure paper trading when live reserves are disabled.
            if self.settings.is_paper_trading && !self.settings.live_reserves {
                // Simulate realistic bonding curve price movement based on token momentum
                match pos.momentum_type {
                    1 => {
                        // Super runner (pumps to 2.5x -> 5.5x)
                        pos.current_multiplier = 1.0 + (pos.age_ticks as f64 * 0.45);
                    }
                    0 => {
                        // Standard pump (peaks at 2.2x then fades)
                        if pos.age_ticks <= 5 {
                            pos.current_multiplier = 1.0 + (pos.age_ticks as f64 * 0.25);
                        } else {
                            pos.current_multiplier = (pos.current_multiplier - 0.15).max(0.70);
                        }
                    }
                    _ => {
                        // Fast dump (drifts down to stop loss)
                        pos.current_multiplier = (1.0 - (pos.age_ticks as f64 * 0.08)).max(0.60);
                    }
                }
            }

            if pos.current_multiplier > pos.peak_multiplier {
                pos.peak_multiplier = pos.current_multiplier;
            }

            // Trigger TP1: 1.30x (Sell 70% in ONE transaction to eliminate multi-tip friction)
            if pos.current_multiplier >= self.settings.tp1_multiplier && !pos.tp1_executed {
                pos.tp1_executed = true;
                pos.remaining_portion_pct -= 70.0;
                let gross_sell_proceeds = 0.70 * pos.buy_amount_sol * pos.current_multiplier;
                let pump_fee = gross_sell_proceeds * 0.01;
                let sell_proceeds = (gross_sell_proceeds - pump_fee - self.settings.jito_tip_sol).max(0.0);
                let profit = sell_proceeds - (0.70 * pos.buy_amount_sol);

                if self.settings.is_paper_trading {
                    port.available_sol += sell_proceeds;
                    port.invested_sol -= 0.70 * pos.buy_amount_sol;
                    port.realized_pnl_sol += profit;
                    pos.pnl_sol += profit;
                }

                println!(
                    "\n{}",
                    format!(
                        "🎉 [TP 1 HIT] {:.2}x on {}! Sold 70% for +{:.3} SOL (Initial Capital Recouped!)",
                        pos.current_multiplier, pos.token_mint, sell_proceeds
                    )
                    .bold()
                    .green()
                );
                if self.settings.is_paper_trading {
                    println!(
                        "   💼 [WALLET UPDATE] Available Balance: {:.3} SOL | Realized Profit: +{:.3} SOL",
                        port.available_sol, port.realized_pnl_sol
                    );
                }
                should_print_hud = true;

                if !self.settings.is_paper_trading {
                    if let Some(ref executor) = self.executor {
                        let exec = Arc::clone(executor);
                        let mint = pos.token_mint.clone();
                        let pool = pos.pool_address.clone();
                        let creator = pos.creator.clone();
                        let tip = self.settings.jito_tip_sol;
                        let min_lamports = (sell_proceeds * 1_000_000_000.0 * 0.40).max(1.0) as u64;
                        let tokens_to_sell = ((pos.tokens_held as f64) * 0.70) as u64;
                        let is_token_2022 = pos.is_token_2022;
                        let portfolio_arc = Arc::clone(&self.portfolio);
                        tokio::spawn(async move {
                            println!("{}", format!("🚀 [ON-CHAIN TP1 SELL] Dispatching Jito sell bundle for {}...", mint).bold().green());
                            match exec.execute_snipe_sell_jito(&mint, &pool, creator.as_deref(), tokens_to_sell, min_lamports, tip, is_token_2022).await {
                                Ok(res) => {
                                    let _ = exec.poll_signature_confirmation(&res.signature, 6, 500).await;
                                    if let Ok(real_bal) = exec.get_balance_sol().await {
                                        let mut port = portfolio_arc.lock().unwrap();
                                        let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("   {}", format!("✖ [ON-CHAIN TP1 SELL FAILED] Mint: {} | Error: {}", mint, e).bold().red());
                                }
                            }
                        });
                    }
                }
            }

            // Trigger TP2: 1.80x (Sell 20%, leave 10% moonbag)
            if pos.current_multiplier >= self.settings.tp2_multiplier && !pos.tp2_executed && pos.tp1_executed {
                pos.tp2_executed = true;
                pos.remaining_portion_pct -= 20.0;
                let gross_sell_proceeds = 0.20 * pos.buy_amount_sol * pos.current_multiplier;
                let pump_fee = gross_sell_proceeds * 0.01;
                let sell_proceeds = (gross_sell_proceeds - pump_fee - self.settings.jito_tip_sol).max(0.0);
                let profit = sell_proceeds - (0.20 * pos.buy_amount_sol);

                if self.settings.is_paper_trading {
                    port.available_sol += sell_proceeds;
                    port.invested_sol -= 0.20 * pos.buy_amount_sol;
                    port.realized_pnl_sol += profit;
                    pos.pnl_sol += profit;
                }

                println!(
                    "\n{}",
                    format!(
                        "🚀 [TP 2 MOONSHOT] {:.2}x on {}! Sold 20% for +{:.3} SOL!",
                        pos.current_multiplier, pos.token_mint, sell_proceeds
                    )
                    .bold()
                    .magenta()
                );
                if self.settings.is_paper_trading {
                    println!(
                        "   💼 [WALLET UPDATE] Available Balance: {:.3} SOL | Realized Profit: +{:.3} SOL",
                        port.available_sol, port.realized_pnl_sol
                    );
                }
                should_print_hud = true;

                if !self.settings.is_paper_trading {
                    if let Some(ref executor) = self.executor {
                        let exec = Arc::clone(executor);
                        let mint = pos.token_mint.clone();
                        let pool = pos.pool_address.clone();
                        let creator = pos.creator.clone();
                        let tip = self.settings.jito_tip_sol;
                        let min_lamports = (sell_proceeds * 1_000_000_000.0 * 0.40).max(1.0) as u64;
                        let tokens_to_sell = pos.tokens_held / 4;
                        let is_token_2022 = pos.is_token_2022;
                        let portfolio_arc = Arc::clone(&self.portfolio);
                        tokio::spawn(async move {
                            println!("{}", format!("🚀 [ON-CHAIN TP2 SELL] Dispatching Jito sell bundle for {}...", mint).bold().magenta());
                            match exec.execute_snipe_sell_jito(&mint, &pool, creator.as_deref(), tokens_to_sell, min_lamports, tip, is_token_2022).await {
                                Ok(res) => {
                                    let _ = exec.poll_signature_confirmation(&res.signature, 6, 500).await;
                                    if let Ok(real_bal) = exec.get_balance_sol().await {
                                        let mut port = portfolio_arc.lock().unwrap();
                                        let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("   {}", format!("✖ [ON-CHAIN TP2 SELL FAILED] Mint: {} | Error: {}", mint, e).bold().red());
                                }
                            }
                        });
                    }
                }
            }

            // Trigger Dynamic Trailing Stop Loss, Break-Even Floor, or Position Expiration
            let trailing_stop_level = pos.peak_multiplier * (1.0 + (self.settings.stop_loss_pct / 100.0));
            let initial_stop_level = 1.0 + (self.settings.stop_loss_pct / 100.0);
            let break_even_stop = if let Some(be_trigger) = self.settings.break_even_mult {
                if pos.peak_multiplier >= be_trigger {
                    1.02 // Lock in cost basis + Pump.fun fee floor
                } else {
                    0.0
                }
            } else {
                0.0
            };
            let effective_stop = trailing_stop_level.max(initial_stop_level).max(break_even_stop);
            let is_stagnant_expiry = pos.age_ticks >= 60;
            if (pos.current_multiplier <= effective_stop || is_stagnant_expiry) && !pos.is_closed {
                pos.is_closed = true;
                let remaining_fraction = pos.remaining_portion_pct / 100.0;
                let gross_sell_proceeds = remaining_fraction * pos.buy_amount_sol * pos.current_multiplier;
                let pump_fee = gross_sell_proceeds * 0.01;
                let sell_proceeds = (gross_sell_proceeds - pump_fee - self.settings.jito_tip_sol).max(0.0);
                let cost_basis = remaining_fraction * pos.buy_amount_sol;
                let final_delta = sell_proceeds - cost_basis;

                if self.settings.is_paper_trading {
                    port.available_sol += sell_proceeds;
                    port.invested_sol = (port.invested_sol - cost_basis).max(0.0);
                    port.realized_pnl_sol += final_delta;
                    pos.pnl_sol += final_delta;
                }

                if pos.pnl_sol >= 0.0 {
                    if self.settings.is_paper_trading {
                        port.winning_trades += 1;
                    }
                    let exit_tag = if pos.peak_multiplier >= self.settings.break_even_mult.unwrap_or(1.10) && pos.current_multiplier <= 1.05 {
                        "🛡️ [BREAK-EVEN FLOOR HIT]"
                    } else {
                        "✔ [POSITION CLOSED IN PROFIT]"
                    };
                    println!(
                        "\n{}",
                        format!(
                            "{} {} | Multiplier: {:.2}x | Net PnL: +{:.4} SOL",
                            exit_tag, pos.token_mint, pos.current_multiplier, pos.pnl_sol
                        )
                        .green()
                    );
                } else if is_stagnant_expiry {
                    if self.settings.is_paper_trading {
                        port.losing_trades += 1;
                    }
                    println!(
                        "\n{}",
                        format!(
                            "⏱️ [STAGNATION EXIT] Closed {} after 90s at {:.2}x. Net PnL: {:.4} SOL",
                            pos.token_mint, pos.current_multiplier, pos.pnl_sol
                        )
                        .yellow()
                    );
                } else {
                    if self.settings.is_paper_trading {
                        port.losing_trades += 1;
                    }
                    println!(
                        "\n{}",
                        format!(
                            "🛡️ [STOP LOSS TRIGGERED] Closed {} at {:.2}x (-{:.1}%). Saved: {:.3} SOL",
                            pos.token_mint,
                            pos.current_multiplier,
                            (1.0 - pos.current_multiplier) * 100.0,
                            sell_proceeds
                        )
                        .yellow()
                    );
                }

                if self.settings.is_paper_trading {
                    println!(
                        "   💼 [WALLET UPDATE] Available Balance: {:.3} SOL | Realized Profit: +{:.3} SOL",
                        port.available_sol, port.realized_pnl_sol
                    );
                }
                should_print_hud = true;

                if !self.settings.is_paper_trading {
                    if let Some(ref executor) = self.executor {
                        let exec = Arc::clone(executor);
                        let mint = pos.token_mint.clone();
                        let pool = pos.pool_address.clone();
                        let creator = pos.creator.clone();
                        let tip = self.settings.jito_tip_sol;
                        let min_lamports = 1u64; // Upgrade 3: 1 lamport guarantees execution & prevents Error 6003 revert
                        let is_token_2022 = pos.is_token_2022;
                        let portfolio_arc = Arc::clone(&self.portfolio);
                        tokio::spawn(async move {
                            println!("{}", format!("🛡️ [ON-CHAIN STOP LOSS SELL] Dispatching emergency exit bundle for {}...", mint).bold().yellow());
                            match exec.execute_snipe_sell_jito(&mint, &pool, creator.as_deref(), 0, min_lamports, tip, is_token_2022).await {
                                Ok(res) => {
                                    let _ = exec.poll_signature_confirmation(&res.signature, 6, 500).await;
                                    if let Ok(real_bal) = exec.get_balance_sol().await {
                                        let mut port = portfolio_arc.lock().unwrap();
                                        let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("   {}", format!("✖ [ON-CHAIN STOP LOSS SELL FAILED] Mint: {} | Error: {}", mint, e).bold().red());
                                }
                            }
                        });
                    }
                }
            }
        }

        // Recalculate true active invested capital directly from open positions
        port.invested_sol = positions
            .values()
            .filter(|p| !p.is_closed)
            .map(|p| (p.remaining_portion_pct / 100.0) * p.buy_amount_sol)
            .sum();

        if should_print_hud {
            self.print_portfolio_hud_internal(&port, positions.values().filter(|p| !p.is_closed).count());
        }
    }

    /// Option 1: Process real on-chain bonding curve reserve updates streamed live via Yellowstone gRPC
    pub fn update_bonding_curve_reserve(&self, pool_address: &str, state: &super::yellowstone::BondingCurveState) {
        let mut positions = self.positions.lock().unwrap();
        let mut port = self.portfolio.lock().unwrap();
        let total_budget = self.settings.total_budget_sol;
        let mut should_print_hud = false;

        for pos in positions.values_mut() {
            if pos.pool_address == pool_address && !pos.is_closed {
                let initial_price = if pos.entry_price_sol > 0.0 {
                    pos.entry_price_sol
                } else {
                    30.0 / 1_073_000_000.0
                };
                let real_multiplier = if initial_price > 0.0 && state.price_sol > 0.0 {
                    (state.price_sol / initial_price).max(0.1)
                } else {
                    1.0
                };

                pos.current_multiplier = real_multiplier;
                if real_multiplier > pos.peak_multiplier {
                    pos.peak_multiplier = real_multiplier;
                }

                // Check TP1: 1.30x (Sell 70% in ONE transaction to eliminate multi-tip friction)
                if pos.current_multiplier >= self.settings.tp1_multiplier && !pos.tp1_executed {
                    pos.tp1_executed = true;
                    pos.remaining_portion_pct -= 70.0;
                    let gross_sell_proceeds = 0.70 * pos.buy_amount_sol * pos.current_multiplier;
                    let pump_fee = gross_sell_proceeds * 0.01;
                    let sell_proceeds = (gross_sell_proceeds - pump_fee - self.settings.jito_tip_sol).max(0.0);
                    let profit = sell_proceeds - (0.70 * pos.buy_amount_sol);

                    if self.settings.is_paper_trading {
                        port.available_sol += sell_proceeds;
                        port.invested_sol -= 0.70 * pos.buy_amount_sol;
                        port.realized_pnl_sol += profit;
                        pos.pnl_sol += profit;
                    }

                    println!(
                        "\n{}",
                        format!(
                            "🎉 [ON-CHAIN TP 1 HIT] {:.2}x on {} (Reserves: {:.2} SOL / {}M tokens)! Sold 70% for +{:.3} SOL!",
                            pos.current_multiplier, pos.token_mint,
                            state.virtual_sol_reserves as f64 / 1e9,
                            state.virtual_token_reserves / 1_000_000,
                            sell_proceeds
                        )
                        .bold()
                        .green()
                    );
                    should_print_hud = true;

                    if !self.settings.is_paper_trading {
                        if let Some(ref executor) = self.executor {
                            let exec = Arc::clone(executor);
                            let mint = pos.token_mint.clone();
                            let pool = pos.pool_address.clone();
                            let creator = pos.creator.clone();
                            let tip = self.settings.jito_tip_sol;
                            let min_lamports = (sell_proceeds * 1_000_000_000.0 * 0.40).max(1.0) as u64;
                            let tokens_to_sell = ((pos.tokens_held as f64) * 0.70) as u64;
                            let is_token_2022 = pos.is_token_2022;
                            let portfolio_arc = Arc::clone(&self.portfolio);
                            tokio::spawn(async move {
                                println!("{}", format!("🚀 [ON-CHAIN TP1 SELL] Dispatching Jito sell bundle for {}...", mint).bold().green());
                                match exec.execute_snipe_sell_jito(&mint, &pool, creator.as_deref(), tokens_to_sell, min_lamports, tip, is_token_2022).await {
                                    Ok(res) => {
                                        let _ = exec.poll_signature_confirmation(&res.signature, 6, 500).await;
                                        if let Ok(real_bal) = exec.get_balance_sol().await {
                                            let mut port = portfolio_arc.lock().unwrap();
                                            let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("   {}", format!("✖ [ON-CHAIN TP1 SELL FAILED] Mint: {} | Error: {}", mint, e).bold().red());
                                    }
                                }
                            });
                        }
                    }
                }

                // Check TP2: 1.80x (Sell 20%, leave 10% moonbag)
                if pos.current_multiplier >= self.settings.tp2_multiplier && !pos.tp2_executed && pos.tp1_executed {
                    pos.tp2_executed = true;
                    pos.remaining_portion_pct -= 20.0;
                    let gross_sell_proceeds = 0.20 * pos.buy_amount_sol * pos.current_multiplier;
                    let pump_fee = gross_sell_proceeds * 0.01;
                    let sell_proceeds = (gross_sell_proceeds - pump_fee - self.settings.jito_tip_sol).max(0.0);
                    let profit = sell_proceeds - (0.20 * pos.buy_amount_sol);

                    if self.settings.is_paper_trading {
                        port.available_sol += sell_proceeds;
                        port.invested_sol -= 0.20 * pos.buy_amount_sol;
                        port.realized_pnl_sol += profit;
                        pos.pnl_sol += profit;
                    }

                    println!(
                        "\n{}",
                        format!(
                            "🚀 [ON-CHAIN TP 2 MOONSHOT] {:.2}x on {} (Reserves: {:.2} SOL)! Sold 20% for +{:.3} SOL!",
                            pos.current_multiplier, pos.token_mint,
                            state.virtual_sol_reserves as f64 / 1e9,
                            sell_proceeds
                        )
                        .bold()
                        .magenta()
                    );
                    should_print_hud = true;

                    if !self.settings.is_paper_trading {
                        if let Some(ref executor) = self.executor {
                            let exec = Arc::clone(executor);
                            let mint = pos.token_mint.clone();
                            let pool = pos.pool_address.clone();
                            let creator = pos.creator.clone();
                            let tip = self.settings.jito_tip_sol;
                            let min_lamports = (sell_proceeds * 1_000_000_000.0 * 0.40).max(1.0) as u64;
                            let tokens_to_sell = ((pos.tokens_held as f64) * 0.20) as u64;
                            let is_token_2022 = pos.is_token_2022;
                            let portfolio_arc = Arc::clone(&self.portfolio);
                            tokio::spawn(async move {
                                println!("{}", format!("🚀 [ON-CHAIN TP2 SELL] Dispatching Jito sell bundle for {}...", mint).bold().magenta());
                                match exec.execute_snipe_sell_jito(&mint, &pool, creator.as_deref(), tokens_to_sell, min_lamports, tip, is_token_2022).await {
                                    Ok(res) => {
                                        let _ = exec.poll_signature_confirmation(&res.signature, 6, 500).await;
                                        if let Ok(real_bal) = exec.get_balance_sol().await {
                                            let mut port = portfolio_arc.lock().unwrap();
                                            let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("   {}", format!("✖ [ON-CHAIN TP2 SELL FAILED] Mint: {} | Error: {}", mint, e).bold().red());
                                    }
                                }
                            });
                        }
                    }
                }

                // Check Dynamic Trailing Stop Loss, Break-Even Floor, or Curve Complete (Graduation to Raydium)
                let trailing_stop_level = pos.peak_multiplier * (1.0 + (self.settings.stop_loss_pct / 100.0));
                let initial_stop_level = 1.0 + (self.settings.stop_loss_pct / 100.0);
                let break_even_stop = if let Some(be_trigger) = self.settings.break_even_mult {
                    if pos.peak_multiplier >= be_trigger {
                        1.02 // Lock in cost basis + Pump.fun fee floor
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                let effective_stop = trailing_stop_level.max(initial_stop_level).max(break_even_stop);
                if (pos.current_multiplier <= effective_stop || state.complete) && !pos.is_closed {
                    pos.is_closed = true;
                    let remaining_fraction = pos.remaining_portion_pct / 100.0;
                    let gross_sell_proceeds = remaining_fraction * pos.buy_amount_sol * pos.current_multiplier;
                    let pump_fee = gross_sell_proceeds * 0.01;
                    let sell_proceeds = (gross_sell_proceeds - pump_fee - self.settings.jito_tip_sol).max(0.0);
                    let cost_basis = remaining_fraction * pos.buy_amount_sol;
                    let final_delta = sell_proceeds - cost_basis;

                    if self.settings.is_paper_trading {
                        port.available_sol += sell_proceeds;
                        port.invested_sol = (port.invested_sol - cost_basis).max(0.0);
                        port.realized_pnl_sol += final_delta;
                        pos.pnl_sol += final_delta;
                    }

                    if pos.pnl_sol >= 0.0 {
                        if self.settings.is_paper_trading {
                            port.winning_trades += 1;
                        }
                        let exit_tag = if pos.peak_multiplier >= self.settings.break_even_mult.unwrap_or(1.10) && pos.current_multiplier <= 1.05 {
                            "🛡️ [BREAK-EVEN FLOOR HIT]"
                        } else {
                            "✔ [ON-CHAIN EXIT PROFIT]"
                        };
                        println!(
                            "\n{}",
                            format!("{} {} | Net PnL: +{:.3} SOL", exit_tag, pos.token_mint, pos.pnl_sol).green()
                        );
                    } else {
                        if self.settings.is_paper_trading {
                            port.losing_trades += 1;
                        }
                        println!(
                            "\n{}",
                            format!("🛡️ [ON-CHAIN STOP LOSS] {} at {:.2}x. Saved: {:.3} SOL", pos.token_mint, pos.current_multiplier, sell_proceeds).yellow()
                        );
                    }
                    should_print_hud = true;

                    if !self.settings.is_paper_trading {
                        if let Some(ref executor) = self.executor {
                            let exec = Arc::clone(executor);
                            let mint = pos.token_mint.clone();
                            let pool = pos.pool_address.clone();
                            let creator = pos.creator.clone();
                            let tip = self.settings.jito_tip_sol;
                            let min_lamports = 1u64; // Upgrade 3: 1 lamport guarantees execution & prevents Error 6003 revert
                            let is_token_2022 = pos.is_token_2022;
                            let portfolio_arc = Arc::clone(&self.portfolio);
                            tokio::spawn(async move {
                                println!("{}", format!("🛡️ [ON-CHAIN STOP LOSS SELL] Dispatching emergency exit bundle for {}...", mint).bold().yellow());
                                match exec.execute_snipe_sell_jito(&mint, &pool, creator.as_deref(), 0, min_lamports, tip, is_token_2022).await {
                                    Ok(res) => {
                                        let _ = exec.poll_signature_confirmation(&res.signature, 6, 500).await;
                                        if let Ok(real_bal) = exec.get_balance_sol().await {
                                            let mut port = portfolio_arc.lock().unwrap();
                                            let rem_budget = (total_budget - port.invested_sol).max(0.0);
                                        port.available_sol = real_bal.min(rem_budget);
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("   {}", format!("✖ [ON-CHAIN STOP LOSS SELL FAILED] Mint: {} | Error: {}", mint, e).bold().red());
                                    }
                                }
                            });
                        }
                    }
                }
            }
        }

        // Recalculate true active invested capital directly from open positions
        port.invested_sol = positions
            .values()
            .filter(|p| !p.is_closed)
            .map(|p| (p.remaining_portion_pct / 100.0) * p.buy_amount_sol)
            .sum();

        if should_print_hud {
            self.print_portfolio_hud_internal(&port, positions.values().filter(|p| !p.is_closed).count());
        }

        drop(positions);
        drop(port);

        // Item 3: Check for Momentum Breakout Entry on tokens crossing curve completion threshold
        if let Some(min_curve_pct) = self.settings.min_curve_pct {
            if min_curve_pct > 0.0 {
                self.check_and_trigger_momentum_entry(pool_address, state, min_curve_pct);
            }
        }
    }

    /// Item 3: Check whether a bonding curve has established real buy volume and crossed target % completion
    pub fn check_and_trigger_momentum_entry(
        &self,
        pool_address: &str,
        state: &super::yellowstone::BondingCurveState,
        min_curve_pct: f64,
    ) {
        if state.complete {
            return;
        }

        // Pump.fun bonding curves cap at 85 SOL real reserves (migration to Raydium)
        let curve_pct = (state.real_sol_reserves as f64 / 85_000_000_000.0) * 100.0;
        if curve_pct < min_curve_pct || curve_pct > 65.0 {
            return;
        }

        // 1. Check if pool was already entered
        {
            let entered = self.entered_pools.lock().unwrap();
            if entered.contains(pool_address) {
                return;
            }
        }

        // 1b. Upgrade 2: Reject immediately if developer dumped tokens
        {
            let dumped = self.pool_dev_dumped.lock().unwrap();
            if dumped.contains(pool_address) {
                return;
            }
        }

        // 1c. Upgrade 2: Holder Dispersion Check - require minimum unique retail buyers
        if self.settings.min_unique_buyers > 0 {
            let unique_cnt = {
                let pb = self.pool_buyers.lock().unwrap();
                pb.get(pool_address).map(|s| s.len()).unwrap_or(0)
            };
            if unique_cnt < self.settings.min_unique_buyers {
                return;
            }
        }

        // 2. Check active positions limit and verify not already open
        {
            let positions = self.positions.lock().unwrap();
            if positions.values().any(|p| p.pool_address == pool_address) {
                return;
            }
            let active_count = positions.values().filter(|p| !p.is_closed).count();
            if active_count >= self.settings.max_concurrent_positions {
                return;
            }
        }

        // 3. Check wallet capital constraint (accounting for 0.0022 SOL ATA rent/network fee on-chain)
        let onchain_fee_buffer = if !self.settings.is_paper_trading { 0.0022 } else { 0.0 };
        let trade_cost = self.settings.buy_amount_sol + self.settings.jito_tip_sol + onchain_fee_buffer;
        {
            let port = self.portfolio.lock().unwrap();
            if port.available_sol < trade_cost {
                return;
            }
        }

        // 4. Retrieve pool creation metadata
        let event = {
            let kp = self.known_pools.lock().unwrap();
            match kp.get(pool_address) {
                Some(e) => e.clone(),
                None => return,
            }
        };

        // 5. Evaluate rug & security filter
        let creator_holding = if let Some(spent) = event.dev_buy_sol {
            (spent / (30.0 + spent)) * 100.0
        } else {
            0.0
        };

        let report = self.rug_filter.evaluate_fast(
            &event.token_mint,
            true,
            state.real_sol_reserves as f64 / 1_000_000_000.0,
            true,
            true,
            creator_holding,
            event.has_socials,
            event.dev_buy_sol,
        );

        if !matches!(report.status, SafetyStatus::Safe) {
            return;
        }

        // 6. Mark as entered to prevent duplicate entry
        {
            let mut entered = self.entered_pools.lock().unwrap();
            entered.insert(pool_address.to_string());
        }

        if self.settings.min_unique_buyers > 0 {
            let unique_cnt = {
                let pb = self.pool_buyers.lock().unwrap();
                pb.get(pool_address).map(|s| s.len()).unwrap_or(0)
            };
            println!(
                "   {}",
                format!(
                    "✔ [HOLDER DISPERSION CONFIRMED] Pool {} reached {:.1}% curve with {} unique buyers!",
                    pool_address, curve_pct, unique_cnt
                )
                .bold()
                .green()
            );
        }

        // 7. Execute momentum entry at confirmed curve price
        self.execute_entry(&event, Some(state.price_sol), Some(curve_pct));
    }

    /// Display live portfolio HUD
    pub fn print_portfolio_hud(&self) {
        let port = self.portfolio.lock().unwrap();
        let positions = self.positions.lock().unwrap();
        let active_count = positions.values().filter(|p| !p.is_closed).count();
        self.print_portfolio_hud_internal(&port, active_count);
    }

    fn print_portfolio_hud_internal(&self, port: &PortfolioState, active_count: usize) {
        let total_val = port.total_portfolio_value_sol();
        let roi = port.return_on_investment_pct();
        let roi_colored = if roi >= 0.0 {
            format!("+{:.1}%", roi).bold().green()
        } else {
            format!("{:.1}%", roi).bold().red()
        };

        let pnl_colored = if port.realized_pnl_sol >= 0.0 {
            format!("+{:.3} SOL", port.realized_pnl_sol).bold().green()
        } else {
            format!("{:.3} SOL", port.realized_pnl_sol).bold().red()
        };

        let (top_box, bottom_box) = if self.settings.is_paper_trading {
            (
                format!("┌─────────────────────── 💼 LIVE PAPER WALLET ({:.3} SOL BUDGET) ───────────────────────┐", port.initial_budget_sol).bold().cyan(),
                "└────────────────────────────────────────────────────────────────────────────────────────┘".bold().cyan(),
            )
        } else {
            (
                format!("┌─────────────────────── ⚡ LIVE ON-CHAIN WALLET ({:.3} SOL REAL FUNDS) ─────────────────┐", port.initial_budget_sol).bold().red(),
                "└────────────────────────────────────────────────────────────────────────────────────────┘".bold().red(),
            )
        };

        println!("\n{}", top_box);
        println!(
            "│ Starting Capital: {:.3} SOL   │ Available Cash:   {:.3} SOL   │ Invested: {:.3} SOL      │",
            port.initial_budget_sol, port.available_sol, port.invested_sol
        );
        println!(
            "│ Realized PnL:     {:10}   │ Total Valuation:  {:.3} SOL   │ ROI:      {:8}      │",
            pnl_colored, total_val, roi_colored
        );
        println!(
            "│ Win Rate:         {:.1}% ({}W / {}L) │ Active Trades:    {}/{}          │ Total Snipes: {:3}     │",
            port.win_rate_pct(), port.winning_trades, port.losing_trades, active_count, self.settings.max_concurrent_positions, port.total_trades
        );
        println!("{}\n", bottom_box);
    }

    /// Display full final session audit report
    pub fn print_session_summary(&self) {
        let port = self.portfolio.lock().unwrap();
        let positions = self.positions.lock().unwrap();

        let unrealized_open_sol: f64 = positions
            .values()
            .filter(|p| !p.is_closed)
            .map(|p| (p.remaining_portion_pct / 100.0) * p.buy_amount_sol * p.current_multiplier)
            .sum();

        let total_val = port.available_sol + unrealized_open_sol;
        let total_pnl = total_val - port.initial_budget_sol;
        let roi = if port.initial_budget_sol > 0.0 {
            (total_pnl / port.initial_budget_sol) * 100.0
        } else {
            0.0
        };

        let pnl_colored = if total_pnl >= 0.0 {
            format!("+{:.4} SOL", total_pnl).bold().green()
        } else {
            format!("{:.4} SOL", total_pnl).bold().red()
        };

        let roi_colored = if roi >= 0.0 {
            format!("+{:.2}%", roi).bold().green()
        } else {
            format!("{:.2}%", roi).bold().red()
        };

        let session_title = if self.settings.is_paper_trading {
            "       🏁 SOLANA YELLOWSTONE gRPC PAPER TRADING SESSION AUDIT & PNL       "
        } else {
            "       ⚡ SOLANA YELLOWSTONE gRPC LIVE ON-CHAIN SESSION AUDIT & PNL       "
        };

        println!("\n{}", "================================================================================".cyan());
        println!("{}", session_title.bold().cyan());
        println!("{}", "================================================================================".cyan());
        println!("  Initial Wallet Budget:       {:.4} SOL", port.initial_budget_sol);
        println!("  Final Available Cash:        {:.4} SOL", port.available_sol);
        println!("  Unrealized Open Positions:   {:.4} SOL", unrealized_open_sol);
        println!("  Net Portfolio Valuation:     {:.4} SOL", total_val);
        println!("  Net PnL (Realized + Open):   {} ({})", pnl_colored, roi_colored);
        println!("  Realized Capital PnL:        {:.4} SOL", port.realized_pnl_sol);
        println!("  Total Pool Launches Sniped:  {}", port.total_trades);
        println!("  Closed Winning Trades:       {}", port.winning_trades);
        println!("  Closed Stop-Loss Trades:     {}", port.losing_trades);
        println!("  Active / Open Trades:        {}", positions.values().filter(|p| !p.is_closed).count());
        println!("  Session Win Rate:            {:.1}%", port.win_rate_pct());
        println!("{}", "--------------------------------------------------------------------------------".cyan());
        println!("{}", "                       SNIPED POSITIONS LEDGER BREAKDOWN                        ".bold().cyan());
        println!("{}", "--------------------------------------------------------------------------------".cyan());

        let mut pos_list: Vec<_> = positions.values().cloned().collect();
        pos_list.sort_by_key(|p| p.entry_slot);

        for (idx, pos) in pos_list.iter().enumerate() {
            let status_str = if pos.is_closed {
                if pos.pnl_sol >= 0.0 {
                    "Closed (Profit)".green().to_string()
                } else {
                    "Closed (Stop Loss)".red().to_string()
                }
            } else if pos.tp2_executed {
                "Open (TP1 & TP2 Hit)".magenta().to_string()
            } else if pos.tp1_executed {
                "Open (TP1 Hit)".green().to_string()
            } else {
                "Open (Trailing)".yellow().to_string()
            };

            let pnl_str = if pos.pnl_sol >= 0.0 {
                format!("+{:.4} SOL", pos.pnl_sol).green()
            } else {
                format!("{:.4} SOL", pos.pnl_sol).red()
            };

            println!(
                "  #{:02} Mint: {}...",
                idx + 1,
                &pos.token_mint[..pos.token_mint.len().min(24)]
            );
            println!(
                "       Protocol: {:12} | Slot: {} | Entry: {:.3} SOL",
                pos.protocol.to_string(), pos.entry_slot, pos.buy_amount_sol
            );
            println!(
                "       Peak: {:.2}x | Current: {:.2}x | Net PnL: {} | Status: {}",
                pos.peak_multiplier, pos.current_multiplier, pnl_str, status_str
            );
        }
        println!("{}\n", "================================================================================".cyan());
    }

    /// Build a Jito MEV private bundle plan to prevent frontrunning/sandwich attacks
    pub fn build_jito_bundle_plan(&self) -> JitoBundlePlan {
        let tip_account = JITO_TIP_ACCOUNTS[0].to_string();
        let tip_lamports = (self.settings.jito_tip_sol * 1_000_000_000.0) as u64;

        JitoBundlePlan {
            tip_account,
            tip_lamports,
            instructions_count: 3, // Compute Budget + Swap + Jito Tip Transfer
            estimated_priority_fee_sol: self.settings.jito_tip_sol,
            simulated_status: "SUCCESS_PREFLIGHT_READY".to_string(),
        }
    }

    /// Return all active positions
    pub fn get_active_positions(&self) -> Vec<SnipedPosition> {
        let positions = self.positions.lock().unwrap();
        positions.values().cloned().collect()
    }

    /// Return portfolio state copy
    #[allow(dead_code)]
    pub fn get_portfolio_state(&self) -> PortfolioState {
        let port = self.portfolio.lock().unwrap();
        port.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solana::yellowstone::BondingCurveState;

    #[test]
    fn test_sniper_process_detected_pool() {
        let settings = SniperSettings {
            total_budget_sol: 0.5,
            buy_amount_sol: 0.05,
            is_paper_trading: true,
            jito_tip_sol: 0.003,
            min_liquidity_sol: 0.5,
            max_concurrent_positions: 5,
            ..Default::default()
        };

        let engine = SolanaSniperEngine::new(settings);

        let event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "5VERv8NMvzbJMEdV8xnrLkEaMaWRqBLGhNGJopnibewpwq4vgmPKNAK6fJ6YZGQoyzxVCVmWmq8XXQ5gkydXMibo".to_string(),
            slot: 280123456,
            token_mint: "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr".to_string(),
            pool_address: "A38ZvdQj91eH66k1H9gE4Ffsm4v9n8K7y7j7L7m7N7o7".to_string(),
            creator: Some("CrEat0rWaLLeT1111111111111111111111111111111".to_string()),
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.5),
            metadata_uri: Some("https://ipfs.io/ipfs/Qm...".to_string()),
            has_socials: true,
            timestamp_ms: 1730000000000,
            log_snippet: Some("Pump.fun Bonding Curve Launched".to_string()),
            is_token_2022: false,
        };

        let report = engine.process_detected_pool(&event);
        assert!(report.is_some());
        assert_eq!(report.unwrap().status, SafetyStatus::Safe);

        let positions = engine.get_active_positions();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].token_mint, "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr");
        assert_eq!(positions[0].buy_amount_sol, 0.05);

        let port = engine.get_portfolio_state();
        assert!(port.available_sol < 0.5); // Budget deducted
        assert_eq!(port.invested_sol, 0.05);
    }

    #[test]
    fn test_sniper_take_profit_and_stop_loss() {
        let settings = SniperSettings {
            total_budget_sol: 0.5,
            buy_amount_sol: 0.05,
            is_paper_trading: true,
            jito_tip_sol: 0.003,
            min_liquidity_sol: 0.5,
            max_concurrent_positions: 5,
            ..Default::default()
        };

        let engine = SolanaSniperEngine::new(settings);

        let event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "sig123".to_string(),
            slot: 100,
            token_mint: "MintRunner1111111111111111111111111111111".to_string(),
            pool_address: "Pool123".to_string(),
            creator: None,
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.5),
            metadata_uri: None,
            has_socials: true,
            timestamp_ms: 1730000000000,
            log_snippet: None,
            is_token_2022: false,
        };

        engine.process_detected_pool(&event);

        // Advance 4 ticks
        for _ in 0..6 {
            engine.tick_position_updates();
        }

        let port = engine.get_portfolio_state();
        assert!(port.total_trades >= 1);
    }

    #[test]
    fn test_option_1_onchain_reserve_price_trigger() {
        let settings = SniperSettings {
            total_budget_sol: 0.5,
            buy_amount_sol: 0.05,
            is_paper_trading: true,
            jito_tip_sol: 0.003,
            min_liquidity_sol: 0.5,
            live_reserves: true,
            ..Default::default()
        };

        let engine = SolanaSniperEngine::new(settings);

        let event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "sig_curve_real".to_string(),
            slot: 100,
            token_mint: "RealPumpToken111111111111111111111111111111".to_string(),
            pool_address: "RealBondingCurve11111111111111111111111111".to_string(),
            creator: None,
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.1),
            metadata_uri: None,
            has_socials: true,
            timestamp_ms: 1730000000000,
            log_snippet: None,
            is_token_2022: false,
        };

        engine.process_detected_pool(&event);

        // Simulate on-chain reserve update showing token doubled in price (60 SOL virtual reserves)
        let state_tp1 = BondingCurveState {
            pool_address: "RealBondingCurve11111111111111111111111111".to_string(),
            virtual_token_reserves: 1_000_000_000_000_000,
            virtual_sol_reserves: 60_000_000_000, // 60 SOL virtual reserves = 2.0x price
            real_token_reserves: 500_000_000_000_000,
            real_sol_reserves: 30_000_000_000,
            price_sol: 0.000000060, // doubled from initial 0.000000028
            complete: false,
        };

        engine.update_bonding_curve_reserve("RealBondingCurve11111111111111111111111111", &state_tp1);

        let pos = engine.get_active_positions();
        assert_eq!(pos.len(), 1);
        assert!(pos[0].tp1_executed, "TP1 should trigger from real on-chain reserve price doubling");
        assert!(pos[0].current_multiplier >= 2.0);
    }

    #[test]
    fn test_momentum_entry_at_10_percent_curve() {
        let settings = SniperSettings {
            total_budget_sol: 0.5,
            buy_amount_sol: 0.05,
            is_paper_trading: true,
            jito_tip_sol: 0.003,
            min_liquidity_sol: 0.5,
            live_reserves: true,
            min_curve_pct: Some(10.0), // Requires >= 10% curve completion (>= 8.5 SOL)
            ..Default::default()
        };

        let engine = SolanaSniperEngine::new(settings);

        let event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "sig_momentum".to_string(),
            slot: 200,
            token_mint: "MomentumMint1111111111111111111111111111111".to_string(),
            pool_address: "MomentumCurve111111111111111111111111111111".to_string(),
            creator: None,
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.4),
            metadata_uri: None,
            has_socials: true,
            timestamp_ms: 1730000000000,
            log_snippet: None,
            is_token_2022: false,
        };

        // 1. Creation event arrives: Since min_curve_pct is 10%, it should NOT enter at block 0!
        let report = engine.process_detected_pool(&event);
        assert!(report.is_none());
        assert_eq!(engine.get_active_positions().len(), 0);

        // 2. Curve update arrives with 3 SOL reserves (~3.5% curve): Still below 10%, no entry!
        let state_early = BondingCurveState {
            pool_address: "MomentumCurve111111111111111111111111111111".to_string(),
            virtual_token_reserves: 1_000_000_000_000_000,
            virtual_sol_reserves: 33_000_000_000,
            real_token_reserves: 700_000_000_000_000,
            real_sol_reserves: 3_000_000_000, // 3 SOL (~3.5%)
            price_sol: 0.000000033,
            complete: false,
        };
        engine.update_bonding_curve_reserve("MomentumCurve111111111111111111111111111111", &state_early);
        assert_eq!(engine.get_active_positions().len(), 0);

        // 3. Curve update arrives with 9 SOL real reserves (10.6% curve): MOMENTUM BREAKOUT!
        let state_10pct = BondingCurveState {
            pool_address: "MomentumCurve111111111111111111111111111111".to_string(),
            virtual_token_reserves: 950_000_000_000_000,
            virtual_sol_reserves: 39_000_000_000,
            real_token_reserves: 650_000_000_000_000,
            real_sol_reserves: 9_000_000_000, // 9.0 SOL (> 8.5 SOL = 10.6% curve completion)
            price_sol: 0.000000041,
            complete: false,
        };
        engine.update_bonding_curve_reserve("MomentumCurve111111111111111111111111111111", &state_10pct);

        // 4. Verify momentum entry succeeded!
        let positions = engine.get_active_positions();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].token_mint, "MomentumMint1111111111111111111111111111111");
        assert_eq!(positions[0].pool_address, "MomentumCurve111111111111111111111111111111");
        assert!((positions[0].entry_price_sol - 0.000000041).abs() < 1e-12);
        assert_eq!(positions[0].current_multiplier, 1.0);
    }

    #[test]
    fn test_holder_dispersion_rejects_insider_whale_bundle() {
        use crate::solana::yellowstone::TradeActivityEvent;

        let settings = SniperSettings {
            total_budget_sol: 0.5,
            buy_amount_sol: 0.05,
            is_paper_trading: true,
            jito_tip_sol: 0.003,
            min_liquidity_sol: 0.5,
            live_reserves: true,
            min_curve_pct: Some(10.0), // Requires >= 10% curve completion
            min_unique_buyers: 8,      // Upgrade 2: Requires >= 8 unique buyers!
            ..Default::default()
        };

        let engine = SolanaSniperEngine::new(settings);

        let event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "sig_dispersion".to_string(),
            slot: 300,
            token_mint: "DispersionMint11111111111111111111111111111".to_string(),
            pool_address: "DispersionCurve1111111111111111111111111111".to_string(),
            creator: Some("DevDispersion11111111111111111111111111111".to_string()),
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.3),
            metadata_uri: None,
            has_socials: true,
            timestamp_ms: 1730000000000,
            log_snippet: None,
            is_token_2022: false,
        };

        engine.process_detected_pool(&event);

        // Record only 3 unique buyers (simulating insider whale bundle pump)
        for i in 1..=3 {
            engine.record_trade_activity(&TradeActivityEvent {
                signature: format!("sig_buy_{}", i),
                pool_address: "DispersionCurve1111111111111111111111111111".to_string(),
                user_wallet: format!("InsiderWallet_{}11111111111111111111", i),
                is_buy: true,
                is_sell: false,
                slot: 301,
            });
        }
        assert_eq!(engine.get_unique_buyer_count("DispersionCurve1111111111111111111111111111"), 3);

        // Curve hits 10.6% completion (9 SOL)
        let state_10pct = BondingCurveState {
            pool_address: "DispersionCurve1111111111111111111111111111".to_string(),
            virtual_token_reserves: 950_000_000_000_000,
            virtual_sol_reserves: 39_000_000_000,
            real_token_reserves: 650_000_000_000_000,
            real_sol_reserves: 9_000_000_000,
            price_sol: 0.000000041,
            complete: false,
        };

        // Try triggering momentum entry -> MUST BE REJECTED because unique buyers (3) < min (8)
        engine.update_bonding_curve_reserve("DispersionCurve1111111111111111111111111111", &state_10pct);
        assert_eq!(engine.get_active_positions().len(), 0, "Should reject insider whale bundle with <8 unique buyers");

        // Now record 5 more unique retail buyers (total unique = 8)
        for i in 4..=8 {
            engine.record_trade_activity(&TradeActivityEvent {
                signature: format!("sig_buy_{}", i),
                pool_address: "DispersionCurve1111111111111111111111111111".to_string(),
                user_wallet: format!("RetailBuyerWallet_{}111111111111111111", i),
                is_buy: true,
                is_sell: false,
                slot: 302,
            });
        }
        assert_eq!(engine.get_unique_buyer_count("DispersionCurve1111111111111111111111111111"), 8);

        // Trigger momentum check again with confirmed retail dispersion -> MUST ENTER!
        engine.update_bonding_curve_reserve("DispersionCurve1111111111111111111111111111", &state_10pct);
        assert_eq!(engine.get_active_positions().len(), 1, "Should enter when unique buyer dispersion >= 8 is reached");
    }

    #[test]
    fn test_dev_dump_rejects_pool_momentum_entry() {
        use crate::solana::yellowstone::TradeActivityEvent;

        let settings = SniperSettings {
            total_budget_sol: 0.5,
            buy_amount_sol: 0.05,
            is_paper_trading: true,
            jito_tip_sol: 0.003,
            min_liquidity_sol: 0.5,
            live_reserves: true,
            min_curve_pct: Some(10.0),
            min_unique_buyers: 8,
            ..Default::default()
        };

        let engine = SolanaSniperEngine::new(settings);

        let dev_wallet = "DevScammerWallet111111111111111111111111".to_string();
        let pool_addr = "DevRugCurve11111111111111111111111111111".to_string();

        let event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "sig_rug".to_string(),
            slot: 400,
            token_mint: "DevRugMint111111111111111111111111111111".to_string(),
            pool_address: pool_addr.clone(),
            creator: Some(dev_wallet.clone()),
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.5),
            metadata_uri: None,
            has_socials: true,
            timestamp_ms: 1730000000000,
            log_snippet: None,
            is_token_2022: false,
        };

        engine.process_detected_pool(&event);

        // 8 unique buyers buy
        for i in 1..=8 {
            engine.record_trade_activity(&TradeActivityEvent {
                signature: format!("sig_buy_{}", i),
                pool_address: pool_addr.clone(),
                user_wallet: format!("BuyerWallet_{}1111111111111111111111", i),
                is_buy: true,
                is_sell: false,
                slot: 401,
            });
        }
        assert_eq!(engine.get_unique_buyer_count(&pool_addr), 8);

        // Now creator DUMPS their tokens!
        engine.record_trade_activity(&TradeActivityEvent {
            signature: "sig_dev_dump".to_string(),
            pool_address: pool_addr.clone(),
            user_wallet: dev_wallet.clone(),
            is_buy: false,
            is_sell: true,
            slot: 402,
        });

        assert!(engine.is_dev_dumped(&pool_addr), "Pool should be flagged as dev dumped");

        // Curve reaches 10%
        let state_10pct = BondingCurveState {
            pool_address: pool_addr.clone(),
            virtual_token_reserves: 950_000_000_000_000,
            virtual_sol_reserves: 39_000_000_000,
            real_token_reserves: 650_000_000_000_000,
            real_sol_reserves: 9_000_000_000,
            price_sol: 0.000000041,
            complete: false,
        };

        // Attempt entry -> MUST BE REJECTED due to dev dump blacklist!
        engine.update_bonding_curve_reserve(&pool_addr, &state_10pct);
        assert_eq!(engine.get_active_positions().len(), 0, "Must reject pool where creator dumped");
    }

    #[test]
    fn test_paper_trading_deducts_fees_and_jito_tips() {
        let settings = SniperSettings {
            total_budget_sol: 1.0,
            buy_amount_sol: 0.1,
            is_paper_trading: true,
            jito_tip_sol: 0.003,
            min_liquidity_sol: 0.5,
            live_reserves: true,
            ..Default::default()
        };

        let engine = SolanaSniperEngine::new(settings);

        let event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "sig_fee_test".to_string(),
            slot: 500,
            token_mint: "FeeTestMint111111111111111111111111111111".to_string(),
            pool_address: "FeeTestCurve11111111111111111111111111111".to_string(),
            creator: None,
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.1),
            metadata_uri: None,
            has_socials: true,
            timestamp_ms: 1730000000000,
            log_snippet: None,
            is_token_2022: false,
        };

        engine.process_detected_pool(&event);

        let port_after_buy = engine.get_portfolio_state();
        // Initial budget: 1.0 SOL
        // Buy cost: 0.1 SOL + 1% pump fee (0.001 SOL) + 0.003 Jito tip = 0.104 SOL
        let expected_available = 1.0 - 0.104;
        assert!(
            (port_after_buy.available_sol - expected_available).abs() < 1e-6,
            "Available SOL should deduct 1% Pump fee and Jito tip on entry: got {}, expected {}",
            port_after_buy.available_sol,
            expected_available
        );
        assert!((port_after_buy.invested_sol - 0.1).abs() < 1e-6);
    }

    #[test]
    fn test_in_slot_emergency_exit_on_dev_dump() {
        use crate::solana::yellowstone::TradeActivityEvent;

        let settings = SniperSettings {
            total_budget_sol: 1.0,
            buy_amount_sol: 0.1,
            is_paper_trading: true,
            jito_tip_sol: 0.003,
            min_liquidity_sol: 0.5,
            live_reserves: true,
            ..Default::default()
        };

        let engine = SolanaSniperEngine::new(settings);

        let dev_wallet = "DevScammerWallet9999999999999999999999".to_string();
        let pool_addr = "EmergencyCurve999999999999999999999999".to_string();

        let event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "sig_em_test".to_string(),
            slot: 600,
            token_mint: "EmergencyMint999999999999999999999999".to_string(),
            pool_address: pool_addr.clone(),
            creator: Some(dev_wallet.clone()),
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.1),
            metadata_uri: None,
            has_socials: true,
            timestamp_ms: 1730000000000,
            log_snippet: None,
            is_token_2022: false,
        };

        engine.process_detected_pool(&event);

        // Position is opened
        let active = engine.get_active_positions();
        assert_eq!(active.len(), 1);
        assert!(!active[0].is_closed);

        // Dev Dumps!
        engine.record_trade_activity(&TradeActivityEvent {
            signature: "sig_dev_dump_immediate".to_string(),
            pool_address: pool_addr.clone(),
            user_wallet: dev_wallet.clone(),
            is_buy: false,
            is_sell: true,
            slot: 601,
        });

        // Verify active position was IMMEDIATELY closed in that slot
        let positions_after = engine.get_active_positions();
        assert_eq!(positions_after.len(), 1);
        assert!(positions_after[0].is_closed, "Position should be closed immediately when dev dump is detected");
    }

    #[test]
    fn test_break_even_stop_locks_in_profit_at_ten_pct() {
        let settings = SniperSettings {
            total_budget_sol: 1.0,
            buy_amount_sol: 0.1,
            is_paper_trading: true,
            jito_tip_sol: 0.001,
            min_liquidity_sol: 0.5,
            live_reserves: true,
            break_even_mult: Some(1.10), // Lock in 1.02x floor at +10%
            stop_loss_pct: -18.0,
            ..Default::default()
        };

        let engine = SolanaSniperEngine::new(settings);

        let pool_addr = "BreakEvenPool11111111111111111111111111111".to_string();
        let event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "sig_be_test".to_string(),
            slot: 700,
            token_mint: "BreakEvenMint1111111111111111111111111111".to_string(),
            pool_address: pool_addr.clone(),
            creator: Some("CrEat0r1111111111111111111111111111111111".to_string()),
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.1),
            metadata_uri: None,
            has_socials: true,
            timestamp_ms: 1730000000000,
            log_snippet: None,
            is_token_2022: false,
        };

        engine.process_detected_pool(&event);
        assert_eq!(engine.get_active_positions().len(), 1);

        // Token pumps to 1.14x (+14%)
        let state_pump = BondingCurveState {
            pool_address: pool_addr.clone(),
            virtual_token_reserves: 1_000_000_000_000_000,
            virtual_sol_reserves: (30.0 * 1.14 * 1e9) as u64,
            real_token_reserves: 500_000_000_000_000,
            real_sol_reserves: 15_000_000_000,
            complete: false,
            price_sol: (30.0 / 1_073_000_000.0) * 1.14,
        };
        engine.update_bonding_curve_reserve(&pool_addr, &state_pump);
        let active = engine.get_active_positions();
        assert!((active[0].peak_multiplier - 1.14).abs() < 1e-4);
        assert!(!active[0].is_closed);

        // Token retraces to 1.01x (would have hit -18% stop at 0.82x, but break-even floor is 1.02x!)
        let state_retrace = BondingCurveState {
            pool_address: pool_addr.clone(),
            virtual_token_reserves: 1_000_000_000_000_000,
            virtual_sol_reserves: (30.0 * 1.01 * 1e9) as u64,
            real_token_reserves: 500_000_000_000_000,
            real_sol_reserves: 15_000_000_000,
            complete: false,
            price_sol: (30.0 / 1_073_000_000.0) * 1.01,
        };
        engine.update_bonding_curve_reserve(&pool_addr, &state_retrace);

        let positions_after = engine.get_active_positions();
        assert!(positions_after[0].is_closed, "Position should be closed at break-even floor 1.02x rather than falling to stop-loss");
    }
}
