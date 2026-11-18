use anyhow::Result;
use chrono::{DateTime, Utc};
use colored::*;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{info, warn};

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

#[derive(Debug, Clone)]
pub struct WalletStats {
    pub address: String,
    pub label: String,
    pub total_buys: usize,
    pub total_sells: usize,
    pub total_sol_spent: f64,
    pub total_sol_received: f64,
    pub net_pnl_sol: f64,
    pub unique_tokens_traded: usize,
    pub win_count: usize,
    pub loss_count: usize,
    pub win_rate: f64,
    pub avg_hold_time_secs: f64,
    pub biggest_win_sol: f64,
    pub biggest_loss_sol: f64,
    pub tokens: Vec<TokenPnl>,
    pub tx_count: usize,
    pub first_tx: Option<DateTime<Utc>>,
    pub last_tx: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct TokenPnl {
    pub mint: String,
    pub sol_in: f64,
    pub sol_out: f64,
    pub pnl: f64,
    pub buys: usize,
    pub sells: usize,
}

#[derive(Debug, Deserialize)]
struct HeliusTx {
    #[serde(default)]
    description: String,
    #[serde(rename = "type", default)]
    tx_type: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    signature: String,
    #[serde(rename = "tokenTransfers", default)]
    token_transfers: Vec<TokenTransfer>,
    #[serde(rename = "nativeTransfers", default)]
    native_transfers: Vec<NativeTransfer>,
    #[serde(rename = "accountData", default)]
    account_data: Vec<serde_json::Value>,
    timestamp: Option<i64>,
    #[serde(default)]
    fee: u64,
    #[serde(rename = "feePayer", default)]
    fee_payer: String,
}

#[derive(Debug, Deserialize)]
struct TokenTransfer {
    #[serde(rename = "fromUserAccount", default)]
    from_user_account: String,
    #[serde(rename = "toUserAccount", default)]
    to_user_account: String,
    mint: String,
    #[serde(rename = "tokenAmount", default)]
    token_amount: f64,
}

#[derive(Debug, Deserialize)]
struct NativeTransfer {
    #[serde(rename = "fromUserAccount", default)]
    from_user_account: String,
    #[serde(rename = "toUserAccount", default)]
    to_user_account: String,
    amount: u64,
}

pub struct WalletAnalyzer {
    http_client: reqwest::Client,
    helius_api_key: String,
    rpc_url: String,
    max_transactions: usize,
}

impl WalletAnalyzer {
    pub fn new(rpc_url: Option<String>, max_transactions: usize) -> Self {
        let helius_api_key = std::env::var("HELIUS_API_KEY").unwrap_or_default();
        let rpc_url = rpc_url.unwrap_or_else(|| {
            std::env::var("SOLANA_RPC_URL")
                .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string())
        });

        Self {
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap(),
            helius_api_key,
            rpc_url,
            max_transactions,
        }
    }

