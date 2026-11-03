use crate::dataflows::prediction::PredictionMarketInfo;
use crate::solana::USDC_MINT;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionTradeOrder {
    pub market_id: String,
    pub market_title: String,
    pub event_title: String,
    pub is_yes: bool,
    pub deposit_usd: f64,
    pub contract_price_usd: f64,
    pub estimated_contracts: f64,
    pub potential_payout_usd: f64,
    pub potential_net_profit_usd: f64,
    pub return_on_investment_pct: f64,
    pub implied_probability_pct: f64,
    pub status: String,
}

#[allow(dead_code)]
#[derive(Serialize)]
struct CreateOrderRequest<'a> {
    #[serde(rename = "ownerPubkey")]
    owner_pubkey: &'a str,
    #[serde(rename = "marketId")]
    market_id: &'a str,
    #[serde(rename = "isYes")]
    is_yes: bool,
    #[serde(rename = "isBuy")]
    is_buy: bool,
    #[serde(rename = "depositAmount")]
    deposit_amount: String,
    #[serde(rename = "depositMint")]
    deposit_mint: &'a str,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct CreateOrderResponse {
    pub transaction: Option<String>,
}

pub fn formulate_prediction_strategy(
    markets: &[PredictionMarketInfo],
    is_bullish: bool,
    budget_usd: f64,
) -> Option<PredictionTradeOrder> {
    if markets.is_empty() {
        return None;
    }

    // Pick market with high upside or relevant direction
    let target = if is_bullish {
        markets
            .iter()
            .find(|m| m.yes_price_usd > 0.15 && m.yes_price_usd < 0.75)
            .unwrap_or(&markets[0])
    } else {
        markets
            .iter()
            .find(|m| m.no_price_usd > 0.15 && m.no_price_usd < 0.75)
            .unwrap_or(&markets[0])
    };

    let is_yes = is_bullish;
    let contract_price = if is_yes {
        target.yes_price_usd.max(0.05)
    } else {
        target.no_price_usd.max(0.05)
    };

    let deposit_usd = budget_usd.max(5.0); // minimum Jupiter order is $5.00
    let estimated_contracts = deposit_usd / contract_price;
    let potential_payout_usd = estimated_contracts * 1.0; // each winning contract pays $1.00
    let potential_net_profit = potential_payout_usd - deposit_usd;
    let roi_pct = (potential_net_profit / deposit_usd) * 100.0;

    Some(PredictionTradeOrder {
        market_id: target.market_id.clone(),
        market_title: target.market_title.clone(),
        event_title: target.event_title.clone(),
        is_yes,
        deposit_usd,
        contract_price_usd: contract_price,
        estimated_contracts,
        potential_payout_usd,
        potential_net_profit_usd: potential_net_profit,
        return_on_investment_pct: roi_pct,
        implied_probability_pct: target.implied_probability_pct,
        status: "Prepared for Signing (Jupiter Prediction API v1)".to_string(),
    })
}

#[allow(dead_code)]
pub async fn build_jupiter_prediction_order(
    wallet_pubkey: &str,
    market_id: &str,
    is_yes: bool,
    deposit_usd: f64,
) -> Result<CreateOrderResponse> {
    let client = reqwest::Client::builder()
        .user_agent("TradingAgents-rs/0.1")
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    // Convert USD to micro USD (1,000,000 = $1.00)
    let deposit_amount = format!("{:.0}", deposit_usd * 1_000_000.0);

    let req_body = CreateOrderRequest {
        owner_pubkey: wallet_pubkey,
        market_id,
        is_yes,
        is_buy: true,
        deposit_amount,
        deposit_mint: USDC_MINT,
    };

    let res = client
        .post("https://api.jup.ag/prediction/v1/orders")
        .header("Content-Type", "application/json")
        .json(&req_body)
        .send()
        .await?;

    if !res.status().is_success() {
        let err_text = res.text().await.unwrap_or_default();
        anyhow::bail!("Jupiter Prediction Order API error: {}", err_text);
    }

    let resp: CreateOrderResponse = res.json().await?;
    Ok(resp)
}
