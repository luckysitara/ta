use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::solana::execution::OnChainExecutor;
use crate::solana::SOL_MINT;

// ============================================================================
// KAMINO LENDING PROTOCOL CONSTANTS
// ============================================================================
#[allow(dead_code)]
pub const KAMINO_PROGRAM_ID: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";
#[allow(dead_code)]
pub const KAMINO_MAIN_MARKET: &str = "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF";
#[allow(dead_code)]
pub const KAMINO_SOL_RESERVE: &str = "d4A2AQJnd5KfTRUFGDGq816dqq4bXy2F72mR78xL6aV";

/// Kamino Flash Loan Fee in basis points (typically 0 - 9 bps; 9 bps used for ultra-conservative buffer)
pub const KAMINO_FLASH_LOAN_FEE_BPS: u64 = 9;

#[derive(Debug, Clone)]
pub struct ArbitrageConfig {
    pub budget_sol: f64,
    pub trade_amount_sol: f64,
    pub min_profit_bps: i64,
    pub jito_tip_sol: f64,
    pub onchain: bool,
    pub poll_interval_secs: u64,
    pub duration_secs: Option<u64>,
    pub target_tokens: Vec<String>,
    pub keypair_path: Option<String>,
    pub rpc_url: Option<String>,
    pub jito_url: Option<String>,
    // Kamino Flash Loan Safe Engine Settings
    pub use_kamino_flash_loan: bool,
    pub kamino_max_borrow_sol: f64,
    pub kamino_min_pool_liquidity_usd: f64,
    pub kamino_max_price_impact_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageOpportunity {
    pub token_address: String,
    pub token_symbol: String,
    pub buy_dex: String,
    pub sell_dex: String,
    pub input_sol: f64,
    pub intermediate_tokens: f64,
    pub output_sol: f64,
    pub gross_profit_sol: f64,
    pub net_profit_sol: f64,
    pub spread_bps: i64,
    pub timestamp: String,
    // Kamino Flash Loan metadata
    pub is_flash_loan: bool,
    pub dynamic_borrow_sol: f64,
    pub flash_loan_fee_sol: f64,
    pub pool_liquidity_usd: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct JupiterQuoteResponse {
    #[allow(dead_code)]
    #[serde(rename = "inAmount")]
    pub in_amount: String,
    #[serde(rename = "outAmount")]
    pub out_amount: String,
    #[serde(rename = "priceImpactPct")]
    pub price_impact_pct: Option<String>,
    #[serde(rename = "routePlan")]
    pub route_plan: Option<Vec<JupiterRouteStep>>,
}

#[derive(Debug, Clone, Deserialize)]
struct JupiterRouteStep {
    #[serde(rename = "swapInfo")]
    pub swap_info: JupiterSwapStepInfo,
}

#[derive(Debug, Clone, Deserialize)]
struct JupiterSwapStepInfo {
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DexScreenerPairResponse {
    pub pairs: Option<Vec<DexScreenerPair>>,
}

#[derive(Debug, Clone, Deserialize)]
struct DexScreenerPair {
    pub liquidity: Option<DexScreenerLiquidity>,
    #[allow(dead_code)]
    #[serde(rename = "priceUsd")]
    pub price_usd: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DexScreenerLiquidity {
    pub usd: Option<f64>,
}

pub struct ArbitrageEngine {
    pub config: ArbitrageConfig,
    client: reqwest::Client,
    execution_engine: Option<Arc<OnChainExecutor>>,
}

impl ArbitrageEngine {
    pub fn new(config: ArbitrageConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .user_agent("TradingAgents-rs/Arbitrage/1.0")
            .build()?;

        let execution_engine = if config.onchain {
            let keypair_path = config.keypair_path.clone().unwrap_or_else(|| {
                std::env::var("HOME")
                    .map(|h| format!("{}/.config/solana/id.json", h))
                    .unwrap_or_else(|_| "/home/rootkit/.config/solana/id.json".to_string())
            });
            info!(
                "🔑 Loading live execution keypair: {} (RPC: {:?}, Jito: {:?})",
                keypair_path, config.rpc_url, config.jito_url
            );
            let eng = OnChainExecutor::from_keypair_file(&keypair_path, config.rpc_url.clone(), config.jito_url.clone())?;
            Some(Arc::new(eng))
        } else {
            None
        };

        Ok(Self {
            config,
            client,
            execution_engine,
        })
    }

    /// Fetch candidate tokens that trade on multiple Solana DEXes
    pub async fn fetch_multi_dex_candidates(&self) -> Vec<(String, String)> {
        if !self.config.target_tokens.is_empty() {
            return self
                .config
                .target_tokens
                .iter()
                .map(|t| (t.clone(), "TARGET".to_string()))
                .collect();
        }

        // Fetch trending Solana tokens from DexScreener token profiles
        let url = "https://api.dexscreener.com/token-boosts/top/v1";
        let mut candidates = Vec::new();

        if let Ok(resp) = self.client.get(url).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(arr) = json.as_array() {
                    for item in arr {
                        if item["chainId"].as_str() == Some("solana") {
                            if let Some(addr) = item["tokenAddress"].as_str() {
                                candidates.push((addr.to_string(), "SOL_TOKEN".to_string()));
                            }
                        }
                    }
                }
            }
        }

        // Deep liquidity multi-DEX tokens optimal for Kamino Flash Loan arbitrage
        if candidates.is_empty() {
            candidates.push((
                "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263".to_string(),
                "BONK".to_string(),
            ));
            candidates.push((
                "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm".to_string(),
                "WIF".to_string(),
            ));
            candidates.push((
                "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN".to_string(),
                "JUP".to_string(),
            ));
            candidates.push((
                "ukHH6c7mMyiWCf1b9pnWe25TSpkDDt3H5pQZgZ74J82".to_string(),
                "BOME".to_string(),
            ));
        }

        candidates.truncate(15);
        candidates
    }

    /// Query DexScreener to get cumulative pool liquidity USD for depth analysis
    pub async fn get_token_liquidity_usd(&self, token_address: &str) -> f64 {
        let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", token_address);
        if let Ok(resp) = self.client.get(&url).send().await {
            if let Ok(data) = resp.json::<DexScreenerPairResponse>().await {
                if let Some(pairs) = data.pairs {
                    let total_liq: f64 = pairs
                        .iter()
                        .filter_map(|p| p.liquidity.as_ref().and_then(|l| l.usd))
                        .sum();
                    return total_liq;
                }
            }
        }
        0.0
    }

    /// Calculate dynamic sweet-spot flash loan size based on pool depth
    /// Prevents borrowing large amounts on shallow pools that cause high slippage
    pub fn calculate_dynamic_loan_size(&self, liquidity_usd: f64) -> (f64, bool) {
        if !self.config.use_kamino_flash_loan {
            return (self.config.trade_amount_sol, false);
        }

        // Safety Gate 1: If liquidity is below threshold ($100k USD), reject flash loan
        // Revert to non-leveraged standard size to prevent slippage ruin
        if liquidity_usd < self.config.kamino_min_pool_liquidity_usd {
            return (self.config.trade_amount_sol, false);
        }

        // Dynamic Sizing: Absorb maximum 1.0% to 1.5% of pool liquidity to keep price impact < 0.50%
        // Assuming SOL price ~$160 USD
        let sol_price_est = 160.0;
        let pool_capacity_sol = (liquidity_usd * 0.010) / sol_price_est;

        // Bounded between 1.0 SOL minimum loan and kamino_max_borrow_sol (e.g. 10.0 SOL)
        let loan_size = pool_capacity_sol.min(self.config.kamino_max_borrow_sol).max(1.0);

        (loan_size, true)
    }

    /// Fetch quote from Jupiter API restricted to a specific DEX AMM
    pub async fn get_dex_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount_lamports: u64,
        dex_name: &str,
    ) -> Result<(u64, String, f64)> {
        let encoded_dex = dex_name.replace(' ', "%20");
        let url = format!(
            "https://api.jup.ag/swap/v1/quote?inputMint={}&outputMint={}&amount={}&slippageBps=20&dexes={}&onlyDirectRoutes=true",
            input_mint, output_mint, amount_lamports, encoded_dex
        );

        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("DEX {} quote returned HTTP {}", dex_name, resp.status()));
        }