    pub async fn analyze_wallet(&self, address: &str, label: &str) -> Result<WalletStats> {
        info!("[ANALYZER] Fetching transactions for {} ({})", label, &address[..8]);

        let txns = self.fetch_all_transactions(address).await?;
        info!("[ANALYZER] Got {} transactions for {}", txns.len(), label);

        let mut token_map: HashMap<String, TokenPnl> = HashMap::new();
        let mut total_buys = 0usize;
        let mut total_sells = 0usize;
        let mut total_sol_spent = 0.0f64;
        let mut total_sol_received = 0.0f64;
        let mut first_tx: Option<DateTime<Utc>> = None;
        let mut last_tx: Option<DateTime<Utc>> = None;
        let mut tx_count = 0usize;

        for tx in &txns {
            if tx.tx_type != "SWAP" && !tx.description.to_lowercase().contains("swap") {
                continue;
            }

            let ts = tx.timestamp.and_then(|t| DateTime::from_timestamp(t, 0));

            if let Some(t) = ts {
                match first_tx {
                    None => first_tx = Some(t),
                    Some(f) if t < f => first_tx = Some(t),
                    _ => {}
                }
                match last_tx {
                    None => last_tx = Some(t),
                    Some(l) if t > l => last_tx = Some(t),
                    _ => {}
                }
            }

            tx_count += 1;

            let sol_out: f64 = tx.native_transfers.iter()
                .filter(|nt| nt.from_user_account == address)
                .map(|nt| nt.amount as f64 / 1_000_000_000.0)
                .sum();

            let sol_in: f64 = tx.native_transfers.iter()
                .filter(|nt| nt.to_user_account == address)
                .map(|nt| nt.amount as f64 / 1_000_000_000.0)
                .sum();

            let tokens_received: Vec<&TokenTransfer> = tx.token_transfers.iter()
                .filter(|tt| tt.to_user_account == address && tt.mint != SOL_MINT)
                .collect();

            let tokens_sent: Vec<&TokenTransfer> = tx.token_transfers.iter()
                .filter(|tt| tt.from_user_account == address && tt.mint != SOL_MINT)
                .collect();

            let is_buy = sol_out > 0.01 && !tokens_received.is_empty();
            let is_sell = sol_in > 0.01 && !tokens_sent.is_empty();

            if is_buy {
                total_buys += 1;
                let net_sol = sol_out - sol_in;
                total_sol_spent += net_sol;

                for tt in &tokens_received {
                    let entry = token_map.entry(tt.mint.clone()).or_insert(TokenPnl {
                        mint: tt.mint.clone(),
                        sol_in: 0.0,
                        sol_out: 0.0,
                        pnl: 0.0,
                        buys: 0,
                        sells: 0,
                    });
                    entry.sol_in += net_sol;
                    entry.buys += 1;
                }
            }

            if is_sell {
                total_sells += 1;
                let net_sol = sol_in - sol_out;
                total_sol_received += net_sol;

                for tt in &tokens_sent {
                    let entry = token_map.entry(tt.mint.clone()).or_insert(TokenPnl {
                        mint: tt.mint.clone(),
                        sol_in: 0.0,
                        sol_out: 0.0,
                        pnl: 0.0,
                        buys: 0,
                        sells: 0,
                    });
                    entry.sol_out += net_sol;
                    entry.sells += 1;
                }
            }
        }

        for tp in token_map.values_mut() {
            tp.pnl = tp.sol_out - tp.sol_in;
        }

        let mut tokens: Vec<TokenPnl> = token_map.into_values().collect();
        tokens.sort_by(|a, b| b.pnl.partial_cmp(&a.pnl).unwrap_or(std::cmp::Ordering::Equal));

        let win_count = tokens.iter().filter(|t| t.pnl > 0.0).count();
        let loss_count = tokens.iter().filter(|t| t.pnl < 0.0).count();
        let closed_count = tokens.iter().filter(|t| t.sells > 0).count();
        let win_rate = if closed_count > 0 {
            win_count as f64 / closed_count as f64 * 100.0
        } else {
            0.0
        };

        let biggest_win = tokens.iter().map(|t| t.pnl).fold(0.0f64, f64::max);
        let biggest_loss = tokens.iter().map(|t| t.pnl).fold(0.0f64, f64::min);

        let net_pnl_sol = total_sol_received - total_sol_spent;

        let avg_hold = match (first_tx, last_tx) {
            (Some(f), Some(l)) if tx_count > 1 => {
                (l - f).num_seconds() as f64 / tx_count as f64
            }
            _ => 0.0,
        };

        Ok(WalletStats {
            address: address.to_string(),
            label: label.to_string(),
            total_buys,
            total_sells,
            total_sol_spent,
            total_sol_received,
            net_pnl_sol,
            unique_tokens_traded: tokens.len(),
            win_count,
            loss_count,
            win_rate,
            avg_hold_time_secs: avg_hold,
            biggest_win_sol: biggest_win,
            biggest_loss_sol: biggest_loss,
            tokens,
            tx_count,
            first_tx,
            last_tx,
        })
    }

