use anyhow::Result;
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

use crate::solana::execution::{OnChainExecutor, ASSOCIATED_TOKEN_PROGRAM, TOKEN_PROGRAM};
use crate::solana::sniper::PortfolioState;
use crate::solana::SOL_MINT;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteArbSettings {
    pub total_budget_sol: f64,
    pub trade_amount_sol: f64,
    pub min_spread_bps: u64,
    pub max_price_impact_bps: u64,
    pub jito_tip_sol: f64,
    pub target_tokens: Vec<ArbToken>,
    pub poll_interval_secs: u64,
    pub max_concurrent_arbs: usize,
    pub is_paper_trading: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbToken {
    pub mint: String,
    pub symbol: String,
}

impl Default for RouteArbSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.2,
            trade_amount_sol: 0.03,
            min_spread_bps: 30,
            max_price_impact_bps: 50,
            jito_tip_sol: 0.0002,
            target_tokens: vec![
                ArbToken { mint: "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263".to_string(), symbol: "BONK".to_string() },
                ArbToken { mint: "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm".to_string(), symbol: "WIF".to_string() },
                ArbToken { mint: "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN".to_string(), symbol: "JUP".to_string() },
                ArbToken { mint: "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr".to_string(), symbol: "POPCAT".to_string() },
                ArbToken { mint: "MEW1gQWJ3nEXg2qgERiKu7FAFj79PHvQVREQUzScPP5".to_string(), symbol: "MEW".to_string() },
                ArbToken { mint: "ukHH6c7mMyiWCf1b9pnWe25TSpkDDt3H5pQZgZ74J82".to_string(), symbol: "BOME".to_string() },
                ArbToken { mint: "HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3".to_string(), symbol: "PYTH".to_string() },
                ArbToken { mint: "rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBXr".to_string(), symbol: "RENDER".to_string() },
                ArbToken { mint: "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R".to_string(), symbol: "RAY".to_string() },
                ArbToken { mint: "orcaEKTdK7LKz57vaAYr9QeNsVEPfiu6QeMU1kektZE".to_string(), symbol: "ORCA".to_string() },
                ArbToken { mint: "KMNo3nJsBXfcpJTVhZcXLW7RmTwTt4GVFE7suUBo9sS".to_string(), symbol: "KMNO".to_string() },
                ArbToken { mint: "DriFtupJYLTosbwoN8koMbEYSx54aFAVLddWsbksjwg7".to_string(), symbol: "DRIFT".to_string() },
                ArbToken { mint: "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL".to_string(), symbol: "JTO".to_string() },
                ArbToken { mint: "HeLp6NuQkmYB4pYWo2zYs22mESHXPQYzXbB8n4V98jwC".to_string(), symbol: "AI16Z".to_string() },
                ArbToken { mint: "CzLSujWBLFsSjncfkh59rUFqvafWcY5tzedWJSuypump".to_string(), symbol: "GOAT".to_string() },
                ArbToken { mint: "ED5nyyWEzpPPiWimP8vYm7sD7TD3LAt3Q3gRTWHzPJBY".to_string(), symbol: "MOODENG".to_string() },
                ArbToken { mint: "2qEHjDLDLbuBgRYvsxhc5RefwhumuAZgnusqWqb86476".to_string(), symbol: "PNUT".to_string() },
                ArbToken { mint: "A8C3xuqscfmyLrte3VmTqrAq8kgMASius9AFNANwpump".to_string(), symbol: "FARTCOIN".to_string() },
            ],
            poll_interval_secs: 5,
            max_concurrent_arbs: 2,
            is_paper_trading: true,
        }
    }
}

