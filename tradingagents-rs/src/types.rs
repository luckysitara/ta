use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetType {
    Stock,
    Crypto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rating {
    StrongBuy,
    Buy,
    Hold,
    Sell,
    StrongSell,
}

impl std::fmt::Display for Rating {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rating::StrongBuy => write!(f, "STRONG BUY"),
            Rating::Buy => write!(f, "BUY"),
            Rating::Hold => write!(f, "HOLD"),
            Rating::Sell => write!(f, "SELL"),
            Rating::StrongSell => write!(f, "STRONG SELL"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalystType {
    Technical,
    Sentiment,
    News,
    Fundamentals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalystReport {
    pub analyst: AnalystType,
    pub title: String,
    pub summary: String,
    pub key_points: Vec<String>,
    pub score: f32, // -1.0 (very bearish) to +1.0 (very bullish)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateRound {
    pub bull_argument: String,
    pub bear_argument: String,
    pub manager_synthesis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub aggressive_view: String,
    pub conservative_view: String,
    pub neutral_view: String,
    pub risk_score: u8, // 1 (lowest risk) to 10 (highest risk)
    pub max_suggested_allocation_pct: f32,
    pub commentary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeDecision {
    pub ticker: String,
    pub asset_type: AssetType,
    pub rating: Rating,
    pub confidence: f32, // 0.0 to 1.0
    pub allocation_pct: f32,
    pub target_price: Option<f64>,
    pub stop_loss: Option<f64>,
    pub rationale: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    pub cash_usd: f64,
    pub holdings: HashMap<String, f64>, // ticker -> quantity
}

impl Default for Portfolio {
    fn default() -> Self {
        let mut holdings = HashMap::new();
        holdings.insert("SOL".to_string(), 10.0);
        holdings.insert("USDC".to_string(), 5000.0);
        Self {
            cash_usd: 10_000.0,
            holdings,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct AgentState {
    pub ticker: String,
    pub canonical_ticker: String,
    pub asset_type: AssetType,
    pub trade_date: String,
    pub current_price: f64,
    pub portfolio: Portfolio,
    pub analyst_reports: HashMap<AnalystType, AnalystReport>,
    pub prediction_markets: Vec<crate::dataflows::prediction::PredictionMarketInfo>,
    pub debate: Option<DebateRound>,
    pub risk: Option<RiskAssessment>,
    pub decision: Option<TradeDecision>,
}

impl Default for AssetType {
    fn default() -> Self {
        AssetType::Crypto
    }
}