    async fn fetch_all_transactions(&self, address: &str) -> Result<Vec<HeliusTx>> {
        let mut all_txns = Vec::new();
        let page_size = 100;
        let mut last_sig: Option<String> = None;

        loop {
            let txns = self.fetch_page(address, page_size, last_sig.as_deref()).await?;
            let count = txns.len();

            if let Some(last) = txns.last() {
                last_sig = Some(last.signature.clone());
            }

            all_txns.extend(txns);

            if count < page_size || all_txns.len() >= self.max_transactions {
                break;
            }

            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        all_txns.truncate(self.max_transactions);
        Ok(all_txns)
    }

    async fn fetch_page(
        &self,
        address: &str,
        limit: usize,
        before: Option<&str>,
    ) -> Result<Vec<HeliusTx>> {
        let url = if !self.helius_api_key.is_empty() {
            let mut u = format!(
                "https://api.helius.xyz/v0/addresses/{}/transactions?api-key={}&limit={}",
                address, self.helius_api_key, limit
            );
            if let Some(b) = before {
                u.push_str(&format!("&before={}", b));
            }
            u
        } else {
            let mut u = format!(
                "https://api.helius.xyz/v0/addresses/{}/transactions?limit={}",
                address, limit
            );
            if let Some(b) = before {
                u.push_str(&format!("&before={}", b));
            }
            u
        };

        let resp = self.http_client.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Helius API returned {} — {}", status, &body[..body.len().min(200)]);
        }

        let txns: Vec<HeliusTx> = resp.json().await?;
        Ok(txns)
    }

