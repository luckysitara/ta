use anyhow::Result;
use serde::{Deserialize, Serialize};

pub mod arbitrage;
pub mod breakout;
pub mod execution;
pub mod prediction;
pub mod rug_filter;
pub mod sniper;
pub mod yellowstone;

#[allow(dead_code)]
pub const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
#[allow(dead_code)]
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
#[allow(dead_code)]
pub const JUP_MINT: &str = "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN";
#[allow(dead_code)]
pub const BONK_MINT: &str = "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JupiterQuote {
    #[serde(rename = "inAmount")]
    pub in_amount: String,
    #[serde(rename = "outAmount")]
    pub out_amount: String,
    #[serde(rename = "priceImpactPct")]
    pub price_impact_pct: Option<String>,
    #[serde(rename = "routePlan")]
    pub route_plan: Option<Vec<JupiterRoutePlan>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JupiterRoutePlan {
    #[serde(rename = "swapInfo")]
    pub swap_info: JupiterSwapInfo,
    pub percent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JupiterSwapInfo {
    pub label: Option<String>,
    #[serde(rename = "ammKey")]
    pub amm_key: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SolanaExecutionPlan {
    pub input_token: String,
    pub output_token: String,
    pub input_amount: f64,
    pub estimated_output: f64,
    pub dex_route: String,
    pub price_impact_pct: f64,
    pub is_simulation: bool,
}

pub async fn get_jupiter_swap_quote(
    input_mint: &str,
    output_mint: &str,
    amount_lamports: u64,
) -> Result<JupiterQuote> {
    let client = reqwest::Client::builder()
        .user_agent("TradingAgents-rs/0.1")
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    let url = format!(
        "https://api.jup.ag/swap/v1/quote?inputMint={}&outputMint={}&amount={}&slippageBps=50",
        input_mint, output_mint, amount_lamports
    );

    let res = client.get(&url).send().await?;
    if !res.status().is_success() {
        anyhow::bail!("Jupiter API returned HTTP {}", res.status());
    }

    let quote: JupiterQuote = res.json().await?;
    Ok(quote)
}

pub async fn build_solana_trade_route(
    ticker: &str,
    is_buy: bool,
    usd_amount: f64,
    estimated_sol_price: f64,
) -> SolanaExecutionPlan {
    // If buying SOL, input is USDC (6 decimals), output is SOL (9 decimals)
    // If selling SOL, input is SOL, output is USDC
    let (in_mint, out_mint, in_token, out_token, in_units) = if is_buy {
        let usdc_units = (usd_amount * 1_000_000.0) as u64;
        (USDC_MINT, SOL_MINT, "USDC".to_string(), ticker.to_string(), usdc_units)
    } else {
        let sol_amount = usd_amount / estimated_sol_price;
        let sol_lamports = (sol_amount * 1_000_000_000.0) as u64;
        (SOL_MINT, USDC_MINT, ticker.to_string(), "USDC".to_string(), sol_lamports)
    };

    // Try fetching live quote from Jupiter API
    match get_jupiter_swap_quote(in_mint, out_mint, in_units).await {
        Ok(quote) => {
            let out_num: f64 = quote.out_amount.parse().unwrap_or(0.0);
            let estimated_out = if is_buy {
                out_num / 1_000_000_000.0
            } else {
                out_num / 1_000_000.0
            };

            let route_names = quote
                .route_plan
                .map(|plans| {
                    plans
                        .into_iter()
                        .filter_map(|p| p.swap_info.label)
                        .collect::<Vec<_>>()
                        .join(" -> ")
                })
                .unwrap_or_else(|| "Jupiter Multi-Hop / Orca / Raydium".to_string());

            let impact: f64 = quote
                .price_impact_pct
                .and_then(|p| p.parse().ok())
                .unwrap_or(0.02);

            SolanaExecutionPlan {
                input_token: in_token,
                output_token: out_token,
                input_amount: usd_amount,
                estimated_output: estimated_out,
                dex_route: if route_names.is_empty() {
                    "Raydium CLMM / Whirlpool".to_string()
                } else {
                    route_names
                },
                price_impact_pct: impact,
                is_simulation: true,
            }
        }
        Err(_) => {
            // Simulated quote fallback
            let estimated_out = if is_buy {
                usd_amount / estimated_sol_price
            } else {
                usd_amount
            };

            SolanaExecutionPlan {
                input_token: in_token,
                output_token: out_token,
                input_amount: usd_amount,
                estimated_output: estimated_out,
                dex_route: "Jupiter Router (Raydium / Orca Whirlpool / Meteora DLMM)".to_string(),
                price_impact_pct: 0.04,
                is_simulation: true,
            }
        }
    }
}