#[derive(Debug, Clone)]
struct RouteQuote {
    input_mint: String,
    output_mint: String,
    input_amount: u64,
    output_amount: u64,
    route_label: String,
    price_impact_bps: f64,
    slippage_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbOpportunity {
    pub token_symbol: String,
    pub token_mint: String,
    pub buy_route: String,
    pub sell_route: String,
    pub spread_bps: f64,
    pub estimated_profit_sol: f64,
    pub timestamp: DateTime<Utc>,
    pub executed: bool,
}

pub struct RouteArbEngine {
    settings: RouteArbSettings,
    executor: Option<OnChainExecutor>,
    portfolio: Arc<Mutex<PortfolioState>>,
    http_client: reqwest::Client,
    arb_history: Arc<Mutex<Vec<ArbOpportunity>>>,
    active_arbs: Arc<Mutex<usize>>,
}

impl RouteArbEngine {
    pub fn new(settings: RouteArbSettings, executor: Option<OnChainExecutor>) -> Self {
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
            portfolio: Arc::new(Mutex::new(portfolio)),
            http_client: reqwest::Client::builder()
                .user_agent("TradingAgents-rs/0.1")
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            arb_history: Arc::new(Mutex::new(Vec::new())),
            active_arbs: Arc::new(Mutex::new(0)),
        }
    }

    pub fn get_portfolio_state(&self) -> PortfolioState {
        self.portfolio.lock().unwrap().clone()
    }

    pub fn save_to_ledger(&self, ledger: &super::ledger::Ledger) {
        let port = self.portfolio.lock().unwrap();
        let history = self.arb_history.lock().unwrap();
        let empty: HashMap<String, ()> = HashMap::new();
        if let Err(e) = ledger.save_generic_state("route_arb", &*port, &empty) {
            warn!("Failed to save route_arb ledger: {}", e);
        }
    }