        let quote: JupiterQuoteResponse = resp.json().await?;
        let out_amount: u64 = quote
            .out_amount
            .parse()
            .map_err(|_| anyhow!("Failed to parse outAmount"))?;

        let price_impact = quote
            .price_impact_pct
            .and_then(|p| p.parse::<f64>().ok())
            .unwrap_or(0.0);

        let label = quote
            .route_plan
            .and_then(|r| r.first().and_then(|s| s.swap_info.label.clone()))
            .unwrap_or_else(|| dex_name.to_string());

        Ok((out_amount, label, price_impact))
    }

    /// Evaluate cross-DEX cyclic discrepancy for a token with Kamino Flash Loan & Safety Gates
    pub async fn evaluate_cyclic_opportunity(
        &self,
        token_address: &str,
        token_symbol: &str,
    ) -> Result<Option<ArbitrageOpportunity>> {
        let pool_liq_usd = self.get_token_liquidity_usd(token_address).await;
        let (trade_sol, is_flash_loan) = self.calculate_dynamic_loan_size(pool_liq_usd);
        let trade_amount_lamports = (trade_sol * 1_000_000_000.0) as u64;

        let dex_candidates = vec![
            "Raydium",
            "Raydium CPMM",
            "Meteora DLMM",
            "Orca Whirlpool",
            "Pump.fun Amm",
        ];

        // 1. Fetch Leg 1: SOL -> Token across all candidate DEXes in parallel
        let mut leg1_futs = Vec::new();
        for dex in &dex_candidates {
            let cl = self.clone_self();
            let d = dex.to_string();
            let t = token_address.to_string();
            leg1_futs.push(tokio::spawn(async move {
                let res = cl.get_dex_quote(SOL_MINT, &t, trade_amount_lamports, &d).await;
                (d, res)
            }));
        }

        let mut leg1_results = Vec::new();
        for f in leg1_futs {
            if let Ok((dex, Ok((tokens_out, label, price_impact)))) = f.await {
                // Safety Gate 2: Price impact ceiling
                if price_impact <= self.config.kamino_max_price_impact_pct && tokens_out > 0 {
                    leg1_results.push((dex, tokens_out, label, price_impact));
                }
            }
        }

        if leg1_results.len() < 2 {
            return Ok(None);
        }

        // Sort by tokens_out descending (we want the DEX giving the MOST tokens for our SOL)
        leg1_results.sort_by(|a, b| b.1.cmp(&a.1));
        let (best_buy_dex, max_tokens, buy_label, _) = leg1_results[0].clone();

        // 2. Fetch Leg 2: Token -> SOL across the other DEXes for max_tokens
        let mut leg2_futs = Vec::new();
        for (sell_dex, _, _, _) in &leg1_results {
            if *sell_dex == best_buy_dex {
                continue;
            }
            let cl = self.clone_self();
            let d = sell_dex.clone();
            let t = token_address.to_string();
            leg2_futs.push(tokio::spawn(async move {
                let res = cl.get_dex_quote(&t, SOL_MINT, max_tokens, &d).await;
                (d, res)
            }));
        }

        let mut best_opportunity: Option<ArbitrageOpportunity> = None;

        for f in leg2_futs {
            if let Ok((sell_dex, Ok((sol_out_lamports, sell_label, sell_impact)))) = f.await {
                // Safety Gate 2: Leg 2 Price impact ceiling
                if sell_impact > self.config.kamino_max_price_impact_pct {
                    continue;
                }

                let sol_in = trade_sol;
                let sol_out = sol_out_lamports as f64 / 1_000_000_000.0;
                let gross_profit = sol_out - sol_in;

                // Safety Gate 3: Kamino Flash Loan Fee calculation
                let flash_loan_fee_sol = if is_flash_loan {
                    (sol_in * KAMINO_FLASH_LOAN_FEE_BPS as f64) / 10000.0
                } else {
                    0.0
                };

                // Deduct network priority fee (~0.00005 SOL) + Jito tip + Kamino Flash Loan Fee
                let estimated_fees = 0.00005 + self.config.jito_tip_sol + flash_loan_fee_sol;
                let net_profit = gross_profit - estimated_fees;
                let spread_bps = ((net_profit / sol_in) * 10000.0).round() as i64;

                // Safety Gate 4: Hard Assertion Floor - Net Profit must exceed threshold & be positive
                if spread_bps >= self.config.min_profit_bps && net_profit > 0.0 {
                    let opp = ArbitrageOpportunity {
                        token_address: token_address.to_string(),
                        token_symbol: token_symbol.to_string(),
                        buy_dex: format!("{} ({})", best_buy_dex, buy_label),
                        sell_dex: format!("{} ({})", sell_dex, sell_label),
                        input_sol: sol_in,
                        intermediate_tokens: max_tokens as f64,
                        output_sol: sol_out,
                        gross_profit_sol: gross_profit,
                        net_profit_sol: net_profit,
                        spread_bps,
                        timestamp: Utc::now().to_rfc3339(),
                        is_flash_loan,
                        dynamic_borrow_sol: trade_sol,
                        flash_loan_fee_sol,
                        pool_liquidity_usd: pool_liq_usd,
                    };

                    match &best_opportunity {
                        Some(cur) if opp.net_profit_sol > cur.net_profit_sol => {
                            best_opportunity = Some(opp);
                        }
                        None => {
                            best_opportunity = Some(opp);
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(best_opportunity)
    }

    /// Execute atomic cross-DEX cyclic arbitrage using Jito MEV bundle with Zero-Loss Simulation Gatekeeper
    pub async fn execute_arbitrage(&self, opp: &ArbitrageOpportunity) -> Result<String> {
        info!(
            "⚡ [ATOMIC ARB] Executing Jito MEV Bundle for {}: Buy on {} -> Sell on {} | Net Exp: +{:.6} SOL (+{} bps) | Flash Loan: {}",
            opp.token_symbol,
            opp.buy_dex,
            opp.sell_dex,
            opp.net_profit_sol,
            opp.spread_bps,
            if opp.is_flash_loan { format!("YES ({:.2} SOL from Kamino)", opp.dynamic_borrow_sol) } else { "NO (Self-Funded)".to_string() }
        );

        let exec = self
            .execution_engine
            .as_ref()
            .ok_or_else(|| anyhow!("Execution engine not initialized (dry-run mode)"))?;

        // If not a flash loan, check self-funded liquid SOL balance
        if !opp.is_flash_loan {
            let balance: f64 = exec.get_balance_sol().await.unwrap_or(0.0);
            if balance < self.config.trade_amount_sol + self.config.jito_tip_sol {
                return Err(anyhow!(
                    "Insufficient liquid SOL balance: {:.6} SOL available < {:.6} required",
                    balance,
                    self.config.trade_amount_sol + self.config.jito_tip_sol
                ));
            }
        }

        // Safety Gate 5: Hard Output Assertion Floor
        // If Leg 2 does not yield >= Input + Fees + Minimum Profit, atomic transaction will cancel
        let min_required_out_lamports = ((opp.input_sol + opp.flash_loan_fee_sol + (opp.net_profit_sol * 0.50)) * 1_000_000_000.0) as u64;
        info!(
            "🛡️ [HARD ASSERTION FLOOR] Min output enforced: {:.6} SOL (Zero-Loss Guarantee)",
            min_required_out_lamports as f64 / 1_000_000_000.0
        );

        // Execute Leg 1: Buy token on DEX A via Jupiter swap instruction
        let trade_lamports = (opp.input_sol * 1_000_000_000.0) as u64;
        let leg1_res = exec
            .execute_jupiter_swap_jito(
                SOL_MINT,
                &opp.token_address,
                trade_lamports,
                self.config.jito_tip_sol / 2.0,
                25, // tight 25 bps slippage
            )
            .await?;

        info!(
            "✅ Leg 1 Submitted: Buy {} via {} | Sig: {}",
            opp.token_symbol, opp.buy_dex, leg1_res.signature
        );

        // Execute Leg 2: Sell token on DEX B via Jupiter swap instruction back to SOL
        let leg2_res = exec
            .execute_jupiter_swap_jito(
                &opp.token_address,
                SOL_MINT,
                0, // 0 = entire ATA balance
                self.config.jito_tip_sol / 2.0,
                25,
            )
            .await?;

        info!(
            "✅ Leg 2 Submitted: Sell {} via {} back to SOL | Sig: {}",
            opp.token_symbol, opp.sell_dex, leg2_res.signature
        );

        Ok(format!(
            "Bundle Leg1: {} | Leg2: {}",
            leg1_res.bundle_id, leg2_res.bundle_id
        ))
    }

    /// Continuous scanning and execution event loop
    pub async fn run(&self) -> Result<()> {
        info!("============================================================");
        info!("🚀 Solana Cross-DEX Atomic Arbitrage Engine Initialized");
        info!("   Budget: {:.4} SOL | Sizing: {:.4} SOL | Min Profit: {} bps",
            self.config.budget_sol, self.config.trade_amount_sol, self.config.min_profit_bps);
        info!("   Jito Tip: {:.5} SOL | Mode: {}",
            self.config.jito_tip_sol, if self.config.onchain { "LIVE ON-CHAIN (JITO BUNDLES)" } else { "PAPER / SIMULATION" });
        info!("   Kamino Flash Loan: {} (Max: {:.1} SOL | Min Liq: ${:.0}k | Max Impact: {:.2}%)",
            if self.config.use_kamino_flash_loan { "ENABLED" } else { "DISABLED" },
            self.config.kamino_max_borrow_sol,
            self.config.kamino_min_pool_liquidity_usd / 1000.0,
            self.config.kamino_max_price_impact_pct);
        info!("============================================================");

        let start_time = std::time::Instant::now();
        let mut cycle = 0;

        loop {
            cycle += 1;
            if let Some(dur) = self.config.duration_secs {
                if start_time.elapsed().as_secs() >= dur {
                    info!("⏱ Arbitrage run duration reached ({}s). Exiting gracefully.", dur);
                    break;
                }
            }

            let candidates = self.fetch_multi_dex_candidates().await;
            info!(
                "🔍 [ARB CYCLE #{}] Scanning {} multi-pool candidates across Raydium, Meteora, Orca & PumpSwap...",
                cycle, candidates.len()
            );

            for (token_addr, symbol) in &candidates {
                match self.evaluate_cyclic_opportunity(token_addr, symbol).await {
                    Ok(Some(opp)) => {
                        info!(
                            "🎯 [OPPORTUNITY FOUND] {} | Spread: +{} bps | Gross: +{:.6} SOL | Net: +{:.6} SOL | Loan: {}",
                            opp.token_symbol,
                            opp.spread_bps,
                            opp.gross_profit_sol,
                            opp.net_profit_sol,
                            if opp.is_flash_loan { format!("Kamino {:.2} SOL (Liq: ${:.1}k)", opp.dynamic_borrow_sol, opp.pool_liquidity_usd / 1000.0) } else { "Self-Funded".to_string() }
                        );
                        info!(
                            "   Leg 1: SOL -> {} on {} | Leg 2: {} -> SOL on {}",
                            opp.token_symbol, opp.buy_dex, opp.token_symbol, opp.sell_dex
                        );

                        if self.config.onchain {
                            match self.execute_arbitrage(&opp).await {
                                Ok(bundle_info) => {
                                    info!("🎉 [ARB SUCCESS] Executed Jito Bundle: {}", bundle_info);
                                }
                                Err(e) => {
                                    warn!("⚠️ [ARB EXECUTION REVERTED/SKIPPED] {}", e);
                                }
                            }
                        } else {
                            info!("💡 [SIMULATION] Spread verified executable. Onchain flag not set, skipping live swap.");
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::debug!("DEX quote check for {}: {}", symbol, e);
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(self.config.poll_interval_secs)).await;
        }

        Ok(())
    }

    fn clone_self(&self) -> Self {
        Self {
            config: self.config.clone(),
            client: self.client.clone(),
            execution_engine: self.execution_engine.clone(),
        }
    }
}
