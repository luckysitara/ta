use anyhow::Result;
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

use crate::solana::execution::OnChainExecutor;
use crate::solana::sniper::PortfolioState;
use crate::solana::SOL_MINT;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraduationSettings {
    pub total_budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_concurrent_positions: usize,
    pub jito_tip_sol: f64,
    pub min_curve_pct: f64,
    pub max_curve_pct: f64,
    pub min_market_cap_sol: f64,
    pub min_unique_holders: usize,
    pub min_volume_sol: f64,
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub graduation_exit_pct: f64,
    pub trailing_stop_pct: f64,
    pub break_even_mult: f64,
    pub poll_interval_secs: u64,
    pub is_paper_trading: bool,
}

impl Default for GraduationSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.15,
            buy_amount_sol: 0.03,
            max_concurrent_positions: 3,
            jito_tip_sol: 0.0003,
            min_curve_pct: 85.0,
            max_curve_pct: 99.0,
            min_market_cap_sol: 50.0,
            min_unique_holders: 50,
            min_volume_sol: 10.0,
            stop_loss_pct: -8.0,
            take_profit_pct: 30.0,
            graduation_exit_pct: 50.0,
            trailing_stop_pct: -12.0,
            break_even_mult: 1.06,
            poll_interval_secs: 5,
            is_paper_trading: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraduationPosition {
    pub token_address: String,
    pub symbol: String,
    pub entry_price_sol: f64,
    pub entry_amount_sol: f64,
    pub tokens_held: f64,
    pub entry_time: DateTime<Utc>,
    pub entry_curve_pct: f64,
    pub current_price_sol: f64,
    pub peak_price_sol: f64,
    pub peak_multiplier: f64,
    pub current_multiplier: f64,
    pub graduated: bool,
    pub graduation_sell_done: bool,
    pub trailing_active: bool,
    pub break_even_locked: bool,
}

#[derive(Debug, Deserialize)]
struct PumpFunToken {
    mint: String,
    #[serde(default)]
    symbol: String,
    #[serde(rename = "bondingCurveProgress", default)]
    bonding_curve_progress: Option<f64>,
    #[serde(rename = "marketCapSol", default)]
    market_cap_sol: Option<f64>,
    #[serde(rename = "vSolInBondingCurve", default)]
    v_sol_in_bonding_curve: Option<f64>,
    #[serde(rename = "replyCount", default)]
    reply_count: Option<u64>,
    #[serde(rename = "usdMarketCap", default)]
    usd_market_cap: Option<f64>,
    #[serde(rename = "priceInSol", default)]
    price_in_sol: Option<f64>,
    #[serde(rename = "holderCount", default)]
    holder_count: Option<usize>,
    #[serde(rename = "volumeSol24h", default)]
    volume_sol_24h: Option<f64>,
    #[serde(rename = "complete", default)]
    complete: bool,
    #[serde(rename = "raydiumPool", default)]
    raydium_pool: Option<String>,
}

pub struct GraduationEngine {
    settings: GraduationSettings,
    executor: Option<OnChainExecutor>,
    positions: Arc<Mutex<HashMap<String, GraduationPosition>>>,
    entered_tokens: Arc<Mutex<HashSet<String>>>,
    portfolio: Arc<Mutex<PortfolioState>>,
    http_client: reqwest::Client,
}

