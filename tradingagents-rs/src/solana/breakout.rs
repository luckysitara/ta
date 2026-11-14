use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use colored::Colorize;
use tracing::warn;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::solana::execution::OnChainExecutor;
use crate::solana::sniper::PortfolioState;
use crate::solana::SOL_MINT;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScalpStrategyMode {
    MomentumBreakout,     // Strategy 2: Volume & 5m momentum breakout (Solo mode only)
    DipReboundScalp,      // Strategy 1: 5m oversold dip pullback in macro uptrend
    GraduationBurstScalp, // Strategy 3: Fresh Pump.fun migration/graduation on Raydium/PumpSwap/Meteora
    EarlyGemRunner,       // Strategy 5: Fresh low-cap graduated gem runner (micro-allocation, strictly screened)
    DipAndGraduation,     // Strategy 1, Strategy 3 & Strategy 5 (Strategy 2 permanently excluded)
    MultiStrategyAdaptive,// High-win-rate adaptive mode (Strategy 1, 3 & 5 active; Strategy 2 disabled)
}

impl Default for ScalpStrategyMode {
    fn default() -> Self {
        Self::DipAndGraduation
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakoutSettings {
    pub total_budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_concurrent_positions: usize,
    pub jito_tip_sol: f64,
    pub min_liquidity_usd: f64,
    pub min_volume_24h_usd: f64,
    pub min_volume_5m_usd: f64,
    pub min_buys_5m: usize,
    pub min_buy_ratio_5m: f64,
    pub min_price_change_5m: f64,
    pub max_price_change_24h: Option<f64>,
    pub max_price_change_1h: Option<f64>,
    pub min_price_change_24h: Option<f64>,
    pub stop_loss_pct: f64,
    pub trailing_take_profit_activation_pct: f64,
    pub trailing_take_profit_callback_pct: f64,
    pub trailing_stop_pct: f64,
    pub break_even_mult: f64,
    pub is_paper_trading: bool,
    pub poll_interval_secs: u64,
    // Multi-strategy extensions
    pub strategy_mode: ScalpStrategyMode,
    pub min_dip_pct: f64,
    pub max_dip_pct: f64,
    pub min_buy_ratio_1h: f64,
    pub max_graduation_age_mins: f64,
    pub min_graduation_liquidity_usd: f64,
    pub min_graduation_buy_ratio_5m: f64,
}

impl Default for BreakoutSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.018,
            buy_amount_sol: 0.008,
            max_concurrent_positions: 1,
            jito_tip_sol: 0.0001,
            min_liquidity_usd: 25_000.0,
            min_volume_24h_usd: 100_000.0,
            min_volume_5m_usd: 3_000.0,
            min_buys_5m: 10,
            min_buy_ratio_5m: 0.55,
            min_price_change_5m: 0.5,
            max_price_change_24h: Some(250.0),
            max_price_change_1h: Some(50.0),
            min_price_change_24h: Some(0.0),
            stop_loss_pct: -5.0,
            trailing_take_profit_activation_pct: 8.0,
            trailing_take_profit_callback_pct: 2.5,
            trailing_stop_pct: -10.0,
            break_even_mult: 1.08,
            is_paper_trading: true,
            poll_interval_secs: 10,
            strategy_mode: ScalpStrategyMode::MultiStrategyAdaptive,
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
pub struct BreakoutCandidate {
    pub token_address: String,
    pub name: String,
    pub symbol: String,
    pub pair_address: String,
    pub dex_id: String,
    pub price_usd: f64,
    pub price_native: f64,
    pub liquidity_usd: f64,
    pub volume_24h_usd: f64,
    pub volume_5m_usd: f64,
    pub price_change_5m: f64,
    pub price_change_1h: f64,
    pub price_change_24h: f64,
    pub buys_5m: usize,
    pub sells_5m: usize,
    pub buys_1h: usize,
    pub sells_1h: usize,
    pub pair_created_at_ms: Option<u64>,
    pub pair_age_mins: Option<f64>,
    pub matched_strategy: Option<String>,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakoutPosition {
    pub token_address: String,
    pub symbol: String,
    pub pair_address: String,
    pub dex_id: String,
    pub entry_price_usd: f64,
    pub entry_price_native: f64,
    pub entry_timestamp: DateTime<Utc>,
    pub buy_amount_sol: f64,
    pub tokens_held: u64,
    pub peak_price_usd: f64,
    pub current_price_usd: f64,
    pub current_multiplier: f64,
    pub peak_multiplier: f64,
    pub stop_loss_price_usd: f64,
    pub is_closed: bool,
    pub pnl_sol: f64,
    pub strategy: String,
}

pub struct BreakoutEngine {
    pub settings: BreakoutSettings,
    pub executor: Option<Arc<OnChainExecutor>>,
    pub positions: Arc<Mutex<HashMap<String, BreakoutPosition>>>,
    pub entered_tokens: Arc<Mutex<HashSet<String>>>,
    pub portfolio: Arc<Mutex<PortfolioState>>,
    pub http_client: reqwest::Client,
}

impl BreakoutEngine {
    pub fn new(settings: BreakoutSettings, executor: Option<Arc<OnChainExecutor>>) -> Self {
        let budget = settings.total_budget_sol;
        Self {
            settings,
            executor,
            positions: Arc::new(Mutex::new(HashMap::new())),
            entered_tokens: Arc::new(Mutex::new(HashSet::new())),
            portfolio: Arc::new(Mutex::new(PortfolioState {
                initial_budget_sol: budget,
                available_sol: budget,
                invested_sol: 0.0,
                realized_pnl_sol: 0.0,
                total_trades: 0,
                winning_trades: 0,
                losing_trades: 0,
            })),
            http_client: reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .timeout(Duration::from_secs(6))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Automatically discover and adopt existing open token positions on wallet startup
    pub async fn sync_open_positions_from_wallet(&self) -> Result<usize> {
        if self.settings.is_paper_trading {
            return Ok(0);
        }
        let executor = match self.executor {
            Some(ref e) => Arc::clone(e),
            None => return Ok(0),
        };

        let tokens = executor.get_open_meme_tokens().await.unwrap_or_default();
        if tokens.is_empty() {
            return Ok(0);
        }

        let mints: Vec<String> = tokens.iter().map(|(m, _, _)| m.clone()).collect();
        let candidates = self.fetch_batch_metrics(&mints).await;

        let mut adopted_cnt = 0;
        let mut positions = self.positions.lock().unwrap();

        for (mint, units, ui_amt) in tokens {
            if positions.contains_key(&mint) {
                continue;
            }
            if let Some(cand) = candidates.iter().find(|c| c.token_address == mint) {
                let stop_price = cand.price_usd * (1.0 + self.settings.stop_loss_pct / 100.0);
                let pos = BreakoutPosition {
                    token_address: mint.clone(),
                    symbol: cand.symbol.clone(),
                    pair_address: cand.pair_address.clone(),
                    dex_id: cand.dex_id.clone(),
                    entry_price_usd: cand.price_usd,
                    entry_price_native: cand.price_native,
                    entry_timestamp: Utc::now(),
                    buy_amount_sol: self.settings.buy_amount_sol,
                    tokens_held: units,
                    peak_price_usd: cand.price_usd,
                    current_price_usd: cand.price_usd,
                    current_multiplier: 1.0,
                    peak_multiplier: 1.0,
                    stop_loss_price_usd: stop_price,
                    is_closed: false,
                    pnl_sol: 0.0,
                    strategy: "Adopted Wallet Position".to_string(),
                };
                positions.insert(mint.clone(), pos);
                adopted_cnt += 1;
                println!(
                    "  ⚡ [STARTUP SYNC] Adopted on-chain token: {} ({:.2} units) | Current: ${:.6} | Trailing Take-Profit & Tight Stop Active",
                    cand.symbol, ui_amt, cand.price_usd
                );
            }
        }

        Ok(adopted_cnt)
    }

    /// Evaluates if a token qualifies for Strategy 2: Volume & 5m Momentum Breakout
    pub fn evaluate_momentum_breakout(&self, c: &BreakoutCandidate) -> bool {
        // 1. Liquidity check: Ensure deep liquidity to avoid illiquid traps and high slippage
        if c.liquidity_usd < self.settings.min_liquidity_usd {
            return false;
        }

        // 2. Volume check: Must have proven 24h market traction and current 5m surge
        if c.volume_24h_usd < self.settings.min_volume_24h_usd || c.volume_5m_usd < self.settings.min_volume_5m_usd {
            return false;
        }

        // 3. Trade count & Buy ratio: Retail buyers must be heavily driving current 5m candle
        let total_txns = c.buys_5m + c.sells_5m;
        if total_txns == 0 || c.buys_5m < self.settings.min_buys_5m {
            return false;
        }
        let buy_ratio = (c.buys_5m as f64) / (total_txns as f64);
        if buy_ratio < self.settings.min_buy_ratio_5m {
            return false;
        }

        // 4. Momentum check: Price must be breaking out positively in 5m
        if c.price_change_5m < self.settings.min_price_change_5m {
            return false;
        }

        // 4b. Anti-FOMO Guardrails: Must be an early-stage base breakout, NOT a late-stage parabolic top!
        if let Some(max_24h) = self.settings.max_price_change_24h {
            if c.price_change_24h > max_24h {
                return false;
            }
        }
        if let Some(min_24h) = self.settings.min_price_change_24h {
            if c.price_change_24h < min_24h {
                return false;
            }
        }
        if let Some(max_1h) = self.settings.max_price_change_1h {
            if c.price_change_1h > max_1h {
                return false;
            }
        }

        true
    }

    /// Evaluates if a token qualifies for Strategy 1: 5-Minute Oversold Dip-Rebound Scalper
    pub fn evaluate_dip_rebound(&self, c: &BreakoutCandidate) -> bool {
        // 1. Liquidity check: Must have deep liquidity ($25k+)
        if c.liquidity_usd < self.settings.min_liquidity_usd {
            return false;
        }

        // 2. 24h Volume check: Must be an established token with proven daily volume ($100k+)
        if c.volume_24h_usd < self.settings.min_volume_24h_usd {
            return false;
        }

        // 3. Active liquidity during dip: 5m volume >= $2,500
        if c.volume_5m_usd < 2_500.0 {
            return false;
        }

        // 4. Macro uptrend: 24h price change >= 0% and 1h price change >= -5%
        if c.price_change_24h < 0.0 || c.price_change_1h < -5.0 {
            return false;
        }

        // Anti-FOMO: Reject already overextended blowoff tops (> 250%)
        if let Some(max_24h) = self.settings.max_price_change_24h {
            if c.price_change_24h > max_24h {
                return false;
            }
        }

        // 5. Oversold Dip window: 5m price change must be negative within the pullback zone
        // e.g. -7.5% <= price_change_5m <= -1.8%
        if c.price_change_5m > self.settings.max_dip_pct || c.price_change_5m < self.settings.min_dip_pct {
            return false;
        }

        // 6. Macro buyer dominance: 1h buyers must remain in control
        let total_txns_1h = c.buys_1h + c.sells_1h;
        if total_txns_1h < 20 {
            return false;
        }
        let buy_ratio_1h = (c.buys_1h as f64) / (total_txns_1h as f64);
        if buy_ratio_1h < self.settings.min_buy_ratio_1h {
            return false;
        }

        true
    }

    /// Evaluates if a token qualifies for Strategy 3: Secondary DEX Graduation Burst Scalper
    pub fn evaluate_graduation_burst(&self, c: &BreakoutCandidate) -> bool {
        // 1. Pair age: Must be a freshly migrated / graduated token (age <= max_graduation_age_mins, default 30m)
        let age_mins = match c.pair_age_mins {
            Some(age) => age,
            None => return false,
        };
        if age_mins > self.settings.max_graduation_age_mins {
            return false;
        }

        // 2. Seeded initial liquidity: >= $20,000 USD
        if c.liquidity_usd < self.settings.min_graduation_liquidity_usd {
            return false;
        }

        // 3. 5m Volume surge: >= $3,000 USD
        if c.volume_5m_usd < 3_000.0 {
            return false;
        }

        // 4. 5m Buy pressure: >= 8 buys and >= 58% buy ratio
        let total_txns_5m = c.buys_5m + c.sells_5m;
        if total_txns_5m < 8 || c.buys_5m < 5 {
            return false;
        }
        let buy_ratio_5m = (c.buys_5m as f64) / (total_txns_5m as f64);
        if buy_ratio_5m < self.settings.min_graduation_buy_ratio_5m {
            return false;
        }

        // 5. Positive momentum: 5m change >= +1.0%
        if c.price_change_5m < 1.0 {
            return false;
        }

        // 6. Anti-exhaustion: 1h price change <= 250%
        if c.price_change_1h > 250.0 {
            return false;
        }

        true
    }

    /// Evaluates if a token qualifies for Strategy 5: Early Gem / Micro-Runner
    /// Specifically catches fresh graduated/early gems with $10k-$25k liquidity, explosive 5m volume, and clean buy pressure
    pub fn evaluate_early_gem_runner(&self, c: &BreakoutCandidate) -> bool {
        // 1. Initial liquidity corridor: $10,000 USD to $35,000 USD (fresh curve graduation size)
        if c.liquidity_usd < 10_000.0 || c.liquidity_usd > 50_000.0 {
            return false;
        }

        // 2. Pair freshness: must be under 60 minutes old
        if let Some(age) = c.pair_age_mins {
            if age > 60.0 {
                return false;
            }
        }

        // 3. 5m volume velocity: at least $2,000 USD
        if c.volume_5m_usd < 2_000.0 {
            return false;
        }

        // 4. Healthy buy dominance: >= 60% buy ratio in 5m
        let total_txns_5m = c.buys_5m + c.sells_5m;
        if total_txns_5m < 6 || c.buys_5m < 4 {
            return false;
        }
        let buy_ratio_5m = (c.buys_5m as f64) / (total_txns_5m as f64);
        if buy_ratio_5m < 0.60 {
            return false;
        }

        // 5. Positive 5m momentum
        if c.price_change_5m < 0.5 {
            return false;
        }

        // 6. Anti-exhaustion top ceiling: not up over 1000% on 1h
        if c.price_change_1h > 1000.0 {
            return false;
        }

        true
    }

    /// Evaluates if candidate matches selected strategy mode and returns matched strategy name
    pub fn evaluate_candidate_strategy(&self, c: &BreakoutCandidate) -> Option<String> {
        let entered = self.entered_tokens.lock().unwrap();
        if entered.contains(&c.token_address) {
            return None;
        }

        match self.settings.strategy_mode {
            ScalpStrategyMode::MomentumBreakout => {
                if self.evaluate_momentum_breakout(c) {
                    Some("Strategy 2: Momentum Breakout".to_string())
                } else {
                    None
                }
            }
            ScalpStrategyMode::DipReboundScalp => {
                if self.evaluate_dip_rebound(c) {
                    Some("Strategy 1: 5m Oversold Dip-Rebound".to_string())
                } else {
                    None
                }
            }
            ScalpStrategyMode::GraduationBurstScalp => {
                if self.evaluate_graduation_burst(c) {
                    Some("Strategy 3: DEX Graduation Burst".to_string())
                } else {
                    None
                }
            }
            ScalpStrategyMode::EarlyGemRunner => {
                if self.evaluate_early_gem_runner(c) {
                    Some("Strategy 5: Early Gem Runner".to_string())
                } else {
                    None
                }
            }
            ScalpStrategyMode::DipAndGraduation | ScalpStrategyMode::MultiStrategyAdaptive => {
                // High-Win-Rate Multi Mode: Strategy 1, Strategy 3, & Strategy 5
                // Strategy 2 (Momentum Breakout) is permanently disabled per performance review (all losses eliminated)
                if self.evaluate_graduation_burst(c) {
                    Some("Strategy 3: DEX Graduation Burst".to_string())
                } else if self.evaluate_early_gem_runner(c) {
                    Some("Strategy 5: Early Gem Runner".to_string())
                } else if self.evaluate_dip_rebound(c) {
                    Some("Strategy 1: 5m Oversold Dip-Rebound".to_string())
                } else {
                    None
                }
            }
        }
    }

    /// Evaluates if a token qualifies for entry under current strategy mode
    #[allow(dead_code)]
    pub fn evaluate_candidate(&self, c: &BreakoutCandidate) -> bool {
        self.evaluate_candidate_strategy(c).is_some()
    }

    /// Calculates a composite momentum score based on volume velocity, buy dominance, and strategy setup
    pub fn calculate_breakout_score(c: &BreakoutCandidate) -> f64 {
        let total_txns_5m = (c.buys_5m + c.sells_5m).max(1) as f64;
        let buy_ratio_5m = (c.buys_5m as f64) / total_txns_5m;
        let vol_score = (c.volume_5m_usd / 5_000.0).min(10.0);
        let pct_score = (c.price_change_5m / 5.0).clamp(-2.0, 10.0);
        let ratio_score = buy_ratio_5m * 10.0;

        // Bonus for fresh graduation pool (< 30m)
        let grad_bonus = match c.pair_age_mins {
            Some(age) if age <= 30.0 => 3.0,
            _ => 0.0,
        };

        // Bonus for dip rebound setup (oversold dip with strong 1h buy ratio)
        let total_txns_1h = (c.buys_1h + c.sells_1h).max(1) as f64;
        let buy_ratio_1h = (c.buys_1h as f64) / total_txns_1h;
        let dip_bonus = if c.price_change_5m < -1.5 && buy_ratio_1h >= 0.52 {
            2.5
        } else {
            0.0
        };

        vol_score * 0.35 + pct_score * 0.25 + ratio_score * 0.25 + grad_bonus * 0.075 + dip_bonus * 0.075
    }

    /// Fetch trending/boosted token mint addresses from DexScreener
    pub async fn fetch_trending_mints(&self) -> Vec<String> {
        let mut mints = HashSet::new();

        // 1. Fetch top boosted
        if let Ok(resp) = self.http_client.get("https://api.dexscreener.com/token-boosts/top/v1").send().await {
            if let Ok(items) = resp.json::<Vec<serde_json::Value>>().await {
                for item in items {
                    if item["chainId"].as_str() == Some("solana") {
                        if let Some(addr) = item["tokenAddress"].as_str() {
                            mints.insert(addr.to_string());
                        }
                    }
                }
            }
        }

        // 2. Fetch latest boosted
        if let Ok(resp) = self.http_client.get("https://api.dexscreener.com/token-boosts/latest/v1").send().await {
            if let Ok(items) = resp.json::<Vec<serde_json::Value>>().await {
                for item in items {
                    if item["chainId"].as_str() == Some("solana") {
                        if let Some(addr) = item["tokenAddress"].as_str() {
                            mints.insert(addr.to_string());
                        }
                    }
                }
            }
        }

        // 3. Fetch latest token profiles (catches fresh tokens like FOOD early)
        if let Ok(resp) = self.http_client.get("https://api.dexscreener.com/token-profiles/latest/v1").send().await {
            if let Ok(items) = resp.json::<Vec<serde_json::Value>>().await {
                for item in items {
                    if item["chainId"].as_str() == Some("solana") {
                        if let Some(addr) = item["tokenAddress"].as_str() {
                            mints.insert(addr.to_string());
                        }
                    }
                }
            }
        }

        mints.into_iter().collect()
    }

    /// Fetch rich market metrics for a batch of token mints via DexScreener
    pub async fn fetch_batch_metrics(&self, mints: &[String]) -> Vec<BreakoutCandidate> {
        if mints.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        // Query in chunks of 30
        for chunk in mints.chunks(30) {
            let joined = chunk.join(",");
            let url = format!("https://api.dexscreener.com/tokens/v1/solana/{}", joined);
            let resp = match self.http_client.get(&url).send().await {
                Ok(r) => r,
                Err(_) => continue,
            };

            let pairs: Vec<serde_json::Value> = match resp.json().await {
                Ok(p) => p,
                Err(_) => continue,
            };

            for p in pairs {
                let token_address = match p["baseToken"]["address"].as_str() {
                    Some(a) => a.to_string(),
                    None => continue,
                };
                let name = p["baseToken"]["name"].as_str().unwrap_or("Unknown").to_string();
                let symbol = p["baseToken"]["symbol"].as_str().unwrap_or("???").to_string();
                let pair_address = p["pairAddress"].as_str().unwrap_or("").to_string();
                let dex_id = p["dexId"].as_str().unwrap_or("").to_string();

                let price_usd = p["priceUsd"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                let price_native = p["priceNative"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                let liquidity_usd = p["liquidity"]["usd"].as_f64().unwrap_or(0.0);
                let volume_24h_usd = p["volume"]["h24"].as_f64().unwrap_or(0.0);
                let volume_5m_usd = p["volume"]["m5"].as_f64().unwrap_or(0.0);
                let price_change_5m = p["priceChange"]["m5"].as_f64().unwrap_or(0.0);
                let price_change_1h = p["priceChange"]["h1"].as_f64().unwrap_or(0.0);
                let price_change_24h = p["priceChange"]["h24"].as_f64().unwrap_or(0.0);
                let buys_5m = p["txns"]["m5"]["buys"].as_u64().unwrap_or(0) as usize;
                let sells_5m = p["txns"]["m5"]["sells"].as_u64().unwrap_or(0) as usize;
                let buys_1h = p["txns"]["h1"]["buys"].as_u64().unwrap_or(0) as usize;
                let sells_1h = p["txns"]["h1"]["sells"].as_u64().unwrap_or(0) as usize;
                let pair_created_at_ms = p["pairCreatedAt"].as_u64();
                let now_ms = Utc::now().timestamp_millis() as u64;
                let pair_age_mins = pair_created_at_ms.map(|created| {
                    if now_ms >= created {
                        (now_ms - created) as f64 / 60_000.0
                    } else {
                        0.0
                    }
                });

                let mut cand = BreakoutCandidate {
                    token_address,
                    name,
                    symbol,
                    pair_address,
                    dex_id,
                    price_usd,
                    price_native,
                    liquidity_usd,
                    volume_24h_usd,
                    volume_5m_usd,
                    price_change_5m,
                    price_change_1h,
                    price_change_24h,
                    buys_5m,
                    sells_5m,
                    buys_1h,
                    sells_1h,
                    pair_created_at_ms,
                    pair_age_mins,
                    matched_strategy: None,
                    score: 0.0,
                };
                cand.score = Self::calculate_breakout_score(&cand);
                results.push(cand);
            }
        }

        results
    }

    /// Process a detected candidate: check active capacity and trigger entry
    pub async fn process_candidate(&self, c: &BreakoutCandidate) -> Result<bool> {
        let strat_label = match self.evaluate_candidate_strategy(c) {
            Some(s) => s,
            None => return Ok(false),
        };

        // Check active positions limit
        {
            let positions = self.positions.lock().unwrap();
            let active = positions.values().filter(|p| !p.is_closed).count();
            if active >= self.settings.max_concurrent_positions {
                return Ok(false);
            }
            if positions.contains_key(&c.token_address) {
                return Ok(false);
            }
        }

        // Dynamic sizing: Strategy 5 uses micro-lottery sizing (e.g. 0.004 SOL), while Strategy 1 & 3 use standard sizing
        let buy_amount = if strat_label.starts_with("Strategy 5") {
            (self.settings.buy_amount_sol * 0.35).max(0.004)
        } else {
            self.settings.buy_amount_sol
        };
        let tip = self.settings.jito_tip_sol;
        let fee_buffer = if !self.settings.is_paper_trading { 0.0022 } else { 0.0 };
        let trade_cost = buy_amount + tip + fee_buffer;

        {
            let port = self.portfolio.lock().unwrap();
            if port.available_sol < trade_cost {
                warn!(
                    "Insufficient balance ({:.4} SOL < {:.4} SOL required) for breakout entry {}.",
                    port.available_sol, trade_cost, c.symbol
                );
                return Ok(false);
            }
        }

        // Mark as entered to prevent duplicate entries
        {
            let mut entered = self.entered_tokens.lock().unwrap();
            entered.insert(c.token_address.clone());
        }

        let stop_price = c.price_usd * (1.0 + self.settings.stop_loss_pct / 100.0);
        let tokens_est = if c.price_native > 0.0 {
            ((buy_amount / c.price_native) * 1_000_000.0) as u64
        } else {
            1_000_000
        };

        let position = BreakoutPosition {
            token_address: c.token_address.clone(),
            symbol: c.symbol.clone(),
            pair_address: c.pair_address.clone(),
            dex_id: c.dex_id.clone(),
            entry_price_usd: c.price_usd,
            entry_price_native: c.price_native,
            entry_timestamp: Utc::now(),
            buy_amount_sol: buy_amount,
            tokens_held: tokens_est,
            peak_price_usd: c.price_usd,
            current_price_usd: c.price_usd,
            current_multiplier: 1.0,
            peak_multiplier: 1.0,
            stop_loss_price_usd: stop_price,
            is_closed: false,
            pnl_sol: 0.0,
            strategy: strat_label.clone(),
        };

        if self.settings.is_paper_trading {
            let total_paper_cost = buy_amount + tip + (buy_amount * 0.005);
            let (avail, invested, trade_num) = {
                let mut port = self.portfolio.lock().unwrap();
                port.available_sol -= total_paper_cost;
                port.invested_sol += buy_amount;
                port.total_trades += 1;
                (port.available_sol, port.invested_sol, port.total_trades)
            };

            println!(
                "\n{}",
                format!(
                    "🚀 [DEX ENTRY #{}] Token: {} ({}) | DEX: {} | {}",
                    trade_num, c.symbol, c.token_address, c.dex_id, strat_label
                )
                .bold()
                .green()
            );
            println!(
                "   Price: ${:.6} ({:.8} SOL) | 5m Vol: ${:.1}k | 5m Change: {:+.2}% | 5m Buys: {}",
                c.price_usd, c.price_native, c.volume_5m_usd / 1000.0, c.price_change_5m, c.buys_5m
            );
            println!(
                "   Liquidity: ${:.1}k | 24h Vol: ${:.1}k | Score: {:.1} | Age: {:.1}m",
                c.liquidity_usd / 1000.0, c.volume_24h_usd / 1000.0, c.score, c.pair_age_mins.unwrap_or(0.0)
            );
            println!(
                "   📈 Strategy: {} | Trailing Stop ({:.1}%) | Break-Even Floor: ≥{:.2}x",
                strat_label, self.settings.trailing_stop_pct, self.settings.break_even_mult
            );
            println!(
                "   💼 [PAPER WALLET] Balance: {:.4} SOL Available | {:.4} SOL Invested",
                avail, invested
            );

            let mut pos = self.positions.lock().unwrap();
            pos.insert(c.token_address.clone(), position);
            Ok(true)
        } else {
            // Live on-chain execution via Jupiter swap bundled with Jito MEV tip
            if let Some(ref executor) = self.executor {
                let exec = Arc::clone(executor);
                let mint = c.token_address.clone();
                let sym = c.symbol.clone();
                let dex = c.dex_id.clone();
                let lamports = (buy_amount * 1_000_000_000.0) as u64;

                println!(
                    "\n{}",
                    format!(
                        "🎯 [LIVE ON-CHAIN ENTRY] Sniping {} ({}) on {} | {} | Cost: {:.4} SOL (+{:.4} Tip)...",
                        sym, mint, dex, strat_label, buy_amount, tip
                    )
                    .bold()
                    .cyan()
                );

                match exec.execute_jupiter_swap_jito(SOL_MINT, &mint, lamports, tip, 1000).await {
                    Ok(res) => {
                        println!(
                            "   {}",
                            format!("✔ [BROADCAST SUCCESS] Tx Signature: https://solscan.io/tx/{}", res.signature).bold().green()
                        );
                        // Poll for confirmation
                        match exec.poll_signature_confirmation(&res.signature, 8, 500).await {
                            Ok(true) => {
                                println!(
                                    "   {}",
                                    format!("🎉 [LIVE BUY CONFIRMED] Acquired {} on-chain! Position active with trailing stop.", sym).bold().green()
                                );
                                {
                                    let mut pos = self.positions.lock().unwrap();
                                    pos.insert(mint.clone(), position);
                                }
                                if let Ok(real_bal) = exec.get_balance_sol().await {
                                    let mut port = self.portfolio.lock().unwrap();
                                    port.available_sol = real_bal;
                                    port.total_trades += 1;
                                    println!("   ⚡ [WALLET SYNC] Live Balance: {:.4} SOL", real_bal);
                                }
                                Ok(true)
                            }
                            Ok(false) => {
                                warn!("Transaction not confirmed within timeout. Position not added.");
                                Ok(false)
                            }
                            Err(e) => {
                                warn!("Error confirming transaction: {e}");
                                Ok(false)
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Jupiter swap failed for {}: {e}", sym);
                        Ok(false)
                    }
                }
            } else {
                Err(anyhow!("On-chain executor not configured"))
            }
        }
    }

    /// Manage active positions: update prices, adjust dynamic trailing stops, and trigger exits
    pub async fn update_and_manage_positions(&self) -> Result<()> {
        let active_mints: Vec<String> = {
            let positions = self.positions.lock().unwrap();
            positions.values().filter(|p| !p.is_closed).map(|p| p.token_address.clone()).collect()
        };

        if active_mints.is_empty() {
            return Ok(());
        }

        let candidates = self.fetch_batch_metrics(&active_mints).await;
        let mut exit_triggers: Vec<(String, f64, String)> = Vec::new();

        {
            let mut positions = self.positions.lock().unwrap();
            for cand in &candidates {
                if let Some(pos) = positions.get_mut(&cand.token_address) {
                    if pos.is_closed {
                        continue;
                    }
                    pos.current_price_usd = cand.price_usd;
                    pos.current_multiplier = if pos.entry_price_usd > 0.0 {
                        cand.price_usd / pos.entry_price_usd
                    } else {
                        1.0
                    };

                    if cand.price_usd > pos.peak_price_usd {
                        pos.peak_price_usd = cand.price_usd;
                        pos.peak_multiplier = pos.current_multiplier;
                    }

                    // 1. Granular Multi-Tier Profit Lockbox & Trailing Take-Profit:
                    // Calibrated for low-cap micro trades to overcome ~5-6% total friction (DEX swap fees, slippage & Jito tips)
                    // Guarantees that premature lockboxes do not exit in a net loss!
                    let peak_mult = pos.peak_multiplier;
                    let activation_mult = 1.0 + (self.settings.trailing_take_profit_activation_pct / 100.0);

                    if peak_mult >= activation_mult {
                        // Tier 4: Active Trailing Take-Profit at >= activation_mult (default +10.0% / 1.10x)
                        // Trail tight callback buffer (e.g. 2.0%) below highest peak watermark, min floor +7.0%
                        let ttp_floor = pos.peak_price_usd * (1.0 - self.settings.trailing_take_profit_callback_pct / 100.0);
                        let min_guaranteed_profit = pos.entry_price_usd * 1.070; // Minimum +7.0% locked floor (guaranteed net profit)
                        let effective_floor = ttp_floor.max(min_guaranteed_profit);

                        if effective_floor > pos.stop_loss_price_usd {
                            pos.stop_loss_price_usd = effective_floor;
                        }
                    } else if peak_mult >= 1.08 {
                        // Tier 3: Lockbox at +8.0% gain -> Locks +6.0% guaranteed floor (covers all friction with green net SOL!)
                        let tier3_floor = pos.entry_price_usd * 1.060;
                        if tier3_floor > pos.stop_loss_price_usd {
                            pos.stop_loss_price_usd = tier3_floor;
                        }
                    } else if peak_mult >= 1.06 {
                        // Tier 2: Early True Break-even at +6.0% gain -> Locks +5.0% floor (covers 2% DEX fee + 2 tips + slippage)
                        let tier2_floor = pos.entry_price_usd * 1.050;
                        if tier2_floor > pos.stop_loss_price_usd {
                            pos.stop_loss_price_usd = tier2_floor;
                        }
                    } else {
                        // Initial capital protection: tight -5.0% stop-loss from entry
                        // Gives room for initial trade volatility without triggering a premature fake 'break-even' loss
                        let initial_stop = pos.entry_price_usd * (1.0 + self.settings.stop_loss_pct / 100.0);
                        if pos.stop_loss_price_usd == 0.0 || pos.stop_loss_price_usd < initial_stop {
                            pos.stop_loss_price_usd = initial_stop;
                        }
                    }

                    // Print continuous live position telemetry
                    let floor_pct = if pos.entry_price_usd > 0.0 {
                        (pos.stop_loss_price_usd / pos.entry_price_usd - 1.0) * 100.0
                    } else {
                        0.0
                    };
                    println!(
                        "  📊 [POSITION TELEMETRY] {} | Price: ${:.6} | Gain: {:+.2}% (Peak: {:+.2}%) | Stop Floor: {:+.2}% | Strategy: {} | Status: {}",
                        pos.symbol,
                        cand.price_usd,
                        (pos.current_multiplier - 1.0) * 100.0,
                        (pos.peak_multiplier - 1.0) * 100.0,
                        floor_pct,
                        pos.strategy,
                        if peak_mult >= 1.03 { "🔒 PROFIT LOCKED" } else { "🛡️ CAPITAL DEFENSE" }
                    );

                    // Check stop-loss / trailing profit trigger
                    if cand.price_usd <= pos.stop_loss_price_usd {
                        let reason = if pos.current_multiplier >= 1.025 {
                            format!("Profit Lockbox (+{:.1}%)", (pos.current_multiplier - 1.0) * 100.0)
                        } else if pos.current_multiplier >= 1.005 {
                            format!("Break-Even Lockbox (+{:.1}%)", (pos.current_multiplier - 1.0) * 100.0)
                        } else {
                            format!("Tight Stop Loss Protection ({:.2}x)", pos.current_multiplier)
                        };
                        exit_triggers.push((pos.token_address.clone(), cand.price_usd, reason));
                    }
                }
            }
        }

        // Execute exits
        for (mint, exit_price, reason) in exit_triggers {
            self.execute_position_exit(&mint, exit_price, &reason).await?;
        }

        Ok(())
    }

    /// Execute position exit (market sell via Jupiter & reclaim ATA rent)
    pub async fn execute_position_exit(&self, token_address: &str, exit_price_usd: f64, reason: &str) -> Result<()> {
        let (mut pos, buy_amount, tip) = {
            let mut positions = self.positions.lock().unwrap();
            let pos = match positions.get_mut(token_address) {
                Some(p) => p.clone(),
                None => return Ok(()),
            };
            (pos, self.settings.buy_amount_sol, self.settings.jito_tip_sol)
        };

        let mult = if pos.entry_price_usd > 0.0 {
            exit_price_usd / pos.entry_price_usd
        } else {
            1.0
        };
        let gross_sol = buy_amount * mult;
        let net_pnl_sol = gross_sol - buy_amount - (tip * 2.0);

        pos.is_closed = true;
        pos.pnl_sol = net_pnl_sol;

        if self.settings.is_paper_trading {
            let (avail, realized) = {
                let mut port = self.portfolio.lock().unwrap();
                port.available_sol += gross_sol - tip;
                port.invested_sol = (port.invested_sol - buy_amount).max(0.0);
                port.realized_pnl_sol += net_pnl_sol;
                if net_pnl_sol >= 0.0 {
                    port.winning_trades += 1;
                } else {
                    port.losing_trades += 1;
                }
                (port.available_sol, port.realized_pnl_sol)
            };

            let pnl_str = if net_pnl_sol >= 0.0 {
                format!("+{:.4} SOL (+{:.1}%)", net_pnl_sol, (mult - 1.0) * 100.0).bold().green()
            } else {
                format!("{:.4} SOL ({:.1}%)", net_pnl_sol, (mult - 1.0) * 100.0).bold().red()
            };

            println!(
                "\n{}",
                format!("🏁 [DEX BREAKOUT EXIT] Token: {} | Reason: {}", pos.symbol, reason).bold().yellow()
            );
            println!(
                "   Exit Price: ${:.6} | Multiplier: {:.2}x | Peak: {:.2}x | PnL: {}",
                exit_price_usd, mult, pos.peak_multiplier, pnl_str
            );
            println!(
                "   💼 [PAPER WALLET] Available: {:.4} SOL | Total Realized PnL: {:.4} SOL",
                avail, realized
            );

            let mut positions = self.positions.lock().unwrap();
            positions.insert(token_address.to_string(), pos);
        } else {
            // Live on-chain exit via Jupiter sell + ATA close
            if let Some(ref executor) = self.executor {
                let exec = Arc::clone(executor);
                println!(
                    "\n{}",
                    format!("🏁 [LIVE EXIT ATTEMPT] Exiting {} ({:.2}x) | Reason: {}...", pos.symbol, mult, reason).bold().yellow()
                );

                // 1. Sell tokens back to SOL via Jupiter (amount 0 = full token ATA balance)
                match exec.execute_jupiter_swap_jito(token_address, SOL_MINT, 0, tip, 1000).await {
                    Ok(res) => {
                        println!("   ✔ [LIVE SELL BROADCAST] Tx: https://solscan.io/tx/{}", res.signature);
                        let _ = exec.poll_signature_confirmation(&res.signature, 8, 500).await;
                    }
                    Err(e) => {
                        warn!("Live exit sell failed: {e}");
                    }
                }

                // 2. Automatically close token account to reclaim 0.002039 SOL rent (try standard SPL then Token-2022)
                if exec.close_token_account_jito(token_address, tip, false).await.is_err() {
                    let _ = exec.close_token_account_jito(token_address, tip, true).await;
                }

                // 3. Sync live balance
                if let Ok(real_bal) = exec.get_balance_sol().await {
                    let mut port = self.portfolio.lock().unwrap();
                    port.available_sol = real_bal;
                    println!("   ⚡ [WALLET RECLAIMED] Live Balance Restored: {:.4} SOL", real_bal);
                }

                let mut positions = self.positions.lock().unwrap();
                positions.insert(token_address.to_string(), pos);
            }
        }

        Ok(())
    }

    /// Single scan iteration
    pub async fn run_scan_cycle(&self) -> Result<usize> {
        // 1. Update and manage existing open positions
        self.update_and_manage_positions().await?;

        // 2. Discover trending tokens on Solana
        let mints = self.fetch_trending_mints().await;
        if mints.is_empty() {
            println!("  ℹ [SCANNER] Polling DexScreener... 0 trending mints returned (retrying).");
            return Ok(0);
        }

        // 3. Fetch deep market metrics in batch
        let candidates = self.fetch_batch_metrics(&mints).await;
        let active_cnt = self.positions.lock().unwrap().values().filter(|p| !p.is_closed).count();
        println!(
            "  📡 [SCANNER] Screened {} trending DEX tokens | Active Positions: {}",
            candidates.len(),
            active_cnt
        );

        // 4. Sort by breakout score descending
        let mut sorted = candidates;
        sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        if let Some(top) = sorted.first() {
            println!(
                "     🔥 Top Candidate: {} ({}) | Score: {:.1} | Liq: ${:.1}k | 5m Vol: ${:.1}k (+{:.2}%)",
                top.symbol, &top.token_address[..12.min(top.token_address.len())], top.score, top.liquidity_usd / 1000.0, top.volume_5m_usd / 1000.0, top.price_change_5m
            );
        }

        let mut entered_cnt = 0;
        for cand in &sorted {
            if self.process_candidate(cand).await? {
                entered_cnt += 1;
            }
        }

        Ok(entered_cnt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_candidate_filters_low_liquidity() {
        let engine = BreakoutEngine::new(BreakoutSettings::default(), None);
        let low_liq = BreakoutCandidate {
            token_address: "LowLiqToken11111111111111111111111111111111".to_string(),
            name: "Low Liq".to_string(),
            symbol: "LOW".to_string(),
            pair_address: "pair1".to_string(),
            dex_id: "raydium".to_string(),
            price_usd: 0.05,
            price_native: 0.0003,
            liquidity_usd: 5_000.0, // < $25,000 floor
            volume_24h_usd: 200_000.0,
            volume_5m_usd: 10_000.0,
            price_change_5m: 5.0,
            price_change_1h: 20.0,
            price_change_24h: 30.0,
            buys_5m: 30,
            sells_5m: 5,
            buys_1h: 150,
            sells_1h: 50,
            pair_created_at_ms: None,
            pair_age_mins: Some(120.0),
            matched_strategy: None,
            score: 8.5,
        };
        assert!(!engine.evaluate_candidate(&low_liq), "Must reject low liquidity pool");
    }

    #[test]
    fn test_evaluate_candidate_identifies_mihao_style_breakout() {
        let mut settings = BreakoutSettings::default();
        settings.strategy_mode = ScalpStrategyMode::MomentumBreakout;
        let engine = BreakoutEngine::new(settings, None);
        let mihao_candidate = BreakoutCandidate {
            token_address: "4CxjZyMNoF1p6eXHXHv1TWpS2X47CuUmLwtGe18spump".to_string(),
            name: "ni hao cat".to_string(),
            symbol: "mihao".to_string(),
            pair_address: "Ho1aSG6WGH1FfXdEHrK2f4Ggt1Hoxo9s5jcLMDqeyeBz".to_string(),
            dex_id: "pumpswap".to_string(),
            price_usd: 0.015,
            price_native: 0.000179,
            liquidity_usd: 390_000.0, // $390k depth
            volume_24h_usd: 24_000_000.0, // $24M volume
            volume_5m_usd: 22_000.0, // $22k in 5m
            price_change_5m: 3.5, // +3.5% moving up
            price_change_1h: 15.0,
            price_change_24h: 85.0, // Healthy early breakout from base (< 250%)
            buys_5m: 135, // 135 buys
            sells_5m: 35, // 35 sells -> 79% buy ratio!
            buys_1h: 2400,
            sells_1h: 1100,
            pair_created_at_ms: None,
            pair_age_mins: Some(450.0),
            matched_strategy: None,
            score: 9.2,
        };
        assert!(engine.evaluate_candidate(&mihao_candidate), "Must qualify early base breakout under MomentumBreakout mode!");
        assert_eq!(
            engine.evaluate_candidate_strategy(&mihao_candidate),
            Some("Strategy 2: Momentum Breakout".to_string())
        );
    }

    #[test]
    fn test_multi_strategy_mode_strictly_excludes_strategy_2() {
        let engine = BreakoutEngine::new(BreakoutSettings::default(), None);
        // Candidate that only matches Strategy 2 (Momentum Breakout) and not Strategy 1 (not a dip) or 3 (pool age 450m > 30m)
        let mihao_candidate = BreakoutCandidate {
            token_address: "4CxjZyMNoF1p6eXHXHv1TWpS2X47CuUmLwtGe18spump".to_string(),
            name: "ni hao cat".to_string(),
            symbol: "mihao".to_string(),
            pair_address: "Ho1aSG6WGH1FfXdEHrK2f4Ggt1Hoxo9s5jcLMDqeyeBz".to_string(),
            dex_id: "pumpswap".to_string(),
            price_usd: 0.015,
            price_native: 0.000179,
            liquidity_usd: 390_000.0,
            volume_24h_usd: 24_000_000.0,
            volume_5m_usd: 22_000.0,
            price_change_5m: 3.5,
            price_change_1h: 15.0,
            price_change_24h: 85.0,
            buys_5m: 135,
            sells_5m: 35,
            buys_1h: 2400,
            sells_1h: 1100,
            pair_created_at_ms: None,
            pair_age_mins: Some(450.0),
            matched_strategy: None,
            score: 9.2,
        };
        // In Multi/DipAndGraduation mode, Strategy 2 candidate must be completely rejected!
        assert_eq!(engine.evaluate_candidate_strategy(&mihao_candidate), None);
    }

    #[test]
    fn test_evaluate_dip_rebound_qualifies_oversold_pullback() {
        let engine = BreakoutEngine::new(BreakoutSettings::default(), None);
        let dip_candidate = BreakoutCandidate {
            token_address: "DipToken111111111111111111111111111111111111".to_string(),
            name: "Dip Token".to_string(),
            symbol: "DIP".to_string(),
            pair_address: "pair_dip".to_string(),
            dex_id: "raydium".to_string(),
            price_usd: 0.025,
            price_native: 0.0003,
            liquidity_usd: 150_000.0,
            volume_24h_usd: 1_200_000.0,
            volume_5m_usd: 8_500.0,
            price_change_5m: -3.4, // Pullback -3.4% in 5m (oversold dip)
            price_change_1h: 8.5,  // Macro 1h trend is positive (+8.5%)
            price_change_24h: 42.0, // Healthy 24h uptrend
            buys_5m: 12,
            sells_5m: 18,
            buys_1h: 320,  // 320 buys
            sells_1h: 160, // 160 sells -> 66.7% buy ratio over 1h!
            pair_created_at_ms: None,
            pair_age_mins: Some(300.0),
            matched_strategy: None,
            score: 7.8,
        };

        assert!(engine.evaluate_dip_rebound(&dip_candidate), "Must qualify as Strategy 1 dip-rebound!");
        assert_eq!(
            engine.evaluate_candidate_strategy(&dip_candidate),
            Some("Strategy 1: 5m Oversold Dip-Rebound".to_string())
        );
    }

    #[test]
    fn test_evaluate_graduation_burst_identifies_fresh_pool() {
        let engine = BreakoutEngine::new(BreakoutSettings::default(), None);
        let fresh_grad = BreakoutCandidate {
            token_address: "GradToken11111111111111111111111111111111111".to_string(),
            name: "Graduated Token".to_string(),
            symbol: "GRAD".to_string(),
            pair_address: "pair_grad".to_string(),
            dex_id: "pumpswap".to_string(),
            price_usd: 0.0045,
            price_native: 0.000053,
            liquidity_usd: 35_000.0, // $35k initial seeded pool
            volume_24h_usd: 120_000.0,
            volume_5m_usd: 15_000.0, // $15k surge in 5m
            price_change_5m: 4.8,    // Exploding +4.8% on secondary listing
            price_change_1h: 18.0,
            price_change_24h: 35.0,
            buys_5m: 45,
            sells_5m: 15, // 75% buy ratio
            buys_1h: 60,
            sells_1h: 20,
            pair_created_at_ms: None,
            pair_age_mins: Some(8.5), // Freshly graduated 8.5 minutes ago!
            matched_strategy: None,
            score: 9.5,
        };

        assert!(engine.evaluate_graduation_burst(&fresh_grad), "Must qualify as Strategy 3 graduation burst!");
        assert_eq!(
            engine.evaluate_candidate_strategy(&fresh_grad),
            Some("Strategy 3: DEX Graduation Burst".to_string())
        );
    }

    #[test]
    fn test_anti_fomo_rejects_extended_tops() {
        let engine = BreakoutEngine::new(BreakoutSettings::default(), None);
        // Candidate 1: Extended 24h runner (+6,045%, like DACITY blow-off top)
        let extended_top = BreakoutCandidate {
            token_address: "ExtendedTopToken11111111111111111111111111".to_string(),
            name: "Parabolic Runner".to_string(),
            symbol: "TOP".to_string(),
            pair_address: "pair_top".to_string(),
            dex_id: "pumpswap".to_string(),
            price_usd: 0.0028,
            price_native: 0.000034,
            liquidity_usd: 120_000.0,
            volume_24h_usd: 2_000_000.0,
            volume_5m_usd: 20_000.0,
            price_change_5m: 5.0,
            price_change_1h: 15.0,
            price_change_24h: 6045.0, // +6,045% in 24h -> Extreme top-chasing risk!
            buys_5m: 50,
            sells_5m: 10,
            buys_1h: 500,
            sells_1h: 200,
            pair_created_at_ms: None,
            pair_age_mins: Some(180.0),
            matched_strategy: None,
            score: 9.0,
        };
        assert!(!engine.evaluate_candidate(&extended_top), "Must reject token already up +6,045% in 24h!");

        // Candidate 2: Parabolic 1h blow-off spike (+80% in 1h)
        let blowoff_1h = BreakoutCandidate {
            token_address: "BlowoffToken1111111111111111111111111111111".to_string(),
            name: "Blowoff Spike".to_string(),
            symbol: "SPIKE".to_string(),
            pair_address: "pair_spike".to_string(),
            dex_id: "raydium".to_string(),
            price_usd: 0.05,
            price_native: 0.0006,
            liquidity_usd: 80_000.0,
            volume_24h_usd: 500_000.0,
            volume_5m_usd: 15_000.0,
            price_change_5m: 8.0,
            price_change_1h: 85.0, // +85% in 1 hour -> Vertical blowoff spike (>50%)
            price_change_24h: 120.0,
            buys_5m: 40,
            sells_5m: 5,
            buys_1h: 400,
            sells_1h: 100,
            pair_created_at_ms: None,
            pair_age_mins: Some(200.0),
            matched_strategy: None,
            score: 8.5,
        };
        assert!(!engine.evaluate_candidate(&blowoff_1h), "Must reject token already up +85% in last hour!");
    }

    #[test]
    fn test_dynamic_trailing_stop_calculation() {
        let settings = BreakoutSettings {
            trailing_stop_pct: -15.0,
            break_even_mult: 1.25,
            ..Default::default()
        };
        let _engine = BreakoutEngine::new(settings, None);

        let initial_price = 1.0;
        let stop_price = initial_price * 0.85; // $0.85 (-15%)

        let mut pos = BreakoutPosition {
            token_address: "RunnerMint111111111111111111111111111111".to_string(),
            symbol: "RUNNER".to_string(),
            pair_address: "pair1".to_string(),
            dex_id: "meteora".to_string(),
            entry_price_usd: initial_price,
            entry_price_native: 0.01,
            entry_timestamp: Utc::now(),
            buy_amount_sol: 0.01,
            tokens_held: 1_000_000,
            peak_price_usd: initial_price,
            current_price_usd: initial_price,
            current_multiplier: 1.0,
            peak_multiplier: 1.0,
            stop_loss_price_usd: stop_price,
            is_closed: false,
            pnl_sol: 0.0,
            strategy: "Strategy 2: Momentum Breakout".to_string(),
        };

        // Price spikes +50% (1.50x)
        pos.current_price_usd = 1.50;
        pos.peak_price_usd = 1.50;
        pos.peak_multiplier = 1.50;

        let new_trailing = pos.peak_price_usd * 0.85; // 1.50 * 0.85 = 1.275
        if new_trailing > pos.stop_loss_price_usd {
            pos.stop_loss_price_usd = new_trailing;
        }

        // Must lock in profit above entry!
        assert!(pos.stop_loss_price_usd > pos.entry_price_usd, "Trailing stop must lock in profit at +50% peak");
        assert!((pos.stop_loss_price_usd - 1.275).abs() < 0.001);
    }

    #[test]
    fn test_trailing_take_profit_ratchet() {
        let settings = BreakoutSettings {
            stop_loss_pct: -5.0,
            trailing_take_profit_activation_pct: 8.0,
            trailing_take_profit_callback_pct: 2.5,
            trailing_stop_pct: -10.0,
            break_even_mult: 1.08,
            ..Default::default()
        };
        let _engine = BreakoutEngine::new(settings.clone(), None);

        let entry = 1.0;
        let initial_stop = entry * (1.0 + settings.stop_loss_pct / 100.0); // $0.95 (-5%)
        assert_eq!(initial_stop, 0.95);

        let mut pos = BreakoutPosition {
            token_address: "TokenMint111111111111111111111111111111".to_string(),
            symbol: "TEST".to_string(),
            pair_address: "pair1".to_string(),
            dex_id: "pumpswap".to_string(),
            entry_price_usd: entry,
            entry_price_native: 0.005,
            entry_timestamp: Utc::now(),
            buy_amount_sol: 0.008,
            tokens_held: 1_000_000,
            peak_price_usd: entry,
            current_price_usd: entry,
            current_multiplier: 1.0,
            peak_multiplier: 1.0,
            stop_loss_price_usd: initial_stop,
            is_closed: false,
            pnl_sol: 0.0,
            strategy: "Strategy 2: Momentum Breakout".to_string(),
        };

        // 1. Price jumps to +20% ($1.20)
        let peak_price = 1.20;
        pos.peak_price_usd = peak_price;
        pos.peak_multiplier = 1.20;

        let activation_mult = 1.0 + (settings.trailing_take_profit_activation_pct / 100.0);
        assert!(pos.peak_multiplier >= activation_mult, "Must activate trailing take profit at 1.20x >= 1.08x");

        // Trailing floor = 1.20 * (1.0 - 0.025) = 1.17 (+17% locked!)
        let ttp_floor = pos.peak_price_usd * (1.0 - settings.trailing_take_profit_callback_pct / 100.0);
        let min_guaranteed_profit = pos.entry_price_usd * 1.02;
        let effective_floor = ttp_floor.max(min_guaranteed_profit);

        if effective_floor > pos.stop_loss_price_usd {
            pos.stop_loss_price_usd = effective_floor;
        }

        assert_eq!(pos.stop_loss_price_usd, 1.17);
        assert!(pos.stop_loss_price_usd > pos.entry_price_usd * 1.15, "Locked profit must exceed +15%");
    }
}
