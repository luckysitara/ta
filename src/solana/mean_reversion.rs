use anyhow::Result;
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use crate::solana::execution::OnChainExecutor;
use crate::solana::sniper::PortfolioState;
use crate::solana::SOL_MINT;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeanReversionSettings {
    pub total_budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_concurrent_positions: usize,
    pub jito_tip_sol: f64,
    pub tracked_tokens: Vec<TrackedToken>,
    pub rsi_period: usize,
    pub rsi_oversold: f64,
    pub rsi_exit: f64,
    pub price_sample_interval_secs: u64,
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub max_hold_mins: u64,
    pub poll_interval_secs: u64,
    pub is_paper_trading: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedToken {
    pub mint: String,
    pub symbol: String,
}

impl Default for MeanReversionSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.2,
            buy_amount_sol: 0.03,
            max_concurrent_positions: 3,
            jito_tip_sol: 0.0001,
            tracked_tokens: vec![
                TrackedToken { mint: "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263".to_string(), symbol: "BONK".to_string() },
                TrackedToken { mint: "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm".to_string(), symbol: "WIF".to_string() },
                TrackedToken { mint: "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr".to_string(), symbol: "POPCAT".to_string() },
                TrackedToken { mint: "MEW1gQWJ3nEXg2qgERiKu7FAFj79PHvQVREQUzScPP5".to_string(), symbol: "MEW".to_string() },
                TrackedToken { mint: "A8C3xuqscfmyLrte3VmTqrAq8kgMASius9AFNANwpump".to_string(), symbol: "FARTCOIN".to_string() },
                TrackedToken { mint: "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN".to_string(), symbol: "JUP".to_string() },
                TrackedToken { mint: "ukHH6c7mMyiWCf1b9pnWe25TSpkDDt3H5pQZgZ74J82".to_string(), symbol: "BOME".to_string() },
                TrackedToken { mint: "HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3".to_string(), symbol: "PYTH".to_string() },
                TrackedToken { mint: "rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBXr".to_string(), symbol: "RENDER".to_string() },
                TrackedToken { mint: "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R".to_string(), symbol: "RAY".to_string() },
                TrackedToken { mint: "orcaEKTdK7LKz57vaAYr9QeNsVEPfiu6QeMU1kektZE".to_string(), symbol: "ORCA".to_string() },
                TrackedToken { mint: "KMNo3nJsBXfcpJTVhZcXLW7RmTwTt4GVFE7suUBo9sS".to_string(), symbol: "KMNO".to_string() },
                TrackedToken { mint: "DriFtupJYLTosbwoN8koMbEYSx54aFAVLddWsbksjwg7".to_string(), symbol: "DRIFT".to_string() },
                TrackedToken { mint: "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL".to_string(), symbol: "JTO".to_string() },
                TrackedToken { mint: "HeLp6NuQkmYB4pYWo2zYs22mESHXPQYzXbB8n4V98jwC".to_string(), symbol: "AI16Z".to_string() },
                TrackedToken { mint: "CzLSujWBLFsSjncfkh59rUFqvafWcY5tzedWJSuypump".to_string(), symbol: "GOAT".to_string() },
                TrackedToken { mint: "ED5nyyWEzpPPiWimP8vYm7sD7TD3LAt3Q3gRTWHzPJBY".to_string(), symbol: "MOODENG".to_string() },
                TrackedToken { mint: "2qEHjDLDLbuBgRYvsxhc5RefwhumuAZgnusqWqb86476".to_string(), symbol: "PNUT".to_string() },
                TrackedToken { mint: "9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump".to_string(), symbol: "FIDA".to_string() },
                TrackedToken { mint: "Grass7B4RdKfBCjTKgSqnXkqjwiGvQyFbuSCUJr3XXjs".to_string(), symbol: "GRASS".to_string() },
                TrackedToken { mint: "ioNETvc5NiB92q6Zk3YvNsp1qZtQ6Yh9B8y7T78Kpump".to_string(), symbol: "IO".to_string() },
                TrackedToken { mint: "TNSRxcUxoT9xBG3de7PiJyTDYu7kskLqcpddxnEJAS6".to_string(), symbol: "TNSR".to_string() },
                TrackedToken { mint: "ZEUS1a8r3a5hxBRqaHG3ux7A5upF2d84JwEP3STyx3r".to_string(), symbol: "ZEUS".to_string() },
                TrackedToken { mint: "WENWENvqqNya429ubCdXr81ZmD69brwQaaBYY6p3LCU".to_string(), symbol: "WEN".to_string() },
                TrackedToken { mint: "hntyVP6YFm1Hg25TN9WGLqM12b8TQmcknKrdu1oxWux".to_string(), symbol: "HNT".to_string() },
                TrackedToken { mint: "nosXBVoaCTtYdLvKY6Csb4AC8JCdQKKAaWYtx2ZMoo7".to_string(), symbol: "NOS".to_string() },
                TrackedToken { mint: "SHDWyBxihqiCj6YekG2GUr7wqKLeLAMK1gHZck9pL6y".to_string(), symbol: "SHDW".to_string() },
                TrackedToken { mint: "StepAscQoEioFxxWGnh2sLBDFp9d8rvKz2Yp39iDpyT".to_string(), symbol: "STEP".to_string() },
                TrackedToken { mint: "SLNDpmoWTVADgEdndQI29yKitggsuSTvmYUjECzUUCV".to_string(), symbol: "SLND".to_string() },
                TrackedToken { mint: "MNDEFzGvMt87ueuHvVU9VcTqsAP5b3fTGPsHuuPA5ey".to_string(), symbol: "MNDE".to_string() },
                TrackedToken { mint: "BLZEEuZUBVqFhj8adcCFPJvPVCiCyVmh3hkJMrU8KuJA".to_string(), symbol: "BLZE".to_string() },
                TrackedToken { mint: "RLBxxFkseAZ4RgJH3Sqn8jXxhmGoz9jWxDNJMh8LMTQ".to_string(), symbol: "RLB".to_string() },
            ],
            rsi_period: 14,
            rsi_oversold: 25.0,
            rsi_exit: 55.0,
            price_sample_interval_secs: 60,
            stop_loss_pct: -8.0,
            take_profit_pct: 15.0,
            max_hold_mins: 120,
            poll_interval_secs: 30,
            is_paper_trading: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeanReversionPosition {
    pub token_address: String,
    pub symbol: String,
    pub entry_price_sol: f64,
    pub entry_amount_sol: f64,
    pub tokens_held: f64,
    pub entry_time: DateTime<Utc>,
    pub entry_rsi: f64,
    pub current_price_sol: f64,
    pub current_rsi: f64,
    pub current_multiplier: f64,
    pub peak_multiplier: f64,
}

#[derive(Debug, Clone)]
struct PricePoint {
    timestamp: DateTime<Utc>,
    price: f64,
}

pub struct MeanReversionEngine {
    settings: MeanReversionSettings,
    executor: Option<OnChainExecutor>,
    positions: Arc<Mutex<HashMap<String, MeanReversionPosition>>>,
    entered_tokens: Arc<Mutex<HashSet<String>>>,
    portfolio: Arc<Mutex<PortfolioState>>,
    http_client: reqwest::Client,
    price_history: Arc<Mutex<HashMap<String, VecDeque<PricePoint>>>>,
}

impl MeanReversionEngine {
    pub fn new(settings: MeanReversionSettings, executor: Option<OnChainExecutor>) -> Self {
        let portfolio = PortfolioState {
            initial_budget_sol: settings.total_budget_sol,
            available_sol: settings.total_budget_sol,
            invested_sol: 0.0,
            realized_pnl_sol: 0.0,
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
        };

        Self {
            settings,
            executor,
            positions: Arc::new(Mutex::new(HashMap::new())),
            entered_tokens: Arc::new(Mutex::new(HashSet::new())),
            portfolio: Arc::new(Mutex::new(portfolio)),
            http_client: reqwest::Client::builder()
                .user_agent("TradingAgents-rs/0.1")
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            price_history: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get_portfolio_state(&self) -> PortfolioState {
        self.portfolio.lock().unwrap().clone()
    }

    pub fn restore_state(&self, portfolio: PortfolioState, positions: HashMap<String, MeanReversionPosition>) {
        let mut port = self.portfolio.lock().unwrap();
        *port = portfolio;
        let mut pos = self.positions.lock().unwrap();
        let mut entered = self.entered_tokens.lock().unwrap();
        for p in positions.values() {
            entered.insert(p.token_address.clone());
        }
        *pos = positions;
    }

    pub fn save_to_ledger(&self, ledger: &super::ledger::Ledger) {
        let port = self.portfolio.lock().unwrap();
        let positions = self.positions.lock().unwrap();
        if let Err(e) = ledger.save_generic_state("mean_reversion", &*port, &*positions) {
            warn!("Failed to save mean_reversion ledger: {}", e);
        }
    }

    async fn fetch_price(&self, token_mint: &str) -> Option<f64> {
        let url = format!("https://api.dexscreener.com/tokens/v1/solana/{}", token_mint);
        let resp = self.http_client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        #[derive(Deserialize)]
        struct Pair {
            #[serde(rename = "priceNative")]
            price_native: Option<String>,
            liquidity: Option<Liq>,
        }
        #[derive(Deserialize)]
        struct Liq {
            usd: Option<f64>,
        }

        let pairs: Vec<Pair> = resp.json().await.ok()?;
        let best = pairs.iter()
            .filter(|p| p.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0) > 10_000.0)
            .filter_map(|p| p.price_native.as_ref()?.parse::<f64>().ok())
            .next()?;

        Some(best)
    }

    fn record_price(&self, mint: &str, price: f64) {
        let mut history = self.price_history.lock().unwrap();
        let entries = history.entry(mint.to_string()).or_insert_with(VecDeque::new);
        entries.push_back(PricePoint {
            timestamp: Utc::now(),
            price,
        });
        let max_entries = self.settings.rsi_period * 3 + 10;
        while entries.len() > max_entries {
            entries.pop_front();
        }
    }

    fn calculate_rsi(&self, mint: &str) -> Option<f64> {
        let history = self.price_history.lock().unwrap();
        let entries = history.get(mint)?;

        // Require at least 2 full periods of data (e.g. 28 samples) to avoid startup noise
        let min_samples = self.settings.rsi_period * 2;
        if entries.len() < min_samples {
            return None;
        }

        let prices: Vec<f64> = entries.iter().map(|p| p.price).collect();
        let len = prices.len();
        let start = len.saturating_sub(self.settings.rsi_period + 1);
        let window = &prices[start..];

        let mut gains = 0.0_f64;
        let mut losses = 0.0_f64;

        for i in 1..window.len() {
            let change = window[i] - window[i - 1];
            if change > 0.0 {
                gains += change;
            } else {
                losses += change.abs();
            }
        }

        let period = self.settings.rsi_period as f64;
        let avg_gain = gains / period;
        let avg_loss = losses / period;

        // If price is completely flat or has zero price action, RSI is neutral 50, not oversold 0!
        if avg_gain == 0.0 && avg_loss == 0.0 {
            return Some(50.0);
        }

        if avg_loss == 0.0 {
            return Some(100.0);
        }

        // If gains are zero but total loss is microscopic (< 0.5% total drop across window), return neutral 50 to avoid startup false triggers
        let total_price = window.first().cloned().unwrap_or(1.0);
        if avg_gain == 0.0 && (losses / total_price) < 0.015 {
            return Some(50.0);
        }

        let rs = avg_gain / avg_loss;
        let rsi = 100.0 - (100.0 / (1.0 + rs));

        Some(rsi)
    }

    pub async fn run_scan_cycle(&self) -> Result<usize> {
        let mut entries = 0;

        for token in &self.settings.tracked_tokens {
            let price = match self.fetch_price(&token.mint).await {
                Some(p) => p,
                None => continue,
            };

            self.record_price(&token.mint, price);

            let rsi = match self.calculate_rsi(&token.mint) {
                Some(r) => r,
                None => continue,
            };

            // Update existing position
            {
                let mut positions = self.positions.lock().unwrap();
                if let Some(pos) = positions.get_mut(&token.mint) {
                    pos.current_price_sol = price;
                    pos.current_rsi = rsi;
                    pos.current_multiplier = price / pos.entry_price_sol;
                    if pos.current_multiplier > pos.peak_multiplier {
                        pos.peak_multiplier = pos.current_multiplier;
                    }
                    continue;
                }
            }

            // Check for entry signal
            if rsi > self.settings.rsi_oversold {
                continue;
            }

            {
                let entered = self.entered_tokens.lock().unwrap();
                if entered.contains(&token.mint) {
                    continue;
                }
            }

            {
                let positions = self.positions.lock().unwrap();
                if positions.len() >= self.settings.max_concurrent_positions {
                    break;
                }
            }

            {
                let port = self.portfolio.lock().unwrap();
                if port.available_sol < self.settings.buy_amount_sol + self.settings.jito_tip_sol {
                    break;
                }
            }

            let tokens = self.settings.buy_amount_sol / price;

            if !self.settings.is_paper_trading {
                if let Some(ref executor) = self.executor {
                    match executor
                        .execute_jupiter_swap_jito(
                            &SOL_MINT,
                            &token.mint,
                            (self.settings.buy_amount_sol * 1_000_000_000.0) as u64,
                            self.settings.jito_tip_sol,
                        500,
                        )
                        .await
                    {
                        Ok(result) => info!("[MEAN_REV] On-chain buy: {}", result.bundle_id),
                        Err(e) => {
                            warn!("[MEAN_REV] Buy failed for {}: {}", token.symbol, e);
                            continue;
                        }
                    }
                }
            }

            let mode = if self.settings.is_paper_trading { "PAPER" } else { "LIVE" };
            println!(
                "  {} [MEAN REVERSION {}] {} | RSI: {:.1} (oversold < {:.0}) | price: {:.9} SOL | cost: {:.4} SOL",
                "📉".green(),
                mode,
                token.symbol.bold().cyan(),
                rsi,
                self.settings.rsi_oversold,
                price,
                self.settings.buy_amount_sol,
            );

            let position = MeanReversionPosition {
                token_address: token.mint.clone(),
                symbol: token.symbol.clone(),
                entry_price_sol: price,
                entry_amount_sol: self.settings.buy_amount_sol,
                tokens_held: tokens,
                entry_time: Utc::now(),
                entry_rsi: rsi,
                current_price_sol: price,
                current_rsi: rsi,
                current_multiplier: 1.0,
                peak_multiplier: 1.0,
            };

            {
                let mut port = self.portfolio.lock().unwrap();
                port.available_sol -= self.settings.buy_amount_sol + self.settings.jito_tip_sol;
                port.invested_sol += self.settings.buy_amount_sol;
                port.total_trades += 1;
            }
            {
                let mut positions = self.positions.lock().unwrap();
                positions.insert(token.mint.clone(), position);
            }
            {
                let mut entered = self.entered_tokens.lock().unwrap();
                entered.insert(token.mint.clone());
            }

            entries += 1;

            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        self.check_exit_signals().await?;

        Ok(entries)
    }

    async fn check_exit_signals(&self) -> Result<()> {
        let mut to_close: Vec<(String, String)> = Vec::new();

        {
            let positions = self.positions.lock().unwrap();
            for (mint, pos) in positions.iter() {
                let pnl_pct = (pos.current_multiplier - 1.0) * 100.0;

                // Compute exact net PnL after all fees (DEX LP fee 0.30% entry + 0.30% exit + slippage ~0.20% + Jito tip & tx fees)
                let round_trip_fee_sol = (self.settings.jito_tip_sol * 2.0) + 0.000030 + (pos.entry_amount_sol * 0.0080);
                let current_sol_value = pos.entry_amount_sol * pos.current_multiplier;
                let net_pnl_sol = current_sol_value - pos.entry_amount_sol - round_trip_fee_sol;
                let net_pnl_pct = (net_pnl_sol / pos.entry_amount_sol) * 100.0;

                if pnl_pct <= self.settings.stop_loss_pct {
                    to_close.push((mint.clone(), format!("STOP_LOSS (Net: {:.2}%)", net_pnl_pct)));
                    continue;
                }

                if net_pnl_pct >= self.settings.take_profit_pct {
                    to_close.push((mint.clone(), format!("TAKE_PROFIT Net +{:.1}%", net_pnl_pct)));
                    continue;
                }

                // Only exit on RSI overbought if the trade is strictly NET positive after all fees
                if pos.current_rsi >= self.settings.rsi_exit && net_pnl_sol > 0.0 {
                    to_close.push((mint.clone(), format!("RSI_PROFIT Net +{:.2}% (+{:.6} SOL) (RSI {:.1})", net_pnl_pct, net_pnl_sol, pos.current_rsi)));
                    continue;
                }

                let hold_mins = (Utc::now() - pos.entry_time).num_minutes() as u64;
                if self.settings.max_hold_mins > 0 && hold_mins >= self.settings.max_hold_mins {
                    to_close.push((mint.clone(), format!("MAX_HOLD {}m", hold_mins)));
                }
            }
        }

        for (mint, reason) in to_close {
            self.close_position(&mint, &reason).await;
        }

        Ok(())
    }

    async fn close_position(&self, token_addr: &str, reason: &str) {
        let pos = {
            let mut positions = self.positions.lock().unwrap();
            match positions.remove(token_addr) {
                Some(p) => p,
                None => return,
            }
        };

        // Allow re-entry after cool-down
        {
            let mut entered = self.entered_tokens.lock().unwrap();
            entered.remove(token_addr);
        }

        let proceeds = pos.tokens_held * pos.current_price_sol;
        let pnl = proceeds - pos.entry_amount_sol;

        if !self.settings.is_paper_trading {
            if let Some(ref executor) = self.executor {
                match executor
                    .execute_jupiter_swap_jito(
                        token_addr,
                        &SOL_MINT,
                        0, // 0 = automatic 100% full ATA balance on-chain
                        self.settings.jito_tip_sol,
                        500,
                    )
                    .await
                {
                    Ok(res) => {
                        info!("[MEAN_REV] On-chain sell confirmed: {}", res.bundle_id);
                    }
                    Err(e) => {
                        error!("[MEAN_REV] On-chain sell failed for {}: {}", pos.symbol, e);
                    }
                }
            }
        }

        let pnl_display = if pnl >= 0.0 { format!("+{:.4}", pnl).green() } else { format!("{:.4}", pnl).red() };
        println!(
            "  {} [MEAN_REV EXIT] {} | {} | RSI: {:.1}->{:.1} | {:.2}x | PnL: {} SOL",
            if pnl >= 0.0 { "✅" } else { "🛑" },
            pos.symbol.bold(), reason.yellow(),
            pos.entry_rsi, pos.current_rsi,
            pos.current_multiplier, pnl_display,
        );

        let mut port = self.portfolio.lock().unwrap();
        port.available_sol += proceeds;
        port.invested_sol -= pos.entry_amount_sol;
        port.realized_pnl_sol += pnl;
        if pnl >= 0.0 { port.winning_trades += 1; } else { port.losing_trades += 1; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rsi_calculation() {
        let settings = MeanReversionSettings {
            rsi_period: 5,
            ..Default::default()
        };
        let engine = MeanReversionEngine::new(settings, None);

        // Record ascending prices: RSI should be high (need 2 * period = 10 samples)
        let prices = vec![1.0, 1.05, 1.1, 1.15, 1.2, 1.25, 1.3, 1.35, 1.4, 1.45, 1.5, 1.55, 1.6];
        for p in &prices {
            engine.record_price("test_token", *p);
        }

        let rsi = engine.calculate_rsi("test_token").unwrap();
        assert!(rsi > 80.0, "RSI should be high for ascending prices, got {}", rsi);
    }

    #[test]
    fn test_rsi_oversold() {
        let settings = MeanReversionSettings {
            rsi_period: 5,
            ..Default::default()
        };
        let engine = MeanReversionEngine::new(settings, None);

        // Record descending prices: RSI should be low (need 2 * period = 10 samples)
        let prices = vec![2.0, 1.95, 1.9, 1.85, 1.8, 1.75, 1.7, 1.65, 1.6, 1.55, 1.5, 1.45, 1.4];
        for p in &prices {
            engine.record_price("test_token", *p);
        }

        let rsi = engine.calculate_rsi("test_token").unwrap();
        assert!(rsi < 20.0, "RSI should be low for descending prices, got {}", rsi);
    }

    #[test]
    fn test_mean_reversion_defaults() {
        let s = MeanReversionSettings::default();
        assert_eq!(s.rsi_period, 14);
        assert_eq!(s.rsi_oversold, 25.0);
        assert_eq!(s.tracked_tokens.len(), 32);
    }
}
