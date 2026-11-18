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
pub struct CopyTradeSettings {
    pub total_budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_concurrent_positions: usize,
    pub jito_tip_sol: f64,
    pub tracked_wallets: Vec<String>,
    pub min_wallet_convergence: usize,
    pub convergence_window_secs: u64,
    pub min_buy_sol: f64,
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub trailing_stop_pct: f64,
    pub break_even_mult: f64,
    pub poll_interval_secs: u64,
    pub is_paper_trading: bool,
    pub rpc_url: Option<String>,
}

impl Default for CopyTradeSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.2,
            buy_amount_sol: 0.03,
            max_concurrent_positions: 5,
            jito_tip_sol: 0.0001,
            tracked_wallets: Vec::new(),
            min_wallet_convergence: 2,
            convergence_window_secs: 300,
            min_buy_sol: 0.5,
            stop_loss_pct: -8.0,
            take_profit_pct: 20.0,
            trailing_stop_pct: -10.0,
            break_even_mult: 1.08,
            poll_interval_secs: 15,
            is_paper_trading: true,
            rpc_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyTradePosition {
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
    pub trigger_wallets: Vec<String>,
    pub trailing_active: bool,
    pub break_even_locked: bool,
}

#[derive(Debug, Clone)]
struct WalletBuy {
    wallet: String,
    token_mint: String,
    amount_sol: f64,
    timestamp: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct HeliusTransaction {
    #[serde(default)]
    description: String,
    #[serde(rename = "type", default)]
    tx_type: String,
    #[serde(default)]
    source: String,
    #[serde(rename = "tokenTransfers", default)]
    token_transfers: Vec<HeliusTokenTransfer>,
    #[serde(rename = "nativeTransfers", default)]
    native_transfers: Vec<HeliusNativeTransfer>,
    timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct HeliusTokenTransfer {
    #[serde(rename = "fromUserAccount", default)]
    from_user_account: String,
    #[serde(rename = "toUserAccount", default)]
    to_user_account: String,
    mint: String,
    #[serde(rename = "tokenAmount", default)]
    token_amount: f64,
}

#[derive(Debug, Deserialize)]
struct HeliusNativeTransfer {
    #[serde(rename = "fromUserAccount", default)]
    from_user_account: String,
    amount: u64,
}

pub struct CopyTradeEngine {
    settings: CopyTradeSettings,
    executor: Option<OnChainExecutor>,
    positions: Arc<Mutex<HashMap<String, CopyTradePosition>>>,
    entered_tokens: Arc<Mutex<HashSet<String>>>,
    portfolio: Arc<Mutex<PortfolioState>>,
    http_client: reqwest::Client,
    recent_buys: Arc<Mutex<Vec<WalletBuy>>>,
}

impl CopyTradeEngine {
    pub fn new(settings: CopyTradeSettings, executor: Option<OnChainExecutor>) -> Self {
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
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            recent_buys: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn get_portfolio_state(&self) -> PortfolioState {
        self.portfolio.lock().unwrap().clone()
    }

    pub fn restore_state(&self, portfolio: PortfolioState, positions: HashMap<String, CopyTradePosition>) {
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
        if let Err(e) = ledger.save_generic_state("copy_trade", &*port, &*positions) {
            warn!("Failed to save copy_trade ledger: {}", e);
        }
    }

    async fn fetch_wallet_transactions(&self, wallet: &str) -> Result<Vec<HeliusTransaction>> {
        let api_key = std::env::var("HELIUS_API_KEY").unwrap_or_default();
        let url = if !api_key.is_empty() {
            format!(
                "https://api.helius.xyz/v0/addresses/{}/transactions?api-key={}&limit=10&type=SWAP",
                wallet, api_key
            )
        } else {
            let rpc = self.settings.rpc_url.as_deref().unwrap_or("https://api.mainnet-beta.solana.com");
            format!(
                "{}/v0/addresses/{}/transactions?limit=10",
                rpc.trim_end_matches('/'), wallet
            )
        };

        let resp = self.http_client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Helius API returned {}", resp.status());
        }

        let txns: Vec<HeliusTransaction> = resp.json().await.unwrap_or_default();
        Ok(txns)
    }

    fn extract_buys_from_transactions(&self, wallet: &str, txns: &[HeliusTransaction]) -> Vec<WalletBuy> {
        let now = Utc::now();
        let window = chrono::Duration::seconds(self.settings.convergence_window_secs as i64);
        let cutoff = now - window;

        let mut buys = Vec::new();
        for tx in txns {
            if tx.tx_type != "SWAP" && !tx.description.to_lowercase().contains("swap") {
                continue;
            }

            let ts = tx.timestamp.map(|t| {
                DateTime::from_timestamp(t as i64, 0).unwrap_or(now)
            }).unwrap_or(now);

            if ts < cutoff {
                continue;
            }

            let sol_spent: f64 = tx.native_transfers.iter()
                .filter(|nt| nt.from_user_account == wallet)
                .map(|nt| nt.amount as f64 / 1_000_000_000.0)
                .sum();

            if sol_spent < self.settings.min_buy_sol {
                continue;
            }

            for transfer in &tx.token_transfers {
                if transfer.to_user_account == wallet
                    && transfer.mint != SOL_MINT
                    && transfer.token_amount > 0.0
                {
                    buys.push(WalletBuy {
                        wallet: wallet.to_string(),
                        token_mint: transfer.mint.clone(),
                        amount_sol: sol_spent,
                        timestamp: ts,
                    });
                }
            }
        }

        buys
    }

    fn detect_convergence(&self) -> Vec<(String, Vec<String>)> {
        let buys = self.recent_buys.lock().unwrap();
        let mut token_wallets: HashMap<String, HashSet<String>> = HashMap::new();

        for buy in buys.iter() {
            token_wallets
                .entry(buy.token_mint.clone())
                .or_default()
                .insert(buy.wallet.clone());
        }

        token_wallets
            .into_iter()
            .filter(|(_, wallets)| wallets.len() >= self.settings.min_wallet_convergence)
            .map(|(token, wallets)| (token, wallets.into_iter().collect()))
            .collect()
    }

    async fn get_token_price(&self, token_addr: &str) -> Option<f64> {
        let url = format!("https://api.dexscreener.com/tokens/v1/solana/{}", token_addr);
        let resp = self.http_client.get(&url).send().await.ok()?;

        #[derive(Deserialize)]
        struct Pair {
            #[serde(rename = "priceNative")]
            price_native: Option<String>,
        }

        let pairs: Vec<Pair> = resp.json().await.ok()?;
        pairs.first()?.price_native.as_ref()?.parse().ok()
    }

    pub async fn run_scan_cycle(&self) -> Result<usize> {
        let now = Utc::now();
        let window = chrono::Duration::seconds(self.settings.convergence_window_secs as i64);
        let cutoff = now - window;

        {
            let mut buys = self.recent_buys.lock().unwrap();
            buys.retain(|b| b.timestamp >= cutoff);
        }

        for wallet in &self.settings.tracked_wallets {
            match self.fetch_wallet_transactions(wallet).await {
                Ok(txns) => {
                    let wallet_buys = self.extract_buys_from_transactions(wallet, &txns);
                    let mut all_buys = self.recent_buys.lock().unwrap();
                    for buy in wallet_buys {
                        if !all_buys.iter().any(|b| b.wallet == buy.wallet && b.token_mint == buy.token_mint) {
                            all_buys.push(buy);
                        }
                    }
                }
                Err(e) => {
                    warn!("[COPY_TRADE] Failed to fetch txns for {}: {}", &wallet[..8.min(wallet.len())], e);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        let convergences = self.detect_convergence();
        let mut entries = 0;

        for (token_mint, wallets) in convergences {
            {
                let entered = self.entered_tokens.lock().unwrap();
                if entered.contains(&token_mint) {
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

            let price_sol = match self.get_token_price(&token_mint).await {
                Some(p) if p > 0.0 => p,
                _ => continue,
            };

            let tokens = self.settings.buy_amount_sol / price_sol;

            if !self.settings.is_paper_trading {
                if let Some(ref executor) = self.executor {
                    match executor
                        .execute_jupiter_swap_jito(
                            &SOL_MINT,
                            &token_mint,
                            (self.settings.buy_amount_sol * 1_000_000_000.0) as u64,
                            self.settings.jito_tip_sol,
                            500,
                        )
                        .await
                    {
                        Ok(result) => info!("[COPY_TRADE] On-chain buy: {}", result.bundle_id),
                        Err(e) => {
                            warn!("[COPY_TRADE] Buy failed for {}: {}", &token_mint[..8], e);
                            continue;
                        }
                    }
                }
            }

            let mint_short = &token_mint[..token_mint.len().min(12)];
            let mode = if self.settings.is_paper_trading { "PAPER" } else { "LIVE" };
            println!(
                "  {} [COPY TRADE {}] {}... | {} smart wallets converged | price: {:.9} SOL | cost: {:.4} SOL",
                "🧠".green(),
                mode,
                mint_short.bold().cyan(),
                wallets.len(),
                price_sol,
                self.settings.buy_amount_sol,
            );

            let position = CopyTradePosition {
                token_address: token_mint.clone(),
                symbol: mint_short.to_string(),
                entry_price_sol: price_sol,
                entry_amount_sol: self.settings.buy_amount_sol,
                tokens_held: tokens,
                entry_time: Utc::now(),
                current_price_sol: price_sol,
                peak_price_sol: price_sol,
                peak_multiplier: 1.0,
                current_multiplier: 1.0,
                trigger_wallets: wallets,
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
                positions.insert(token_mint.clone(), position);
            }
            {
                let mut entered = self.entered_tokens.lock().unwrap();
                entered.insert(token_mint);
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

        for chunk in position_keys.chunks(10) {
            let addresses = chunk.join(",");
            let url = format!("https://api.dexscreener.com/tokens/v1/solana/{}", addresses);
            let resp = self.http_client.get(&url).send().await;

            #[derive(Deserialize)]
            struct SimplePair {
                #[serde(rename = "baseToken")]
                base_token: SimpleToken,
                #[serde(rename = "priceNative")]
                price_native: Option<String>,
            }
            #[derive(Deserialize)]
            struct SimpleToken {
                address: String,
            }

            let pairs: Vec<SimplePair> = match resp {
                Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
                _ => continue,
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
                            continue;
                        }

                        if pos.break_even_locked && pos.current_multiplier < 1.005 {
                            to_close.push((addr.clone(), "BREAK_EVEN".to_string()));
                            continue;
                        }

                        if pos.trailing_active {
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

            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
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
            "  {} [COPY_TRADE EXIT] {} | {} | {:.2}x | PnL: {} SOL | wallets: {:?}",
            if pnl >= 0.0 { "✅" } else { "🛑" },
            pos.symbol.bold(),
            reason.yellow(),
            pos.current_multiplier,
            pnl_display,
            pos.trigger_wallets.len(),
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
    fn test_copy_trade_defaults() {
        let s = CopyTradeSettings::default();
        assert_eq!(s.min_wallet_convergence, 2);
        assert_eq!(s.convergence_window_secs, 300);
    }

    #[test]
    fn test_convergence_detection() {
        let settings = CopyTradeSettings::default();
        let engine = CopyTradeEngine::new(settings, None);

        {
            let mut buys = engine.recent_buys.lock().unwrap();
            buys.push(WalletBuy {
                wallet: "wallet_a".to_string(),
                token_mint: "token_x".to_string(),
                amount_sol: 1.0,
                timestamp: Utc::now(),
            });
            buys.push(WalletBuy {
                wallet: "wallet_b".to_string(),
                token_mint: "token_x".to_string(),
                amount_sol: 2.0,
                timestamp: Utc::now(),
            });
            buys.push(WalletBuy {
                wallet: "wallet_c".to_string(),
                token_mint: "token_y".to_string(),
                amount_sol: 1.0,
                timestamp: Utc::now(),
            });
        }

        let convergences = engine.detect_convergence();
        assert_eq!(convergences.len(), 1);
        assert_eq!(convergences[0].0, "token_x");
        assert_eq!(convergences[0].1.len(), 2);
    }
}
