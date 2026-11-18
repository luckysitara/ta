use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

use crate::solana::execution::OnChainExecutor;
use crate::solana::sniper::PortfolioState;
use crate::solana::SOL_MINT;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSpikeSettings {
    pub total_budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_concurrent_positions: usize,
    pub jito_tip_sol: f64,
    pub min_liquidity_usd: f64,
    pub min_token_age_hours: f64,
    pub volume_spike_multiplier: f64,
    pub min_volume_5m_usd: f64,
    pub min_buy_ratio_5m: f64,
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub trailing_stop_pct: f64,
    pub break_even_mult: f64,
    pub poll_interval_secs: u64,
    pub is_paper_trading: bool,
}

impl Default for VolumeSpikeSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.1,
            buy_amount_sol: 0.02,
            max_concurrent_positions: 3,
            jito_tip_sol: 0.0001,
            min_liquidity_usd: 50_000.0,
            min_token_age_hours: 24.0,
            volume_spike_multiplier: 3.0,
            min_volume_5m_usd: 5_000.0,
            min_buy_ratio_5m: 0.55,
            stop_loss_pct: -5.0,
            take_profit_pct: 15.0,
            trailing_stop_pct: -8.0,
            break_even_mult: 1.05,
            poll_interval_secs: 10,
            is_paper_trading: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSpikePosition {
    pub token_address: String,
    pub symbol: String,
    pub entry_price_sol: f64,
    pub entry_amount_sol: f64,
    pub tokens_held: f64,
    pub entry_time: DateTime<Utc>,
    pub current_price_sol: f64,
    pub peak_price_sol: f64,
    pub peak_multiplier: f64,
    pub current_multiplier: f64,
    pub volume_at_entry_5m: f64,
    pub baseline_volume_5m: f64,
    pub spike_ratio: f64,
    pub trailing_active: bool,
    pub break_even_locked: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct DexScreenerResponse {
    pairs: Option<Vec<DexScreenerPair>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexScreenerPair {
    base_token: DexToken,
    pair_address: String,
    #[serde(default)]
    dex_id: String,
    price_native: Option<String>,
    price_usd: Option<String>,
    liquidity: Option<DexLiquidity>,
    volume: Option<DexVolume>,
    txns: Option<DexTxns>,
    price_change: Option<DexPriceChange>,
    pair_created_at: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct DexToken {
    address: String,
    symbol: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DexLiquidity {
    usd: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct DexVolume {
    h24: Option<f64>,
    h1: Option<f64>,
    m5: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct DexTxns {
    m5: Option<DexTxnCount>,
    h1: Option<DexTxnCount>,
}

#[derive(Debug, Clone, Deserialize)]
struct DexTxnCount {
    buys: Option<usize>,
    sells: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct DexPriceChange {
    m5: Option<f64>,
    h1: Option<f64>,
    h24: Option<f64>,
}

pub struct VolumeSpikeEngine {
    settings: VolumeSpikeSettings,
    executor: Option<OnChainExecutor>,
    positions: Arc<Mutex<HashMap<String, VolumeSpikePosition>>>,
    entered_tokens: Arc<Mutex<HashSet<String>>>,
    portfolio: Arc<Mutex<PortfolioState>>,
    http_client: reqwest::Client,
    volume_history: Arc<Mutex<HashMap<String, VecDeque<(DateTime<Utc>, f64)>>>>,
}

impl VolumeSpikeEngine {
    pub fn new(settings: VolumeSpikeSettings, executor: Option<OnChainExecutor>) -> Self {
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
            volume_history: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get_portfolio_state(&self) -> PortfolioState {
        self.portfolio.lock().unwrap().clone()
    }

    pub fn restore_state(&self, portfolio: PortfolioState, positions: HashMap<String, VolumeSpikePosition>) {
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
        if let Err(e) = ledger.save_generic_state("volume_spike", &*port, &*positions) {
            warn!("Failed to save volume_spike ledger: {}", e);
        }
    }

    async fn fetch_trending_tokens(&self) -> Result<Vec<DexScreenerPair>> {
        let url = "https://api.dexscreener.com/token-boosts/top/v1";
        let resp = self.http_client.get(url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("DexScreener boosts API returned {}", resp.status());
        }

        #[derive(Deserialize)]
        struct BoostEntry {
            #[serde(rename = "tokenAddress")]
            token_address: String,
            #[serde(rename = "chainId")]
            chain_id: String,
        }

        let boosts: Vec<BoostEntry> = resp.json().await.unwrap_or_default();
        let solana_tokens: Vec<&str> = boosts
            .iter()
            .filter(|b| b.chain_id == "solana")
            .map(|b| b.token_address.as_str())
            .take(30)
            .collect();

        if solana_tokens.is_empty() {
            return Ok(vec![]);
        }

        let mut all_pairs = Vec::new();
        for chunk in solana_tokens.chunks(10) {
            let addresses = chunk.join(",");
            let url = format!("https://api.dexscreener.com/tokens/v1/solana/{}", addresses);
            let resp = self.http_client.get(&url).send().await?;
            if resp.status().is_success() {
                if let Ok(pairs) = resp.json::<Vec<DexScreenerPair>>().await {
                    all_pairs.extend(pairs);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        Ok(all_pairs)
    }

    fn calculate_baseline_volume(&self, token: &str) -> f64 {
        let history = self.volume_history.lock().unwrap();
        if let Some(entries) = history.get(token) {
            if entries.len() >= 3 {
                let sum: f64 = entries.iter().map(|(_, v)| v).sum();
                return sum / entries.len() as f64;
            }
        }
        0.0
    }

    fn record_volume(&self, token: &str, volume_5m: f64) {
        let mut history = self.volume_history.lock().unwrap();
        let entries = history.entry(token.to_string()).or_insert_with(VecDeque::new);
        entries.push_back((Utc::now(), volume_5m));
        while entries.len() > 30 {
            entries.pop_front();
        }
    }

    fn evaluate_spike(&self, pair: &DexScreenerPair) -> Option<(f64, f64, f64)> {
        let liquidity_usd = pair.liquidity.as_ref()?.usd?;
        if liquidity_usd < self.settings.min_liquidity_usd {
            return None;
        }

        let volume_5m = pair.volume.as_ref()?.m5?;
        if volume_5m < self.settings.min_volume_5m_usd {
            return None;
        }

        if let Some(created_at) = pair.pair_created_at {
            let age_hours = (Utc::now().timestamp_millis() as u64).saturating_sub(created_at) as f64 / 3_600_000.0;
            if age_hours < self.settings.min_token_age_hours {
                return None;
            }
        }

        if let Some(txns) = &pair.txns {
            if let Some(m5) = &txns.m5 {
                let buys = m5.buys.unwrap_or(0) as f64;
                let total = buys + m5.sells.unwrap_or(0) as f64;
                if total > 0.0 && (buys / total) < self.settings.min_buy_ratio_5m {
                    return None;
                }
            }
        }

        let baseline = self.calculate_baseline_volume(&pair.base_token.address);
        let spike_ratio = if baseline > 0.0 {
            volume_5m / baseline
        } else {
            let volume_1h = pair.volume.as_ref()?.h1.unwrap_or(0.0);
            if volume_1h > 0.0 {
                let avg_5m_from_1h = volume_1h / 12.0;
                if avg_5m_from_1h > 0.0 { volume_5m / avg_5m_from_1h } else { 0.0 }
            } else {
                0.0
            }
        };

        if spike_ratio >= self.settings.volume_spike_multiplier {
            let price_sol: f64 = pair.price_native.as_ref()?.parse().ok()?;
            Some((spike_ratio, volume_5m, price_sol))
        } else {
            None
        }
    }

    pub async fn run_scan_cycle(&self) -> Result<usize> {
        let pairs = self.fetch_trending_tokens().await?;
        let mut entries = 0;

        for pair in &pairs {
            let volume_5m = pair.volume.as_ref().and_then(|v| v.m5).unwrap_or(0.0);
            self.record_volume(&pair.base_token.address, volume_5m);
        }

        for pair in &pairs {
            let token_addr = &pair.base_token.address;

            {
                let entered = self.entered_tokens.lock().unwrap();
                if entered.contains(token_addr) {
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

            if let Some((spike_ratio, volume_5m, price_sol)) = self.evaluate_spike(pair) {
                let symbol = pair.base_token.symbol.clone().unwrap_or_else(|| "???".to_string());
                let tokens = if price_sol > 0.0 {
                    self.settings.buy_amount_sol / price_sol
                } else {
                    continue;
                };

                if !self.settings.is_paper_trading {
                    if let Some(ref executor) = self.executor {
                        match executor
                            .execute_jupiter_swap_jito(
                                &SOL_MINT,
                                token_addr,
                                (self.settings.buy_amount_sol * 1_000_000_000.0) as u64,
                                self.settings.jito_tip_sol,
                                500,
                            )
                            .await
                        {
                            Ok(result) => {
                                info!("[VOLUME_SPIKE] On-chain buy submitted: {}", result.bundle_id);
                            }
                            Err(e) => {
                                warn!("[VOLUME_SPIKE] On-chain buy failed for {}: {}", symbol, e);
                                continue;
                            }
                        }
                    }
                }

                let position = VolumeSpikePosition {
                    token_address: token_addr.clone(),
                    symbol: symbol.clone(),
                    entry_price_sol: price_sol,
                    entry_amount_sol: self.settings.buy_amount_sol,
                    tokens_held: tokens,
                    entry_time: Utc::now(),
                    current_price_sol: price_sol,
                    peak_price_sol: price_sol,
                    peak_multiplier: 1.0,
                    current_multiplier: 1.0,
                    volume_at_entry_5m: volume_5m,
                    baseline_volume_5m: self.calculate_baseline_volume(token_addr),
                    spike_ratio,
                    trailing_active: false,
                    break_even_locked: false,
                };

                let mode = if self.settings.is_paper_trading { "PAPER" } else { "LIVE" };
                println!(
                    "  {} [VOLUME SPIKE {}] {} | spike: {:.1}x | vol_5m: ${:.0} | price: {:.9} SOL | cost: {:.4} SOL",
                    "🔊".green(),
                    mode,
                    symbol.bold().cyan(),
                    spike_ratio,
                    volume_5m,
                    price_sol,
                    self.settings.buy_amount_sol,
                );

                {
                    let mut port = self.portfolio.lock().unwrap();
                    port.available_sol -= self.settings.buy_amount_sol + self.settings.jito_tip_sol;
                    port.invested_sol += self.settings.buy_amount_sol;
                    port.total_trades += 1;
                }

                {
                    let mut positions = self.positions.lock().unwrap();
                    positions.insert(token_addr.clone(), position);
                }
                {
                    let mut entered = self.entered_tokens.lock().unwrap();
                    entered.insert(token_addr.clone());
                }

                entries += 1;
            }
        }

        self.update_and_manage_positions().await?;

        Ok(entries)
    }

    async fn update_and_manage_positions(&self) -> Result<()> {
        let position_keys: Vec<String> = {
            let pos = self.positions.lock().unwrap();
            pos.keys().cloned().collect()
        };

        if position_keys.is_empty() {
            return Ok(());
        }

        let addresses = position_keys.join(",");
        let url = format!("https://api.dexscreener.com/tokens/v1/solana/{}", addresses);
        let resp = self.http_client.get(&url).send().await;

        let pairs: Vec<DexScreenerPair> = match resp {
            Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
            _ => return Ok(()),
        };

        let mut price_map: HashMap<String, f64> = HashMap::new();
        for pair in &pairs {
            if let Some(price_str) = &pair.price_native {
                if let Ok(price) = price_str.parse::<f64>() {
                    let addr = &pair.base_token.address;
                    let existing = price_map.get(addr).copied().unwrap_or(0.0);
                    if price > existing {
                        price_map.insert(addr.clone(), price);
                    }
                }
            }
        }

        let mut to_close: Vec<(String, String)> = Vec::new();

        {
            let mut positions = self.positions.lock().unwrap();
            for (addr, pos) in positions.iter_mut() {
                if let Some(&current_price) = price_map.get(addr) {
                    pos.current_price_sol = current_price;
                    pos.current_multiplier = current_price / pos.entry_price_sol;

                    if current_price > pos.peak_price_sol {
                        pos.peak_price_sol = current_price;
                        pos.peak_multiplier = current_price / pos.entry_price_sol;
                    }

                    if pos.current_multiplier >= self.settings.break_even_mult as f64 {
                        pos.break_even_locked = true;
                    }

                    let tp_activation = 1.0 + (self.settings.take_profit_pct / 100.0);
                    if pos.peak_multiplier >= tp_activation {
                        pos.trailing_active = true;
                    }

                    let pnl_pct = (pos.current_multiplier - 1.0) * 100.0;

                    // Stop loss
                    if pnl_pct <= self.settings.stop_loss_pct {
                        to_close.push((addr.clone(), "STOP_LOSS".to_string()));
                        continue;
                    }

                    // Break-even floor
                    if pos.break_even_locked && pos.current_multiplier < 1.005 {
                        to_close.push((addr.clone(), "BREAK_EVEN".to_string()));
                        continue;
                    }

                    // Trailing stop from peak
                    if pos.trailing_active {
                        let drawdown_from_peak = (pos.current_multiplier / pos.peak_multiplier - 1.0) * 100.0;
                        if drawdown_from_peak <= self.settings.trailing_stop_pct {
                            to_close.push((addr.clone(), format!("TRAILING_TP peak={:.2}x", pos.peak_multiplier)));
                            continue;
                        }
                    }
                }
            }
        }

        for (addr, reason) in to_close {
            self.close_position(&addr, &reason).await;
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

        let pnl_display = if pnl >= 0.0 {
            format!("+{:.4}", pnl).green()
        } else {
            format!("{:.4}", pnl).red()
        };

        println!(
            "  {} [VOLUME_SPIKE EXIT] {} | {} | {:.2}x (peak {:.2}x) | PnL: {} SOL",
            if pnl >= 0.0 { "✅" } else { "🛑" },
            pos.symbol.bold(),
            reason.yellow(),
            pos.current_multiplier,
            pos.peak_multiplier,
            pnl_display,
        );

        let mut port = self.portfolio.lock().unwrap();
        port.available_sol += proceeds;
        port.invested_sol -= pos.entry_amount_sol;
        port.realized_pnl_sol += pnl;
        if pnl >= 0.0 {
            port.winning_trades += 1;
        } else {
            port.losing_trades += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_spike_defaults() {
        let settings = VolumeSpikeSettings::default();
        assert_eq!(settings.volume_spike_multiplier, 3.0);
        assert_eq!(settings.min_token_age_hours, 24.0);
        assert!(settings.is_paper_trading);
    }

    #[test]
    fn test_volume_spike_engine_creation() {
        let settings = VolumeSpikeSettings::default();
        let engine = VolumeSpikeEngine::new(settings, None);
        let port = engine.get_portfolio_state();
        assert_eq!(port.initial_budget_sol, 0.1);
        assert_eq!(port.available_sol, 0.1);
    }
}
