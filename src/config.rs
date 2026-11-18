use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub sniper: SniperConfig,
    #[serde(default)]
    pub breakout: BreakoutConfig,
    #[serde(default)]
    pub arbitrage: ArbitrageConfig,
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub solana: SolanaConfig,
    #[serde(default)]
    pub llm: LlmConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub paper: bool,
    pub ledger_path: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            paper: true,
            ledger_path: format!("{}/.tradingagents/ledger.json", home),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SniperConfig {
    pub budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_positions: usize,
    pub min_liquidity_sol: f64,
    pub jito_tip_sol: f64,
    pub tp1_multiplier: f64,
    pub tp2_multiplier: f64,
    pub stop_loss_pct: f64,
    pub break_even_mult: Option<f64>,
    pub live_reserves: bool,
    pub require_socials: bool,
    pub max_dev_buy_sol: Option<f64>,
    pub min_dev_buy_sol: Option<f64>,
    pub min_curve_pct: Option<f64>,
    pub max_curve_pct: Option<f64>,
    pub min_unique_buyers: usize,
    pub min_buys_recent: Option<usize>,
    pub max_sells_recent: Option<usize>,
    pub velocity_window_secs: Option<u64>,
    pub require_dev_sold_out: bool,
    pub enable_raydium: bool,
    pub mode: Option<String>,
}

impl Default for SniperConfig {
    fn default() -> Self {
        Self {
            budget_sol: 0.5,
            buy_amount_sol: 0.05,
            max_positions: 5,
            min_liquidity_sol: 1.0,
            jito_tip_sol: 0.0003,
            tp1_multiplier: 1.30,
            tp2_multiplier: 1.80,
            stop_loss_pct: -10.0,
            break_even_mult: Some(1.10),
            live_reserves: false,
            require_socials: false,
            max_dev_buy_sol: None,
            min_dev_buy_sol: None,
            min_curve_pct: None,
            max_curve_pct: None,
            min_unique_buyers: 0,
            min_buys_recent: None,
            max_sells_recent: None,
            velocity_window_secs: None,
            require_dev_sold_out: true,
            enable_raydium: true,
            mode: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakoutConfig {
    pub budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_positions: usize,
    pub jito_tip_sol: f64,
    pub min_liquidity_usd: f64,
    pub min_volume_24h_usd: f64,
    pub min_volume_5m_usd: f64,
    pub min_buys_5m: usize,
    pub min_buy_ratio_5m: f64,
    pub min_price_change_5m: f64,
    pub max_price_change_24h: f64,
    pub max_price_change_1h: f64,
    pub stop_loss_pct: f64,
    pub trailing_tp_activation_pct: f64,
    pub trailing_tp_callback_pct: f64,
    pub trailing_stop_pct: f64,
    pub break_even_mult: f64,
    pub poll_interval_secs: u64,
    pub strategy: String,
    pub min_dip_pct: f64,
    pub max_dip_pct: f64,
    pub min_buy_ratio_1h: f64,
    pub max_graduation_age_mins: f64,
    pub min_graduation_liquidity_usd: f64,
    pub min_graduation_buy_ratio_5m: f64,
}

impl Default for BreakoutConfig {
    fn default() -> Self {
        Self {
            budget_sol: 0.02,
            buy_amount_sol: 0.008,
            max_positions: 2,
            jito_tip_sol: 0.0001,
            min_liquidity_usd: 25_000.0,
            min_volume_24h_usd: 100_000.0,
            min_volume_5m_usd: 3_000.0,
            min_buys_5m: 10,
            min_buy_ratio_5m: 0.55,
            min_price_change_5m: 0.5,
            max_price_change_24h: 250.0,
            max_price_change_1h: 50.0,
            stop_loss_pct: -5.0,
            trailing_tp_activation_pct: 8.0,
            trailing_tp_callback_pct: 2.5,
            trailing_stop_pct: -10.0,
            break_even_mult: 1.08,
            poll_interval_secs: 10,
            strategy: "multi".to_string(),
            min_dip_pct: -7.5,
            max_dip_pct: -1.8,
            min_buy_ratio_1h: 0.52,
            max_graduation_age_mins: 30.0,
            min_graduation_liquidity_usd: 20_000.0,
            min_graduation_buy_ratio_5m: 0.58,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageConfig {
    pub budget_sol: f64,
    pub trade_amount_sol: f64,
    pub min_profit_bps: i64,
    pub jito_tip_sol: f64,
    pub poll_interval_secs: u64,
}

impl Default for ArbitrageConfig {
    fn default() -> Self {
        Self {
            budget_sol: 0.08,
            trade_amount_sol: 0.02,
            min_profit_bps: 30,
            jito_tip_sol: 0.0001,
            poll_interval_secs: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub chat_id: Option<String>,
    pub notify_entries: bool,
    pub notify_exits: bool,
    pub notify_rug_rejections: bool,
    pub notify_session_summary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaConfig {
    pub rpc_url: Option<String>,
    pub grpc_endpoint: Option<String>,
    pub grpc_token: Option<String>,
    pub keypair_path: Option<String>,
    pub jito_block_engine_url: Option<String>,
}

impl Default for SolanaConfig {
    fn default() -> Self {
        Self {
            rpc_url: None,
            grpc_endpoint: None,
            grpc_token: None,
            keypair_path: None,
            jito_block_engine_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: String,
    pub model: Option<String>,
    pub analysts: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            model: None,
            analysts: "technical,sentiment,news,fundamentals".to_string(),
        }
    }
}

#[allow(dead_code)]
impl AppConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let contents = std::fs::read_to_string(path.as_ref())?;
        let config: AppConfig = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn load_or_default() -> Self {
        let candidates = ["config.toml", "tradingagents.toml"];
        for name in &candidates {
            if Path::new(name).exists() {
                match Self::load(name) {
                    Ok(cfg) => {
                        tracing::info!("Loaded config from {}", name);
                        return cfg;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse {}: {}. Using defaults.", name, e);
                    }
                }
            }
        }

        if let Ok(home) = std::env::var("HOME") {
            let home_config = format!("{}/.tradingagents/config.toml", home);
            if Path::new(&home_config).exists() {
                if let Ok(cfg) = Self::load(&home_config) {
                    tracing::info!("Loaded config from {}", home_config);
                    return cfg;
                }
            }
        }

        Self::default()
    }
}