impl GraduationEngine {
    pub fn new(settings: GraduationSettings, executor: Option<OnChainExecutor>) -> Self {
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
        }
    }

    pub fn get_portfolio_state(&self) -> PortfolioState {
        self.portfolio.lock().unwrap().clone()
    }

    pub fn restore_state(&self, portfolio: PortfolioState, positions: HashMap<String, GraduationPosition>) {
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
        if let Err(e) = ledger.save_generic_state("graduation", &*port, &*positions) {
            warn!("Failed to save graduation ledger: {}", e);
        }
    }

    async fn fetch_near_graduation_tokens(&self) -> Result<Vec<PumpFunToken>> {
        let url = "https://frontend-api-v3.pump.fun/coins/about-to-graduate?limit=50&offset=0&includeNsfw=false";
        let resp = self.http_client.get(url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Pump.fun API returned {}", resp.status());
        }
        let tokens: Vec<PumpFunToken> = resp.json().await.unwrap_or_default();
        Ok(tokens)
    }

    fn evaluate_candidate(&self, token: &PumpFunToken) -> bool {
        if token.complete {
            return false;
        }

        let curve_pct = match token.bonding_curve_progress {
            Some(p) => p,
            None => return false,
        };

        if curve_pct < self.settings.min_curve_pct || curve_pct > self.settings.max_curve_pct {
            return false;
        }

        if let Some(mcap) = token.market_cap_sol {
            if mcap < self.settings.min_market_cap_sol {
                return false;
            }
        }

        if let Some(holders) = token.holder_count {
            if holders < self.settings.min_unique_holders {
                return false;
            }
        }

        if let Some(vol) = token.volume_sol_24h {
            if vol < self.settings.min_volume_sol {
                return false;
            }
        }

        true
    }

    pub async fn run_scan_cycle(&self) -> Result<usize> {
        let tokens = self.fetch_near_graduation_tokens().await?;
        let mut entries = 0;

        for token in &tokens {
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

            if !self.evaluate_candidate(token) {
                continue;
            }

            let price_sol = match token.price_in_sol {
                Some(p) if p > 0.0 => p,
                _ => continue,
            };

            let curve_pct = token.bonding_curve_progress.unwrap_or(0.0);
            let tokens_amount = self.settings.buy_amount_sol / price_sol;

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
                        Ok(result) => info!("[GRADUATION] On-chain buy: {}", result.bundle_id),
                        Err(e) => {
                            warn!("[GRADUATION] Buy failed for {}: {}", token.symbol, e);
                            continue;
                        }
                    }
                }
            }

            let mode = if self.settings.is_paper_trading { "PAPER" } else { "LIVE" };
            println!(
                "  {} [GRADUATION {} {}] {} | curve: {:.1}% | mcap: {:.1} SOL | price: {:.9} SOL | cost: {:.4} SOL",
                "🎓".green(),
                "FRONTRUN",
                mode,
                token.symbol.bold().cyan(),
                curve_pct,
                token.market_cap_sol.unwrap_or(0.0),
                price_sol,
                self.settings.buy_amount_sol,
            );

            let position = GraduationPosition {
                token_address: token.mint.clone(),
                symbol: token.symbol.clone(),
                entry_price_sol: price_sol,
                entry_amount_sol: self.settings.buy_amount_sol,
                tokens_held: tokens_amount,
                entry_time: Utc::now(),
                entry_curve_pct: curve_pct,
                current_price_sol: price_sol,
                peak_price_sol: price_sol,
                peak_multiplier: 1.0,
                current_multiplier: 1.0,
                graduated: false,
                graduation_sell_done: false,
                trailing_active: false,
                break_even_locked: false,
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
        }

        self.update_and_manage_positions().await?;

        Ok(entries)
    }

    async fn update_and_manage_positions(&self) -> Result<()> {
        let position_mints: Vec<String> = {
            let pos = self.positions.lock().unwrap();
            pos.keys().cloned().collect()
        };

        if position_mints.is_empty() {
            return Ok(());
        }

        for mint in &position_mints {
            let url = format!("https://frontend-api-v3.pump.fun/coins/{}", mint);
            let resp = self.http_client.get(&url).send().await;

            let token: Option<PumpFunToken> = match resp {
                Ok(r) if r.status().is_success() => r.json().await.ok(),
                _ => None,
            };

            if let Some(token) = token {
                let price_sol = token.price_in_sol.unwrap_or(0.0);
                if price_sol <= 0.0 {
                    continue;
                }

                let mut should_close = None;
                let mut partial_sell = false;

                {
                    let mut positions = self.positions.lock().unwrap();
                    if let Some(pos) = positions.get_mut(mint) {
                        pos.current_price_sol = price_sol;
                        pos.current_multiplier = price_sol / pos.entry_price_sol;

                        if price_sol > pos.peak_price_sol {
                            pos.peak_price_sol = price_sol;
                            pos.peak_multiplier = price_sol / pos.entry_price_sol;
                        }

                        if token.complete && !pos.graduated {
                            pos.graduated = true;
                            println!(
                                "  {} [GRADUATED] {} migrated to Raydium!",
                                "🚀", pos.symbol.bold().green()
                            );
                        }

                        if pos.current_multiplier >= self.settings.break_even_mult as f64 {
                            pos.break_even_locked = true;
                        }

                        let graduation_tp = 1.0 + (self.settings.graduation_exit_pct / 100.0);
                        if pos.graduated && !pos.graduation_sell_done && pos.current_multiplier >= 1.05 {
                            partial_sell = true;
                            pos.graduation_sell_done = true;
                        }

                        let tp_activation = 1.0 + (self.settings.take_profit_pct / 100.0);
                        if pos.peak_multiplier >= tp_activation {
                            pos.trailing_active = true;
                        }

                        let pnl_pct = (pos.current_multiplier - 1.0) * 100.0;

                        if pnl_pct <= self.settings.stop_loss_pct {
                            should_close = Some("STOP_LOSS".to_string());
                        } else if pos.break_even_locked && pos.current_multiplier < 1.005 {
                            should_close = Some("BREAK_EVEN".to_string());
                        } else if pos.trailing_active {
                            let drawdown = (pos.current_multiplier / pos.peak_multiplier - 1.0) * 100.0;
                            if drawdown <= self.settings.trailing_stop_pct {
                                should_close = Some(format!("TRAILING peak={:.2}x", pos.peak_multiplier));
                            }
                        }
                    }
                }

                if partial_sell {
                    self.partial_sell(mint, 0.5, "GRADUATION_TP").await;
                }

                if let Some(reason) = should_close {
                    self.close_position(mint, &reason).await;
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        Ok(())
    }

    async fn partial_sell(&self, token_addr: &str, fraction: f64, reason: &str) {
        let sell_tokens = {
            let mut positions = self.positions.lock().unwrap();
            match positions.get_mut(token_addr) {
                Some(pos) => {
                    let sell = pos.tokens_held * fraction;
                    pos.tokens_held -= sell;
                    let proceeds = sell * pos.current_price_sol;
                    let entry_portion = pos.entry_amount_sol * fraction;
                    let pnl = proceeds - entry_portion;
                    pos.entry_amount_sol -= entry_portion;

                    let mut port = self.portfolio.lock().unwrap();
                    port.available_sol += proceeds;
                    port.invested_sol -= entry_portion;
                    port.realized_pnl_sol += pnl;
                    if pnl >= 0.0 { port.winning_trades += 1; }

                    println!(
                        "  {} [GRADUATION {}] {} | sold {:.0}% | {:.2}x | PnL: {}{:.4} SOL",
                        "🎓", reason,
                        pos.symbol.bold(),
                        fraction * 100.0,
                        pos.current_multiplier,
                        if pnl >= 0.0 { "+" } else { "" },
                        pnl,
                    );

                    sell
                }
                None => return,
            }
        };

        if !self.settings.is_paper_trading {
            if let Some(ref executor) = self.executor {
                let _ = executor
                    .execute_jupiter_swap_jito(
                        token_addr,
                        &SOL_MINT,
                        (sell_tokens * 1_000_000_000.0) as u64,
                        self.settings.jito_tip_sol,
                            500,
                    )
                    .await;
            }
        }
    }

    async fn close_position(&self, token_addr: &str, reason: &str) {
        let pos = {
            let mut positions = self.positions.lock().unwrap();
            match positions.remove(token_addr) {
                Some(p) => p,
                None => return,
            }
        };

        let proceeds = pos.tokens_held * pos.current_price_sol;
        let pnl = proceeds - pos.entry_amount_sol;

        if !self.settings.is_paper_trading {
            if let Some(ref executor) = self.executor {
                let _ = executor
                    .execute_jupiter_swap_jito(
                        token_addr,
                        &SOL_MINT,
                        (pos.tokens_held * 1_000_000_000.0) as u64,
                        self.settings.jito_tip_sol,
                            500,
                    )
                    .await;
            }
        }

        let pnl_display = if pnl >= 0.0 { format!("+{:.4}", pnl).green() } else { format!("{:.4}", pnl).red() };
        println!(
            "  {} [GRADUATION EXIT] {} | {} | {:.2}x (peak {:.2}x) | PnL: {} SOL",
            if pnl >= 0.0 { "✅" } else { "🛑" },
            pos.symbol.bold(), reason.yellow(),
            pos.current_multiplier, pos.peak_multiplier, pnl_display,
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
    fn test_graduation_defaults() {
        let s = GraduationSettings::default();
        assert_eq!(s.min_curve_pct, 85.0);
        assert_eq!(s.max_curve_pct, 99.0);
        assert!(s.is_paper_trading);
    }

    #[test]
    fn test_evaluate_candidate_in_range() {
        let settings = GraduationSettings::default();
        let engine = GraduationEngine::new(settings, None);

        let token = PumpFunToken {
            mint: "test".to_string(),
            symbol: "TEST".to_string(),
            bonding_curve_progress: Some(92.0),
            market_cap_sol: Some(100.0),
            v_sol_in_bonding_curve: Some(50.0),
            reply_count: None,
            usd_market_cap: None,
            price_in_sol: Some(0.0001),
            holder_count: Some(100),
            volume_sol_24h: Some(20.0),
            complete: false,
            raydium_pool: None,
        };
        assert!(engine.evaluate_candidate(&token));
    }

    #[test]
    fn test_evaluate_rejects_low_curve() {
        let settings = GraduationSettings::default();
        let engine = GraduationEngine::new(settings, None);

        let token = PumpFunToken {
            mint: "test".to_string(),
            symbol: "TEST".to_string(),
            bonding_curve_progress: Some(50.0),
            market_cap_sol: Some(100.0),
            v_sol_in_bonding_curve: None,
            reply_count: None,
            usd_market_cap: None,
            price_in_sol: Some(0.0001),
            holder_count: Some(100),
            volume_sol_24h: Some(20.0),
            complete: false,
            raydium_pool: None,
        };
        assert!(!engine.evaluate_candidate(&token));
    }
}