    pub async fn analyze_all(&self, wallets: &[(String, String)]) -> Vec<WalletStats> {
        let mut results = Vec::new();

        for (address, label) in wallets {
            match self.analyze_wallet(address, label).await {
                Ok(stats) => results.push(stats),
                Err(e) => warn!("[ANALYZER] Failed to analyze {} ({}): {}", label, &address[..8], e),
            }

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        results.sort_by(|a, b| b.net_pnl_sol.partial_cmp(&a.net_pnl_sol).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    pub fn print_report(results: &[WalletStats]) {
        println!("\n{}", "================================================================================".cyan());
        println!("{}", "  WALLET ANALYZER — ON-CHAIN PnL REPORT                                         ".bold().cyan());
        println!("{}", "================================================================================".cyan());

        if results.is_empty() {
            println!("  No wallets analyzed.");
            return;
        }

        for (i, w) in results.iter().enumerate() {
            let pnl_color = if w.net_pnl_sol >= 0.0 { "green" } else { "red" };
            let pnl_str = format!("{:+.4} SOL", w.net_pnl_sol);
            let pnl_display = if w.net_pnl_sol >= 0.0 {
                pnl_str.green()
            } else {
                pnl_str.red()
            };
            let _ = pnl_color;

            let roi = if w.total_sol_spent > 0.0 {
                (w.net_pnl_sol / w.total_sol_spent) * 100.0
            } else {
                0.0
            };

            let period = match (w.first_tx, w.last_tx) {
                (Some(f), Some(l)) => {
                    let days = (l - f).num_hours() as f64 / 24.0;
                    format!("{:.1}d", days)
                }
                _ => "N/A".to_string(),
            };

            println!("\n  {}. {} ({}…{})", i + 1,
                w.label.bold(),
                &w.address[..6],
                &w.address[w.address.len().saturating_sub(4)..]);
            println!("  {}", "─".repeat(70));
            println!("     Net PnL:         {}", pnl_display);
            println!("     ROI:             {:+.1}%", roi);
            println!("     Win Rate:        {:.1}% ({} W / {} L)",
                w.win_rate, w.win_count, w.loss_count);
            println!("     Swaps:           {} buys, {} sells ({} total)",
                w.total_buys, w.total_sells, w.tx_count);
            println!("     SOL In/Out:      {:.4} spent → {:.4} received",
                w.total_sol_spent, w.total_sol_received);
            println!("     Tokens Traded:   {}", w.unique_tokens_traded);
            println!("     Biggest Win:     {:+.4} SOL", w.biggest_win_sol);
            println!("     Biggest Loss:    {:+.4} SOL", w.biggest_loss_sol);
            println!("     Period:          {}", period);

            if !w.tokens.is_empty() {
                let top_n = 5.min(w.tokens.len());
                println!("     Top {} tokens:", top_n);
                for t in &w.tokens[..top_n] {
                    let mint_short = if t.mint.len() > 12 {
                        format!("{}…{}", &t.mint[..6], &t.mint[t.mint.len()-4..])
                    } else {
                        t.mint.clone()
                    };
                    let pnl_tok = if t.pnl >= 0.0 {
                        format!("{:+.4}", t.pnl).green()
                    } else {
                        format!("{:+.4}", t.pnl).red()
                    };
                    println!("       {} {} SOL  ({}B/{}S)",
                        mint_short, pnl_tok, t.buys, t.sells);
                }
            }
        }

        println!("\n{}", "================================================================================".cyan());
        println!("{}", "  RANKING SUMMARY".bold().cyan());
        println!("{}", "================================================================================".cyan());
        println!("  {:<4} {:<12} {:>12} {:>8} {:>8} {:>7} {:>6}",
            "#", "Trader", "Net PnL", "ROI", "WinRate", "Trades", "Tokens");
        println!("  {}", "─".repeat(65));

        for (i, w) in results.iter().enumerate() {
            let roi = if w.total_sol_spent > 0.0 {
                (w.net_pnl_sol / w.total_sol_spent) * 100.0
            } else {
                0.0
            };
            println!("  {:<4} {:<12} {:>+12.4} {:>+7.1}% {:>7.1}% {:>7} {:>6}",
                i + 1,
                w.label,
                w.net_pnl_sol,
                roi,
                w.win_rate,
                w.total_buys + w.total_sells,
                w.unique_tokens_traded);
        }

        let total_pnl: f64 = results.iter().map(|w| w.net_pnl_sol).sum();
        let avg_wr: f64 = if !results.is_empty() {
            results.iter().map(|w| w.win_rate).sum::<f64>() / results.len() as f64
        } else {
            0.0
        };
        println!("  {}", "─".repeat(65));
        println!("  Combined PnL: {:+.4} SOL | Avg Win Rate: {:.1}%", total_pnl, avg_wr);

        println!("\n  {} Wallets ranked by on-chain realized PnL from Helius parsed transactions.",
            "ℹ".cyan());
        println!("  {} Only closed positions (buy + sell) contribute to win/loss stats.",
            "ℹ".cyan());
        println!("{}", "================================================================================".cyan());
    }

    pub fn recommend_for_copy_trade(results: &[WalletStats], min_win_rate: f64, min_trades: usize) -> Vec<&WalletStats> {
        let mut candidates: Vec<&WalletStats> = results.iter()
            .filter(|w| w.win_rate >= min_win_rate)
            .filter(|w| (w.total_buys + w.total_sells) >= min_trades)
            .filter(|w| w.net_pnl_sol > 0.0)
            .collect();

        candidates.sort_by(|a, b| {
            let score_a = a.net_pnl_sol * (a.win_rate / 100.0);
            let score_b = b.net_pnl_sol * (b.win_rate / 100.0);
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyzer_creation() {
        let analyzer = WalletAnalyzer::new(None, 200);
        assert_eq!(analyzer.max_transactions, 200);
    }

    #[test]
    fn test_recommend_filters() {
        let stats = vec![
            WalletStats {
                address: "addr1".into(), label: "Good".into(),
                total_buys: 50, total_sells: 30, total_sol_spent: 10.0,
                total_sol_received: 15.0, net_pnl_sol: 5.0,
                unique_tokens_traded: 20, win_count: 12, loss_count: 8,
                win_rate: 60.0, avg_hold_time_secs: 300.0,
                biggest_win_sol: 2.0, biggest_loss_sol: -1.0,
                tokens: vec![], tx_count: 80, first_tx: None, last_tx: None,
            },
            WalletStats {
                address: "addr2".into(), label: "LowWR".into(),
                total_buys: 50, total_sells: 30, total_sol_spent: 10.0,
                total_sol_received: 11.0, net_pnl_sol: 1.0,
                unique_tokens_traded: 20, win_count: 5, loss_count: 15,
                win_rate: 25.0, avg_hold_time_secs: 300.0,
                biggest_win_sol: 2.0, biggest_loss_sol: -1.0,
                tokens: vec![], tx_count: 80, first_tx: None, last_tx: None,
            },
            WalletStats {
                address: "addr3".into(), label: "Negative".into(),
                total_buys: 50, total_sells: 30, total_sol_spent: 10.0,
                total_sol_received: 8.0, net_pnl_sol: -2.0,
                unique_tokens_traded: 20, win_count: 10, loss_count: 10,
                win_rate: 50.0, avg_hold_time_secs: 300.0,
                biggest_win_sol: 1.0, biggest_loss_sol: -3.0,
                tokens: vec![], tx_count: 80, first_tx: None, last_tx: None,
            },
        ];

        let recs = WalletAnalyzer::recommend_for_copy_trade(&stats, 50.0, 10);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].label, "Good");
    }
}