    async fn get_jupiter_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount_lamports: u64,
        slippage_bps: u16,
        only_direct: bool,
    ) -> Result<RouteQuote> {
        let mut url = format!(
            "https://api.jup.ag/swap/v1/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}&dexes=Raydium,Raydium%20CLMM,Raydium%20CP,Meteora,Meteora%20DLMM,Whirlpool,Orca,PancakeSwap",
            input_mint, output_mint, amount_lamports, slippage_bps
        );
        if only_direct {
            url.push_str("&onlyDirectRoutes=true");
        }

        let resp = self.http_client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Jupiter quote API returned {}", resp.status());
        }

        #[derive(Deserialize)]
        struct JupQuote {
            #[serde(rename = "inAmount")]
            in_amount: String,
            #[serde(rename = "outAmount")]
            out_amount: String,
            #[serde(rename = "priceImpactPct")]
            price_impact_pct: Option<String>,
            #[serde(rename = "routePlan")]
            route_plan: Option<Vec<RoutePlanItem>>,
        }

        #[derive(Deserialize)]
        struct RoutePlanItem {
            #[serde(rename = "swapInfo")]
            swap_info: SwapInfo,
        }

        #[derive(Deserialize)]
        struct SwapInfo {
            label: Option<String>,
        }

        let quote: JupQuote = resp.json().await?;
        let out_amount: u64 = quote.out_amount.parse().unwrap_or(0);
        let impact_pct: f64 = quote.price_impact_pct.and_then(|p| p.parse().ok()).unwrap_or(0.0);

        let route_label = quote.route_plan
            .map(|plans| {
                plans.iter()
                    .filter_map(|p| p.swap_info.label.clone())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            })
            .unwrap_or_else(|| "unknown".to_string());

        Ok(RouteQuote {
            input_mint: input_mint.to_string(),
            output_mint: output_mint.to_string(),
            input_amount: amount_lamports,
            output_amount: out_amount,
            route_label,
            price_impact_bps: impact_pct * 100.0,
            slippage_bps,
        })
    }

    async fn find_arb_opportunity(&self, token: &ArbToken) -> Option<ArbOpportunity> {
        let trade_lamports = (self.settings.trade_amount_sol * 1_000_000_000.0) as u64;

        // Get buy quote: SOL -> Token (best route)
        let buy_quote = self.get_jupiter_quote(
            &SOL_MINT, &token.mint, trade_lamports, 50, false
        ).await.ok()?;

        // Get buy quote: SOL -> Token (direct only)
        let buy_direct = self.get_jupiter_quote(
            &SOL_MINT, &token.mint, trade_lamports, 50, true
        ).await.ok();

        // Get sell quote: Token -> SOL (using best-route output amount)
        let sell_quote = self.get_jupiter_quote(
            &token.mint, &SOL_MINT, buy_quote.output_amount, 50, false
        ).await.ok()?;

        // Also try selling with direct route
        let sell_direct = self.get_jupiter_quote(
            &token.mint, &SOL_MINT, buy_quote.output_amount, 50, true
        ).await.ok();

        // Compare: best buy route -> best sell route roundtrip
        let roundtrip_output_sol = sell_quote.output_amount as f64 / 1_000_000_000.0;
        let input_sol = self.settings.trade_amount_sol;
        let net_after_tip = roundtrip_output_sol - self.settings.jito_tip_sol * 2.0;
        let spread_bps = ((net_after_tip / input_sol) - 1.0) * 10_000.0;

        // Also check: direct buy -> aggregated sell (or vice versa)
        let mut best_spread = spread_bps;
        let mut best_buy_route = buy_quote.route_label.clone();
        let mut best_sell_route = sell_quote.route_label.clone();

        if let Some(ref bd) = buy_direct {
            if let Ok(sell_from_direct) = self.get_jupiter_quote(
                &token.mint, &SOL_MINT, bd.output_amount, 50, false
            ).await {
                let rt_sol = sell_from_direct.output_amount as f64 / 1_000_000_000.0;
                let net = rt_sol - self.settings.jito_tip_sol * 2.0;
                let sp = ((net / input_sol) - 1.0) * 10_000.0;
                if sp > best_spread {
                    best_spread = sp;
                    best_buy_route = bd.route_label.clone();
                    best_sell_route = sell_from_direct.route_label;
                }
            }
        }

        if best_spread < self.settings.min_spread_bps as f64 {
            return None;
        }

        let estimated_profit_sol = input_sol * (best_spread / 10_000.0);

        Some(ArbOpportunity {
            token_symbol: token.symbol.clone(),
            token_mint: token.mint.clone(),
            buy_route: best_buy_route,
            sell_route: best_sell_route,
            spread_bps: best_spread,
            estimated_profit_sol,
            timestamp: Utc::now(),
            executed: false,
        })
    }

    async fn execute_arb(&self, opp: &mut ArbOpportunity) -> Result<()> {
        let trade_lamports = (self.settings.trade_amount_sol * 1_000_000_000.0) as u64;

        if !self.settings.is_paper_trading {
            if let Some(ref executor) = self.executor {
                let buy_result = executor
                    .execute_jupiter_swap_jito(
                        &SOL_MINT,
                        &opp.token_mint,
                        trade_lamports,
                        self.settings.jito_tip_sol,
                        100,
                    )
                    .await?;

                info!("[ROUTE_ARB] Buy leg: {}", buy_result.bundle_id);

                // Wait for buy confirmation before executing sell leg
                let buy_confirmed = executor
                    .poll_signature_confirmation(&buy_result.signature, 15, 1000)
                    .await
                    .unwrap_or(false);

                if !buy_confirmed {
                    warn!("[ROUTE_ARB] Buy leg unconfirmed for {}: {}. Skipping sell leg.", opp.token_symbol, buy_result.signature);
                    return Err(anyhow::anyhow!("Buy leg unconfirmed: {}", buy_result.signature));
                }

                // Query exact on-chain balance of token to sell 100% of it
                let token_pk = solana_sdk::pubkey::Pubkey::from_str(&opp.token_mint)?;
                let token_prog = solana_sdk::pubkey::Pubkey::from_str(TOKEN_PROGRAM)?;
                let ata_prog = solana_sdk::pubkey::Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM)?;
                let (user_ata, _) = solana_sdk::pubkey::Pubkey::find_program_address(
                    &[&executor.pubkey().to_bytes(), &token_prog.to_bytes(), &token_pk.to_bytes()],
                    &ata_prog,
                );

                let mut actual_balance = executor.get_token_balance(&user_ata).await.unwrap_or(0);
                if actual_balance == 0 {
                    let token_2022_prog = solana_sdk::pubkey::Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?;
                    let (user_ata_2022, _) = solana_sdk::pubkey::Pubkey::find_program_address(
                        &[&executor.pubkey().to_bytes(), &token_2022_prog.to_bytes(), &token_pk.to_bytes()],
                        &ata_prog,
                    );
                    actual_balance = executor.get_token_balance(&user_ata_2022).await.unwrap_or(0);
                }

                if actual_balance == 0 {
                    warn!("[ROUTE_ARB] No token balance found on-chain after buy for {}", opp.token_symbol);
                    return Err(anyhow::anyhow!("No tokens received from buy leg for {}", opp.token_symbol));
                }

                let sell_result = match executor
                    .execute_jupiter_swap_jito(
                        &opp.token_mint,
                        &SOL_MINT,
                        actual_balance,
                        self.settings.jito_tip_sol,
                        150,
                    )
                    .await
                {
                    Ok(res) => {
                        info!("[ROUTE_ARB] Sell leg: {}", res.bundle_id);
                        let _ = executor.poll_signature_confirmation(&res.signature, 15, 1000).await;
                        Some(res)
                    }
                    Err(e) => {
                        warn!("[ROUTE_ARB] Sell leg failed: {}. Attempting emergency liquidation...", e);
                        // Fallback: emergency liquidation to SOL
                        let _ = executor
                            .execute_jupiter_swap_jito(
                                &opp.token_mint,
                                &SOL_MINT,
                                0,
                                self.settings.jito_tip_sol * 2.0,
                                300,
                            )
                            .await;
                        None
                    }
                };

                // Close ATA and reclaim rent
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                let _ = executor.close_token_account_jito(&opp.token_mint, 0.0001, false).await;
            }
        }

        opp.executed = true;

        let mode = if self.settings.is_paper_trading { "PAPER" } else { "LIVE" };
        let profit_display = format!("+{:.6}", opp.estimated_profit_sol).green();

        println!(
            "  {} [ROUTE ARB {}] {} | spread: {:.1} bps | buy: {} | sell: {} | est profit: {} SOL",
            "⚡".green(),
            mode,
            opp.token_symbol.bold().cyan(),
            opp.spread_bps,
            opp.buy_route.dimmed(),
            opp.sell_route.dimmed(),
            profit_display,
        );

        {
            let mut port = self.portfolio.lock().unwrap();
            port.total_trades += 1;
            port.realized_pnl_sol += opp.estimated_profit_sol;
            if opp.estimated_profit_sol > 0.0 {
                port.winning_trades += 1;
            } else {
                port.losing_trades += 1;
            }
        }

        {
            let mut history = self.arb_history.lock().unwrap();
            history.push(opp.clone());
            if history.len() > 500 {
                history.drain(..100);
            }
        }

        Ok(())
    }

    pub async fn run_scan_cycle(&self) -> Result<usize> {
        let mut executed = 0;

        {
            let active = self.active_arbs.lock().unwrap();
            if *active >= self.settings.max_concurrent_arbs {
                return Ok(0);
            }
        }

        {
            let port = self.portfolio.lock().unwrap();
            if port.available_sol < self.settings.trade_amount_sol + self.settings.jito_tip_sol * 2.0 {
                return Ok(0);
            }
        }

        for token in &self.settings.target_tokens {
            if let Some(mut opp) = self.find_arb_opportunity(token).await {
                {
                    let mut active = self.active_arbs.lock().unwrap();
                    if *active >= self.settings.max_concurrent_arbs {
                        break;
                    }
                    *active += 1;
                }

                match self.execute_arb(&mut opp).await {
                    Ok(()) => executed += 1,
                    Err(e) => warn!("[ROUTE_ARB] Execution failed for {}: {}", token.symbol, e),
                }

                {
                    let mut active = self.active_arbs.lock().unwrap();
                    *active = active.saturating_sub(1);
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        Ok(executed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_arb_defaults() {
        let s = RouteArbSettings::default();
        assert_eq!(s.min_spread_bps, 30);
        assert_eq!(s.target_tokens.len(), 18);
    }

    #[test]
    fn test_engine_creation() {
        let settings = RouteArbSettings::default();
        let engine = RouteArbEngine::new(settings, None);
        let port = engine.get_portfolio_state();
        assert_eq!(port.initial_budget_sol, 0.2);
    }
}
