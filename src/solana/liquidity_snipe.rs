use anyhow::Result;
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use crate::solana::execution::OnChainExecutor;
use crate::solana::sniper::PortfolioState;
use crate::solana::SOL_MINT;

pub const METEORA_DLMM_PROGRAM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
pub const ORCA_WHIRLPOOL_PROGRAM: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
pub const METEORA_POOLS_PROGRAM: &str = "Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquiditySnipeSettings {
    pub total_budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_concurrent_positions: usize,
    pub jito_tip_sol: f64,
    pub min_liquidity_usd: f64,
    pub max_pool_age_mins: f64,
    pub min_initial_liquidity_sol: f64,
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub trailing_stop_pct: f64,
    pub break_even_mult: f64,
    pub poll_interval_secs: u64,
    pub is_paper_trading: bool,
    pub target_dexes: Vec<String>,
}

impl Default for LiquiditySnipeSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.1,
            buy_amount_sol: 0.02,
            max_concurrent_positions: 3,
            jito_tip_sol: 0.0003,
            min_liquidity_usd: 5_000.0,
            max_pool_age_mins: 30.0,
            min_initial_liquidity_sol: 2.0,
            stop_loss_pct: -10.0,
            take_profit_pct: 25.0,
            trailing_stop_pct: -12.0,
            break_even_mult: 1.08,
            poll_interval_secs: 10,
            is_paper_trading: true,
            target_dexes: vec!["meteora".to_string(), "orca".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquiditySnipePosition {
    pub token_address: String,
    pub pair_address: String,
    pub symbol: String,
    pub dex: String,
    pub entry_price_sol: f64,
    pub entry_amount_sol: f64,
    pub tokens_held: f64,
    pub entry_time: DateTime<Utc>,
    pub pool_age_at_entry_mins: f64,
    pub initial_liquidity_usd: f64,
    pub current_price_sol: f64,
    pub peak_price_sol: f64,
    pub peak_multiplier: f64,
    pub current_multiplier: f64,
    pub trailing_active: bool,
    pub break_even_locked: bool,
}

#[derive(Debug, Deserialize)]
struct DexScreenerPair {
    #[serde(rename = "baseToken")]
    base_token: DexToken,
    #[serde(rename = "pairAddress", default)]
    pair_address: String,
    #[serde(rename = "dexId", default)]
    dex_id: String,
    #[serde(rename = "priceNative")]
    price_native: Option<String>,
    liquidity: Option<DexLiquidity>,
    volume: Option<DexVolume>,
    txns: Option<DexTxns>,
    #[serde(rename = "pairCreatedAt")]
    pair_created_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DexToken {
    address: String,
    symbol: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DexLiquidity {
    usd: Option<f64>,
    base: Option<f64>,
    quote: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexVolume {
    m5: Option<f64>,
    h1: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexTxns {
    m5: Option<DexTxnCount>,
}

#[derive(Debug, Deserialize)]
struct DexTxnCount {
    buys: Option<usize>,
    sells: Option<usize>,
}

pub struct LiquiditySnipeEngine {
    settings: LiquiditySnipeSettings,
    executor: Option<OnChainExecutor>,
    positions: Arc<Mutex<HashMap<String, LiquiditySnipePosition>>>,
    entered_tokens: Arc<Mutex<HashSet<String>>>,
    portfolio: Arc<Mutex<PortfolioState>>,
    http_client: reqwest::Client,
}

impl LiquiditySnipeEngine {
    pub fn new(settings: LiquiditySnipeSettings, executor: Option<OnChainExecutor>) -> Self {
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

    pub fn restore_state(&self, portfolio: PortfolioState, positions: HashMap<String, LiquiditySnipePosition>) {
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
        if let Err(e) = ledger.save_generic_state("liquidity_snipe", &*port, &*positions) {
            warn!("Failed to save liquidity_snipe ledger: {}", e);
        }
    }

    async fn fetch_new_pools(&self) -> Result<Vec<DexScreenerPair>> {
        let mut all_pairs = Vec::new();

        for dex in &self.settings.target_dexes {
            let url = format!(
                "https://api.dexscreener.com/latest/dex/pairs/solana?sort=pairAge&order=asc"
            );

            // Use the token-profiles endpoint to find new Solana pairs
            let search_url = format!(
                "https://api.dexscreener.com/token-profiles/latest/v1"
            );

            let resp = self.http_client.get(&search_url).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    #[derive(Deserialize)]
                    struct TokenProfile {
                        #[serde(rename = "tokenAddress")]
                        token_address: String,
                        #[serde(rename = "chainId")]
                        chain_id: String,
                    }

                    let profiles: Vec<TokenProfile> = r.json().await.unwrap_or_default();
                    let solana_tokens: Vec<&str> = profiles
                        .iter()
                        .filter(|p| p.chain_id == "solana")
                        .map(|p| p.token_address.as_str())
                        .take(20)
                        .collect();

                    if !solana_tokens.is_empty() {
                        for chunk in solana_tokens.chunks(10) {
                            let addrs = chunk.join(",");
                            let pair_url = format!("https://api.dexscreener.com/tokens/v1/solana/{}", addrs);
                            if let Ok(pair_resp) = self.http_client.get(&pair_url).send().await {
                                if pair_resp.status().is_success() {
                                    let pairs: Vec<DexScreenerPair> = pair_resp.json().await.unwrap_or_default();
                                    let filtered: Vec<DexScreenerPair> = pairs
                                        .into_iter()
                                        .filter(|p| {
                                            let dex_match = self.settings.target_dexes.iter().any(|d| {
                                                p.dex_id.to_lowercase().contains(&d.to_lowercase())
                                            });
                                            dex_match
                                        })
                                        .collect();
                                    all_pairs.extend(filtered);
                                }
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        }
                    }
                }
                _ => {}
            }

            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        Ok(all_pairs)
    }

    fn evaluate_new_pool(&self, pair: &DexScreenerPair) -> bool {
        let liquidity_usd = pair.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
        if liquidity_usd < self.settings.min_liquidity_usd {
            return false;
        }

        if let Some(created_at) = pair.pair_created_at {
            let now_ms = Utc::now().timestamp_millis() as u64;
            let age_mins = now_ms.saturating_sub(created_at) as f64 / 60_000.0;
            if age_mins > self.settings.max_pool_age_mins {
                return false;
            }
        } else {
            return false;
        }

        if let Some(txns) = &pair.txns {
            if let Some(m5) = &txns.m5 {
                let buys = m5.buys.unwrap_or(0);
                if buys < 3 {
                    return false;
                }
            }
        }

        true
    }

    pub async fn run_scan_cycle(&self) -> Result<usize> {
        let pairs = self.fetch_new_pools().await?;
        let mut entries = 0;

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

            if !self.evaluate_new_pool(pair) {
                continue;
            }

            let price_sol: f64 = match pair.price_native.as_ref().and_then(|p| p.parse().ok()) {
                Some(p) if p > 0.0 => p,
                _ => continue,
            };

            let liquidity_usd = pair.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
            let pool_age_mins = pair.pair_created_at
                .map(|ca| (Utc::now().timestamp_millis() as u64).saturating_sub(ca) as f64 / 60_000.0)
                .unwrap_or(0.0);
            let symbol = pair.base_token.symbol.clone().unwrap_or_else(|| "???".to_string());
            let tokens = self.settings.buy_amount_sol / price_sol;

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
                        Ok(result) => info!("[LIQ_SNIPE] On-chain buy: {}", result.bundle_id),
                        Err(e) => {
                            warn!("[LIQ_SNIPE] Buy failed for {}: {}", symbol, e);
                            continue;
                        }
                    }
                }
            }

            let mode = if self.settings.is_paper_trading { "PAPER" } else { "LIVE" };
            println!(
                "  {} [LIQ SNIPE {} {}] {} on {} | age: {:.1}m | liq: ${:.0} | price: {:.9} SOL | cost: {:.4} SOL",
                "💧".green(),
                pair.dex_id.to_uppercase(),
                mode,
                symbol.bold().cyan(),
                pair.dex_id,
                pool_age_mins,
                liquidity_usd,
                price_sol,
                self.settings.buy_amount_sol,
            );

            let position = LiquiditySnipePosition {
                token_address: token_addr.clone(),
                pair_address: pair.pair_address.clone(),
                symbol,
                dex: pair.dex_id.clone(),
                entry_price_sol: price_sol,
                entry_amount_sol: self.settings.buy_amount_sol,
                tokens_held: tokens,
                entry_time: Utc::now(),
                pool_age_at_entry_mins: pool_age_mins,
                initial_liquidity_usd: liquidity_usd,
                current_price_sol: price_sol,
                peak_price_sol: price_sol,
                peak_multiplier: 1.0,
                current_multiplier: 1.0,
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
                positions.insert(token_addr.clone(), position);
            }
            {
                let mut entered = self.entered_tokens.lock().unwrap();
                entered.insert(token_addr.clone());
            }

            entries += 1;
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
            if let Some(price) = pair.price_native.as_ref().and_then(|p| p.parse::<f64>().ok()) {
                let existing = price_map.get(&pair.base_token.address).copied().unwrap_or(0.0);
                if price > existing {
                    price_map.insert(pair.base_token.address.clone(), price);
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

                    if pos.current_multiplier >= self.settings.break_even_mult {
                        pos.break_even_locked = true;
                    }

                    let tp_activation = 1.0 + (self.settings.take_profit_pct / 100.0);
                    if pos.peak_multiplier >= tp_activation {
                        pos.trailing_active = true;
                    }

                    let pnl_pct = (pos.current_multiplier - 1.0) * 100.0;

                    if pnl_pct <= self.settings.stop_loss_pct {
                        to_close.push((addr.clone(), "STOP_LOSS".to_string()));
                    } else if pos.break_even_locked && pos.current_multiplier < 1.005 {
                        to_close.push((addr.clone(), "BREAK_EVEN".to_string()));
                    } else if pos.trailing_active {
                        let drawdown = (pos.current_multiplier / pos.peak_multiplier - 1.0) * 100.0;
                        if drawdown <= self.settings.trailing_stop_pct {
                            to_close.push((addr.clone(), format!("TRAILING peak={:.2}x", pos.peak_multiplier)));
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
                        info!("[LIQ_SNIPE] On-chain sell confirmed: {}", res.bundle_id);
                    }
                    Err(e) => {
                        error!("[LIQ_SNIPE] On-chain sell failed for {}: {}", pos.symbol, e);
                    }
                }
            }
        }

        let pnl_display = if pnl >= 0.0 { format!("+{:.4}", pnl).green() } else { format!("{:.4}", pnl).red() };
        println!(
            "  {} [LIQ_SNIPE EXIT] {} ({}) | {} | {:.2}x | PnL: {} SOL",
            if pnl >= 0.0 { "✅" } else { "🛑" },
            pos.symbol.bold(), pos.dex, reason.yellow(),
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
    fn test_liquidity_snipe_defaults() {
        let s = LiquiditySnipeSettings::default();
        assert_eq!(s.target_dexes, vec!["meteora", "orca"]);
        assert_eq!(s.max_pool_age_mins, 30.0);
    }
}
