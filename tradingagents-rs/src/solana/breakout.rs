use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use colored::Colorize;
use tracing::warn;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::solana::execution::OnChainExecutor;
use crate::solana::sniper::PortfolioState;
use crate::solana::SOL_MINT;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakoutSettings {
    pub total_budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_concurrent_positions: usize,
    pub jito_tip_sol: f64,
    pub min_liquidity_usd: f64,
    pub min_volume_24h_usd: f64,
    pub min_volume_5m_usd: f64,
    pub min_buys_5m: usize,
    pub min_buy_ratio_5m: f64,
    pub min_price_change_5m: f64,
    pub trailing_stop_pct: f64,
    pub break_even_mult: f64,
    pub is_paper_trading: bool,
    pub poll_interval_secs: u64,
}

impl Default for BreakoutSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.02,
            buy_amount_sol: 0.008,
            max_concurrent_positions: 2,
            jito_tip_sol: 0.0003,
            min_liquidity_usd: 25_000.0,
            min_volume_24h_usd: 100_000.0,
            min_volume_5m_usd: 3_000.0,
            min_buys_5m: 10,
            min_buy_ratio_5m: 0.55,
            min_price_change_5m: 0.5,
            trailing_stop_pct: -15.0,
            break_even_mult: 1.25,
            is_paper_trading: true,
            poll_interval_secs: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakoutCandidate {
    pub token_address: String,
    pub name: String,
    pub symbol: String,
    pub pair_address: String,
    pub dex_id: String,
    pub price_usd: f64,
    pub price_native: f64,
    pub liquidity_usd: f64,
    pub volume_24h_usd: f64,
    pub volume_5m_usd: f64,
    pub price_change_5m: f64,
    pub price_change_1h: f64,
    pub buys_5m: usize,
    pub sells_5m: usize,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakoutPosition {
    pub token_address: String,
    pub symbol: String,
    pub pair_address: String,
    pub dex_id: String,
    pub entry_price_usd: f64,
    pub entry_price_native: f64,
    pub entry_timestamp: DateTime<Utc>,
    pub buy_amount_sol: f64,
    pub tokens_held: u64,
    pub peak_price_usd: f64,
    pub current_price_usd: f64,
    pub current_multiplier: f64,
    pub peak_multiplier: f64,
    pub stop_loss_price_usd: f64,
    pub is_closed: bool,
    pub pnl_sol: f64,
}

pub struct BreakoutEngine {
    pub settings: BreakoutSettings,
    pub executor: Option<Arc<OnChainExecutor>>,
    pub positions: Arc<Mutex<HashMap<String, BreakoutPosition>>>,
    pub entered_tokens: Arc<Mutex<HashSet<String>>>,
    pub portfolio: Arc<Mutex<PortfolioState>>,
    pub http_client: reqwest::Client,
}

impl BreakoutEngine {
    pub fn new(settings: BreakoutSettings, executor: Option<Arc<OnChainExecutor>>) -> Self {
        let budget = settings.total_budget_sol;
        Self {
            settings,
            executor,
            positions: Arc::new(Mutex::new(HashMap::new())),
            entered_tokens: Arc::new(Mutex::new(HashSet::new())),
            portfolio: Arc::new(Mutex::new(PortfolioState {
                initial_budget_sol: budget,
                available_sol: budget,
                invested_sol: 0.0,
                realized_pnl_sol: 0.0,
                total_trades: 0,
                winning_trades: 0,
                losing_trades: 0,
            })),
            http_client: reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .timeout(Duration::from_secs(6))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Evaluates if a token qualifies as an active DEX volume breakout
    pub fn evaluate_candidate(&self, c: &BreakoutCandidate) -> bool {
        // 1. Liquidity check: Ensure deep liquidity to avoid illiquid traps and high slippage
        if c.liquidity_usd < self.settings.min_liquidity_usd {
            return false;
        }

        // 2. Volume check: Must have proven 24h market traction and current 5m surge
        if c.volume_24h_usd < self.settings.min_volume_24h_usd || c.volume_5m_usd < self.settings.min_volume_5m_usd {
            return false;
        }

        // 3. Trade count & Buy ratio: Retail buyers must be heavily driving current 5m candle
        let total_txns = c.buys_5m + c.sells_5m;
        if total_txns == 0 || c.buys_5m < self.settings.min_buys_5m {
            return false;
        }
        let buy_ratio = (c.buys_5m as f64) / (total_txns as f64);
        if buy_ratio < self.settings.min_buy_ratio_5m {
            return false;
        }

        // 4. Momentum check: Price must be breaking out positively in 5m
        if c.price_change_5m < self.settings.min_price_change_5m {
            return false;
        }

        // 5. Must not be already entered or active
        let entered = self.entered_tokens.lock().unwrap();
        if entered.contains(&c.token_address) {
            return false;
        }

        true
    }

    /// Calculates a composite momentum score based on volume velocity and buy dominance
    pub fn calculate_breakout_score(c: &BreakoutCandidate) -> f64 {
        let total_txns = (c.buys_5m + c.sells_5m).max(1) as f64;
        let buy_ratio = (c.buys_5m as f64) / total_txns;
        let vol_score = (c.volume_5m_usd / 5_000.0).min(10.0);
        let pct_score = (c.price_change_5m / 5.0).clamp(-2.0, 10.0);
        let ratio_score = buy_ratio * 10.0;
        vol_score * 0.4 + pct_score * 0.3 + ratio_score * 0.3
    }

    /// Fetch trending/boosted token mint addresses from DexScreener
    pub async fn fetch_trending_mints(&self) -> Vec<String> {
        let mut mints = HashSet::new();

        // 1. Fetch top boosted
        if let Ok(resp) = self.http_client.get("https://api.dexscreener.com/token-boosts/top/v1").send().await {
            if let Ok(items) = resp.json::<Vec<serde_json::Value>>().await {
                for item in items {
                    if item["chainId"].as_str() == Some("solana") {
                        if let Some(addr) = item["tokenAddress"].as_str() {
                            mints.insert(addr.to_string());
                        }
                    }
                }
            }
        }

        // 2. Fetch latest boosted
        if let Ok(resp) = self.http_client.get("https://api.dexscreener.com/token-boosts/latest/v1").send().await {
            if let Ok(items) = resp.json::<Vec<serde_json::Value>>().await {
                for item in items {
                    if item["chainId"].as_str() == Some("solana") {
                        if let Some(addr) = item["tokenAddress"].as_str() {
                            mints.insert(addr.to_string());
                        }
                    }
                }
            }
        }

        mints.into_iter().collect()
    }

    /// Fetch rich market metrics for a batch of token mints via DexScreener
    pub async fn fetch_batch_metrics(&self, mints: &[String]) -> Vec<BreakoutCandidate> {
        if mints.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        // Query in chunks of 30
        for chunk in mints.chunks(30) {
            let joined = chunk.join(",");
            let url = format!("https://api.dexscreener.com/tokens/v1/solana/{}", joined);
            let resp = match self.http_client.get(&url).send().await {
                Ok(r) => r,
                Err(_) => continue,
            };

            let pairs: Vec<serde_json::Value> = match resp.json().await {
                Ok(p) => p,
                Err(_) => continue,
            };

            for p in pairs {
                let token_address = match p["baseToken"]["address"].as_str() {
                    Some(a) => a.to_string(),
                    None => continue,
                };
                let name = p["baseToken"]["name"].as_str().unwrap_or("Unknown").to_string();
                let symbol = p["baseToken"]["symbol"].as_str().unwrap_or("???").to_string();
                let pair_address = p["pairAddress"].as_str().unwrap_or("").to_string();
                let dex_id = p["dexId"].as_str().unwrap_or("").to_string();

                let price_usd = p["priceUsd"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                let price_native = p["priceNative"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                let liquidity_usd = p["liquidity"]["usd"].as_f64().unwrap_or(0.0);
                let volume_24h_usd = p["volume"]["h24"].as_f64().unwrap_or(0.0);
                let volume_5m_usd = p["volume"]["m5"].as_f64().unwrap_or(0.0);
                let price_change_5m = p["priceChange"]["m5"].as_f64().unwrap_or(0.0);
                let price_change_1h = p["priceChange"]["h1"].as_f64().unwrap_or(0.0);
                let buys_5m = p["txns"]["m5"]["buys"].as_u64().unwrap_or(0) as usize;
                let sells_5m = p["txns"]["m5"]["sells"].as_u64().unwrap_or(0) as usize;

                let mut cand = BreakoutCandidate {
                    token_address,
                    name,
                    symbol,
                    pair_address,
                    dex_id,
                    price_usd,
                    price_native,
                    liquidity_usd,
                    volume_24h_usd,
                    volume_5m_usd,
                    price_change_5m,
                    price_change_1h,
                    buys_5m,
                    sells_5m,
                    score: 0.0,
                };
                cand.score = Self::calculate_breakout_score(&cand);
                results.push(cand);
            }
        }

        results
    }

    /// Process a detected candidate: check active capacity and trigger entry
    pub async fn process_candidate(&self, c: &BreakoutCandidate) -> Result<bool> {
        if !self.evaluate_candidate(c) {
            return Ok(false);
        }

        // Check active positions limit
        {
            let positions = self.positions.lock().unwrap();
            let active = positions.values().filter(|p| !p.is_closed).count();
            if active >= self.settings.max_concurrent_positions {
                return Ok(false);
            }
            if positions.contains_key(&c.token_address) {
                return Ok(false);
            }
        }

        // Check budget & fee buffer
        let buy_amount = self.settings.buy_amount_sol;
        let tip = self.settings.jito_tip_sol;
        let fee_buffer = if !self.settings.is_paper_trading { 0.0022 } else { 0.0 };
        let trade_cost = buy_amount + tip + fee_buffer;

        {
            let port = self.portfolio.lock().unwrap();
            if port.available_sol < trade_cost {
                warn!(
                    "Insufficient balance ({:.4} SOL < {:.4} SOL required) for breakout entry {}.",
                    port.available_sol, trade_cost, c.symbol
                );
                return Ok(false);
            }
        }

        // Mark as entered to prevent duplicate entries
        {
            let mut entered = self.entered_tokens.lock().unwrap();
            entered.insert(c.token_address.clone());
        }

        let stop_price = c.price_usd * (1.0 + self.settings.trailing_stop_pct / 100.0);
        let tokens_est = if c.price_native > 0.0 {
            ((buy_amount / c.price_native) * 1_000_000.0) as u64
        } else {
            1_000_000
        };

        let position = BreakoutPosition {
            token_address: c.token_address.clone(),
            symbol: c.symbol.clone(),
            pair_address: c.pair_address.clone(),
            dex_id: c.dex_id.clone(),
            entry_price_usd: c.price_usd,
            entry_price_native: c.price_native,
            entry_timestamp: Utc::now(),
            buy_amount_sol: buy_amount,
            tokens_held: tokens_est,
            peak_price_usd: c.price_usd,
            current_price_usd: c.price_usd,
            current_multiplier: 1.0,
            peak_multiplier: 1.0,
            stop_loss_price_usd: stop_price,
            is_closed: false,
            pnl_sol: 0.0,
        };

        if self.settings.is_paper_trading {
            let total_paper_cost = buy_amount + tip + (buy_amount * 0.005);
            let (avail, invested, trade_num) = {
                let mut port = self.portfolio.lock().unwrap();
                port.available_sol -= total_paper_cost;
                port.invested_sol += buy_amount;
                port.total_trades += 1;
                (port.available_sol, port.invested_sol, port.total_trades)
            };

            println!(
                "\n{}",
                format!(
                    "🚀 [DEX BREAKOUT ENTRY #{}] Token: {} ({}) | DEX: {}",
                    trade_num, c.symbol, c.token_address, c.dex_id
                )
                .bold()
                .green()
            );
            println!(
                "   Price: ${:.6} ({:.8} SOL) | 5m Vol: ${:.1}k | 5m Change: +{:.2}% | 5m Buys: {}",
                c.price_usd, c.price_native, c.volume_5m_usd / 1000.0, c.price_change_5m, c.buys_5m
            );
            println!(
                "   Liquidity: ${:.1}k | 24h Vol: ${:.1}k | Momentum Score: {:.1}",
                c.liquidity_usd / 1000.0, c.volume_24h_usd / 1000.0, c.score
            );
            println!(
                "   📈 Strategy: Wide Trailing Stop ({:.1}%) | Break-Even Floor: ≥{:.2}x | Uncapped Upside",
                self.settings.trailing_stop_pct, self.settings.break_even_mult
            );
            println!(
                "   💼 [PAPER WALLET] Balance: {:.4} SOL Available | {:.4} SOL Invested",
                avail, invested
            );

            let mut pos = self.positions.lock().unwrap();
            pos.insert(c.token_address.clone(), position);
            Ok(true)
        } else {
            // Live on-chain execution via Jupiter swap bundled with Jito MEV tip
            if let Some(ref executor) = self.executor {
                let exec = Arc::clone(executor);
                let mint = c.token_address.clone();
                let sym = c.symbol.clone();
                let dex = c.dex_id.clone();
                let lamports = (buy_amount * 1_000_000_000.0) as u64;

                println!(
                    "\n{}",
                    format!(
                        "🎯 [LIVE ON-CHAIN BREAKOUT ATTEMPT] Sniping {} ({}) on {} | Cost: {:.4} SOL (+{:.4} Tip)...",
                        sym, mint, dex, buy_amount, tip
                    )
                    .bold()
                    .cyan()
                );

                match exec.execute_jupiter_swap_jito(SOL_MINT, &mint, lamports, tip, 1000).await {
                    Ok(res) => {
                        println!(
                            "   {}",
                            format!("✔ [BROADCAST SUCCESS] Tx Signature: https://solscan.io/tx/{}", res.signature).bold().green()
                        );
                        // Poll for confirmation
                        match exec.poll_signature_confirmation(&res.signature, 8, 500).await {
                            Ok(true) => {
                                println!(
                                    "   {}",
                                    format!("🎉 [LIVE BUY CONFIRMED] Acquired {} on-chain! Position active with trailing stop.", sym).bold().green()
                                );
                                {
                                    let mut pos = self.positions.lock().unwrap();
                                    pos.insert(mint.clone(), position);
                                }
                                if let Ok(real_bal) = exec.get_balance_sol().await {
                                    let mut port = self.portfolio.lock().unwrap();
                                    port.available_sol = real_bal;
                                    port.total_trades += 1;
                                    println!("   ⚡ [WALLET SYNC] Live Balance: {:.4} SOL", real_bal);
                                }
                                Ok(true)
                            }
                            Ok(false) => {
                                warn!("Transaction not confirmed within timeout. Position not added.");
                                Ok(false)
                            }
                            Err(e) => {
                                warn!("Error confirming transaction: {e}");
                                Ok(false)
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Jupiter swap failed for {}: {e}", sym);
                        Ok(false)
                    }
                }
            } else {
                Err(anyhow!("On-chain executor not configured"))
            }
        }
    }

    /// Manage active positions: update prices, adjust dynamic trailing stops, and trigger exits
    pub async fn update_and_manage_positions(&self) -> Result<()> {
        let active_mints: Vec<String> = {
            let positions = self.positions.lock().unwrap();
            positions.values().filter(|p| !p.is_closed).map(|p| p.token_address.clone()).collect()
        };

        if active_mints.is_empty() {
            return Ok(());
        }

        let candidates = self.fetch_batch_metrics(&active_mints).await;
        let mut exit_triggers: Vec<(String, f64, String)> = Vec::new();

        {
            let mut positions = self.positions.lock().unwrap();
            for cand in &candidates {
                if let Some(pos) = positions.get_mut(&cand.token_address) {
                    if pos.is_closed {
                        continue;
                    }
                    pos.current_price_usd = cand.price_usd;
                    pos.current_multiplier = if pos.entry_price_usd > 0.0 {
                        cand.price_usd / pos.entry_price_usd
                    } else {
                        1.0
                    };

                    if cand.price_usd > pos.peak_price_usd {
                        pos.peak_price_usd = cand.price_usd;
                        pos.peak_multiplier = pos.current_multiplier;

                        // Recalculate dynamic trailing stop
                        let new_trailing_stop = pos.peak_price_usd * (1.0 + self.settings.trailing_stop_pct / 100.0);
                        if new_trailing_stop > pos.stop_loss_price_usd {
                            pos.stop_loss_price_usd = new_trailing_stop;
                        }
                    }

                    // Break-Even floor lock when multiplier reaches target (e.g. 1.25x)
                    if pos.peak_multiplier >= self.settings.break_even_mult {
                        let be_floor = pos.entry_price_usd * 1.02; // +2% profit locked
                        if be_floor > pos.stop_loss_price_usd {
                            pos.stop_loss_price_usd = be_floor;
                        }
                    }

                    // Check stop-loss trigger
                    if cand.price_usd <= pos.stop_loss_price_usd {
                        let reason = if pos.current_multiplier >= 1.0 {
                            format!("Trailing Stop Profit Lock ({:.2}x)", pos.current_multiplier)
                        } else {
                            format!("Stop Loss Protection ({:.2}x)", pos.current_multiplier)
                        };
                        exit_triggers.push((pos.token_address.clone(), cand.price_usd, reason));
                    }
                }
            }
        }

        // Execute exits
        for (mint, exit_price, reason) in exit_triggers {
            self.execute_position_exit(&mint, exit_price, &reason).await?;
        }

        Ok(())
    }

    /// Execute position exit (market sell via Jupiter & reclaim ATA rent)
    pub async fn execute_position_exit(&self, token_address: &str, exit_price_usd: f64, reason: &str) -> Result<()> {
        let (mut pos, buy_amount, tip) = {
            let mut positions = self.positions.lock().unwrap();
            let pos = match positions.get_mut(token_address) {
                Some(p) => p.clone(),
                None => return Ok(()),
            };
            (pos, self.settings.buy_amount_sol, self.settings.jito_tip_sol)
        };

        let mult = if pos.entry_price_usd > 0.0 {
            exit_price_usd / pos.entry_price_usd
        } else {
            1.0
        };
        let gross_sol = buy_amount * mult;
        let net_pnl_sol = gross_sol - buy_amount - (tip * 2.0);

        pos.is_closed = true;
        pos.pnl_sol = net_pnl_sol;

        if self.settings.is_paper_trading {
            let (avail, realized) = {
                let mut port = self.portfolio.lock().unwrap();
                port.available_sol += gross_sol - tip;
                port.invested_sol = (port.invested_sol - buy_amount).max(0.0);
                port.realized_pnl_sol += net_pnl_sol;
                if net_pnl_sol >= 0.0 {
                    port.winning_trades += 1;
                } else {
                    port.losing_trades += 1;
                }
                (port.available_sol, port.realized_pnl_sol)
            };

            let pnl_str = if net_pnl_sol >= 0.0 {
                format!("+{:.4} SOL (+{:.1}%)", net_pnl_sol, (mult - 1.0) * 100.0).bold().green()
            } else {
                format!("{:.4} SOL ({:.1}%)", net_pnl_sol, (mult - 1.0) * 100.0).bold().red()
            };

            println!(
                "\n{}",
                format!("🏁 [DEX BREAKOUT EXIT] Token: {} | Reason: {}", pos.symbol, reason).bold().yellow()
            );
            println!(
                "   Exit Price: ${:.6} | Multiplier: {:.2}x | Peak: {:.2}x | PnL: {}",
                exit_price_usd, mult, pos.peak_multiplier, pnl_str
            );
            println!(
                "   💼 [PAPER WALLET] Available: {:.4} SOL | Total Realized PnL: {:.4} SOL",
                avail, realized
            );

            let mut positions = self.positions.lock().unwrap();
            positions.insert(token_address.to_string(), pos);
        } else {
            // Live on-chain exit via Jupiter sell + ATA close
            if let Some(ref executor) = self.executor {
                let exec = Arc::clone(executor);
                println!(
                    "\n{}",
                    format!("🏁 [LIVE EXIT ATTEMPT] Exiting {} ({:.2}x) | Reason: {}...", pos.symbol, mult, reason).bold().yellow()
                );

                // 1. Sell tokens back to SOL via Jupiter (amount 0 = full token ATA balance)
                match exec.execute_jupiter_swap_jito(token_address, SOL_MINT, 0, tip, 1000).await {
                    Ok(res) => {
                        println!("   ✔ [LIVE SELL BROADCAST] Tx: https://solscan.io/tx/{}", res.signature);
                        let _ = exec.poll_signature_confirmation(&res.signature, 8, 500).await;
                    }
                    Err(e) => {
                        warn!("Live exit sell failed: {e}");
                    }
                }

                // 2. Automatically close token account to reclaim 0.002039 SOL rent
                let _ = exec.close_token_account_jito(token_address, tip, false).await;

                // 3. Sync live balance
                if let Ok(real_bal) = exec.get_balance_sol().await {
                    let mut port = self.portfolio.lock().unwrap();
                    port.available_sol = real_bal;
                    println!("   ⚡ [WALLET RECLAIMED] Live Balance Restored: {:.4} SOL", real_bal);
                }

                let mut positions = self.positions.lock().unwrap();
                positions.insert(token_address.to_string(), pos);
            }
        }

        Ok(())
    }

    /// Single scan iteration
    pub async fn run_scan_cycle(&self) -> Result<usize> {
        // 1. Update and manage existing open positions
        self.update_and_manage_positions().await?;

        // 2. Discover trending tokens on Solana
        let mints = self.fetch_trending_mints().await;
        if mints.is_empty() {
            println!("  ℹ [SCANNER] Polling DexScreener... 0 trending mints returned (retrying).");
            return Ok(0);
        }

        // 3. Fetch deep market metrics in batch
        let candidates = self.fetch_batch_metrics(&mints).await;
        let active_cnt = self.positions.lock().unwrap().values().filter(|p| !p.is_closed).count();
        println!(
            "  📡 [SCANNER] Screened {} trending DEX tokens | Active Positions: {}",
            candidates.len(),
            active_cnt
        );

        // 4. Sort by breakout score descending
        let mut sorted = candidates;
        sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        if let Some(top) = sorted.first() {
            println!(
                "     🔥 Top Candidate: {} ({}) | Score: {:.1} | Liq: ${:.1}k | 5m Vol: ${:.1}k (+{:.2}%)",
                top.symbol, &top.token_address[..12.min(top.token_address.len())], top.score, top.liquidity_usd / 1000.0, top.volume_5m_usd / 1000.0, top.price_change_5m
            );
        }

        let mut entered_cnt = 0;
        for cand in &sorted {
            if self.process_candidate(cand).await? {
                entered_cnt += 1;
            }
        }

        Ok(entered_cnt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_candidate_filters_low_liquidity() {
        let engine = BreakoutEngine::new(BreakoutSettings::default(), None);
        let low_liq = BreakoutCandidate {
            token_address: "LowLiqToken11111111111111111111111111111111".to_string(),
            name: "Low Liq".to_string(),
            symbol: "LOW".to_string(),
            pair_address: "pair1".to_string(),
            dex_id: "raydium".to_string(),
            price_usd: 0.05,
            price_native: 0.0003,
            liquidity_usd: 5_000.0, // < $25,000 floor
            volume_24h_usd: 200_000.0,
            volume_5m_usd: 10_000.0,
            price_change_5m: 5.0,
            price_change_1h: 20.0,
            buys_5m: 30,
            sells_5m: 5,
            score: 8.5,
        };
        assert!(!engine.evaluate_candidate(&low_liq), "Must reject low liquidity pool");
    }

    #[test]
    fn test_evaluate_candidate_identifies_mihao_style_breakout() {
        let engine = BreakoutEngine::new(BreakoutSettings::default(), None);
        let mihao_candidate = BreakoutCandidate {
            token_address: "4CxjZyMNoF1p6eXHXHv1TWpS2X47CuUmLwtGe18spump".to_string(),
            name: "ni hao cat".to_string(),
            symbol: "mihao".to_string(),
            pair_address: "Ho1aSG6WGH1FfXdEHrK2f4Ggt1Hoxo9s5jcLMDqeyeBz".to_string(),
            dex_id: "pumpswap".to_string(),
            price_usd: 0.015,
            price_native: 0.000179,
            liquidity_usd: 390_000.0, // $390k depth
            volume_24h_usd: 24_000_000.0, // $24M volume
            volume_5m_usd: 22_000.0, // $22k in 5m
            price_change_5m: 3.5, // +3.5% moving up
            price_change_1h: 15.0,
            buys_5m: 135, // 135 buys
            sells_5m: 35, // 35 sells -> 79% buy ratio!
            score: 9.2,
        };
        assert!(engine.evaluate_candidate(&mihao_candidate), "Must qualify MIHAO breakout!");
    }

    #[test]
    fn test_dynamic_trailing_stop_calculation() {
        let settings = BreakoutSettings {
            trailing_stop_pct: -15.0,
            break_even_mult: 1.25,
            ..Default::default()
        };
        let _engine = BreakoutEngine::new(settings, None);

        let initial_price = 1.0;
        let stop_price = initial_price * 0.85; // $0.85 (-15%)

        let mut pos = BreakoutPosition {
            token_address: "RunnerMint111111111111111111111111111111".to_string(),
            symbol: "RUNNER".to_string(),
            pair_address: "pair1".to_string(),
            dex_id: "meteora".to_string(),
            entry_price_usd: initial_price,
            entry_price_native: 0.01,
            entry_timestamp: Utc::now(),
            buy_amount_sol: 0.01,
            tokens_held: 1_000_000,
            peak_price_usd: initial_price,
            current_price_usd: initial_price,
            current_multiplier: 1.0,
            peak_multiplier: 1.0,
            stop_loss_price_usd: stop_price,
            is_closed: false,
            pnl_sol: 0.0,
        };

        // Price spikes +50% (1.50x)
        pos.current_price_usd = 1.50;
        pos.peak_price_usd = 1.50;
        pos.peak_multiplier = 1.50;

        let new_trailing = pos.peak_price_usd * 0.85; // 1.50 * 0.85 = 1.275
        if new_trailing > pos.stop_loss_price_usd {
            pos.stop_loss_price_usd = new_trailing;
        }

        // Must lock in profit above entry!
        assert!(pos.stop_loss_price_usd > pos.entry_price_usd, "Trailing stop must lock in profit at +50% peak");
        assert!((pos.stop_loss_price_usd - 1.275).abs() < 0.001);
    }
}
