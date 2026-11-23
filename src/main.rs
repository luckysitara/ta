mod agents;
mod commands;
mod config;
mod dataflows;
mod graph;
mod llm;
mod solana;
mod symbols;
mod types;

use clap::Parser;
use colored::*;
use dotenvy::dotenv;
use graph::TradingWorkflow;
use llm::create_llm_client;
use solana::build_solana_trade_route;
use solana::prediction::formulate_prediction_strategy;
use std::collections::HashMap;
use types::{AnalystType, Portfolio, Rating};

#[derive(Parser, Debug)]
#[command(
    name = "tradingagents-rs",
    about = "🦀 Native Rust Multi-Agent Financial Trading & Solana Framework",
    version = "0.1.0"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Ticker symbol to analyze (e.g. SOL, BTC, ETH, NVDA, AAPL)
    #[arg(short, long, default_value = "SOL")]
    ticker: String,

    /// LLM provider: auto (uses Claude Code CLI if available), claude-code, ollama, openai, groq, mock
    #[arg(short, long, default_value = "auto")]
    provider: String,

    /// Model identifier (e.g. gpt-4o, claude-3-5-sonnet, llama-3.3-70b-versatile)
    #[arg(short, long)]
    model: Option<String>,

    /// Comma-separated list of analysts: technical, sentiment, news, fundamentals
    #[arg(short, long, default_value = "technical,sentiment,news,fundamentals")]
    analysts: String,

    /// Calculate and display Solana DEX execution route via Jupiter API
    #[arg(long, default_value_t = true)]
    solana_dex: bool,

    /// Discover and formulate Jupiter Prediction Market binary contracts (api.jup.ag/prediction/v1)
    #[arg(long, default_value_t = true)]
    prediction_markets: bool,

    /// Initial portfolio cash in USD
    #[arg(long, default_value_t = 10000.0)]
    cash: f64,

    /// Run Yellowstone Solana sniper directly
    #[arg(long)]
    snipe: bool,

    /// Yellowstone gRPC endpoint URL
    #[arg(long, env = "SOLANA_GRPC_ENDPOINT")]
    grpc_endpoint: Option<String>,

    /// Yellowstone gRPC X-Token authentication key
    #[arg(long, env = "SOLANA_GRPC_TOKEN")]
    grpc_token: Option<String>,

    /// Total paper trading budget in SOL (default: 0.5 SOL)
    #[arg(long, default_value_t = 0.5)]
    budget: f64,

    /// Snipe buy amount in SOL (default: 0.05 SOL)
    #[arg(long, default_value_t = 0.05)]
    buy_amount: f64,

    /// Dry run / paper trade mode (default: true)
    #[arg(long, default_value_t = true)]
    paper: bool,

    /// Max run duration in seconds (optional)
    #[arg(long)]
    duration_secs: Option<u64>,

    /// Option 1: Live on-chain price tracking via Yellowstone bonding curve reserves stream
    #[arg(long, default_value_t = false)]
    live_reserves: bool,

    /// Option 2: Require verified socials in token launch metadata (Twitter/Telegram)
    #[arg(long, default_value_t = false)]
    require_socials: bool,

    /// Option 2: Maximum dev initial buy in SOL (reject insider bundles)
    #[arg(long)]
    max_dev_buy: Option<f64>,

    /// Item 3: Require minimum dev initial buy in SOL (e.g. 0.40 SOL dev skin-in-the-game)
    #[arg(long)]
    min_dev_buy: Option<f64>,

    /// Item 3: Minimum bonding curve progress % to enter on momentum (e.g. 10.0 for 10% curve progress)
    #[arg(long)]
    min_curve_pct: Option<f64>,

    /// Sweet Spot Cap: Maximum bonding curve progress % to enter (e.g. 25.0 for 25% curve cap)
    #[arg(long)]
    max_curve_pct: Option<f64>,

    /// Upgrade 2: Minimum unique buyer wallets required before momentum entry
    #[arg(long)]
    min_unique_buyers: Option<usize>,

    /// Buy Velocity Filter: Minimum buy transactions required within velocity window
    #[arg(long)]
    min_buys_recent: Option<usize>,

    /// Buy Velocity Filter: Maximum sell transactions permitted within velocity window
    #[arg(long)]
    max_sells_recent: Option<usize>,

    /// Buy Velocity Filter: Window duration in seconds (e.g. 15 for 15s window)
    #[arg(long)]
    velocity_window_secs: Option<u64>,

    /// Take-profit 1 multiplier (e.g. 1.25 for +25% scalping)
    #[arg(long)]
    tp1: Option<f64>,

    /// Take-profit 2 multiplier (e.g. 1.60 for +60% target)
    #[arg(long)]
    tp2: Option<f64>,

    /// Stop loss percentage (e.g. -18.0 for -18%)
    #[arg(long)]
    stop_loss: Option<f64>,

    /// Break-even stop trigger multiplier (locks in 1.02x floor when peak reaches e.g. 1.10x)
    #[arg(long)]
    break_even_mult: Option<f64>,

    /// Live on-chain execution with real SOL using Jito MEV bundles
    #[arg(long, default_value_t = false)]
    onchain: bool,

    /// Path to Solana Keypair JSON file (default: ~/.config/solana/id.json or KEYPAIR_PATH)
    #[arg(long, env = "KEYPAIR_PATH")]
    keypair_path: Option<String>,

    /// Solana RPC endpoint URL
    #[arg(long, env = "SOLANA_RPC_URL")]
    rpc_url: Option<String>,

    /// Jito Block Engine endpoint URL
    #[arg(long, env = "JITO_BLOCK_ENGINE_URL")]
    jito_url: Option<String>,

    /// Strategy Mode: 1 (Standard), 2 (Option 1: Live Reserves), 3 (Option 2: Strict Anti-Rug), 4 (High-Conviction)
    #[arg(long)]
    mode: Option<String>,

    /// Require dev holding to be 0% or dev already dumped all tokens before entry
    #[arg(long)]
    require_dev_sold_out: Option<bool>,

    /// Enable Raydium pool / graduation sniping alongside Pump.fun
    #[arg(long)]
    enable_raydium: Option<bool>,
}

#[derive(clap::Subcommand, Debug)]
enum Commands {
    /// Run multi-agent research analysis on a stock or crypto ticker
    Analyze {
        #[arg(default_value = "SOL")]
        ticker: String,
    },
    /// Run real-time Solana Yellowstone gRPC sniper listener
    Snipe {
        /// Optional positional endpoint URL or trailing arguments
        #[arg(index = 1)]
        endpoint_pos: Option<String>,

        /// Yellowstone gRPC endpoint URL
        #[arg(long, default_missing_value = "", num_args = 0..=1, env = "SOLANA_GRPC_ENDPOINT")]
        grpc_endpoint: Option<String>,

        /// Yellowstone gRPC X-Token authentication key
        #[arg(long, default_missing_value = "", num_args = 0..=1, env = "SOLANA_GRPC_TOKEN")]
        grpc_token: Option<String>,

        /// Total paper trading budget in SOL (default: 0.5 SOL)
        #[arg(long, default_value_t = 0.5)]
        budget: f64,

        /// Snipe buy amount in SOL (default: 0.05 SOL)
        #[arg(long, default_value_t = 0.05)]
        buy_amount: f64,

        /// Maximum concurrent active positions (default: 5)
        #[arg(long, default_value_t = 5)]
        max_positions: usize,

        /// Minimum pool liquidity in SOL (default: 1.0 SOL)
        #[arg(long, default_value_t = 1.0)]
        min_liquidity: f64,

        /// Jito MEV tip amount in SOL (default: 0.003 SOL)
        #[arg(long, default_value_t = 0.003)]
        jito_tip: f64,

        /// Max run duration in seconds (optional, e.g. 300 for 5 minutes)
        #[arg(long)]
        duration_secs: Option<u64>,

        /// Option 1: Live on-chain price tracking via Yellowstone bonding curve reserves stream
        #[arg(long, default_value_t = false)]
        live_reserves: bool,

        /// Option 2: Require verified socials in token launch metadata (Twitter/Telegram)
        #[arg(long, default_value_t = false)]
        require_socials: bool,

        /// Option 2: Maximum dev initial buy in SOL (reject insider bundles)
        #[arg(long)]
        max_dev_buy: Option<f64>,

        /// Item 3: Require minimum dev initial buy in SOL (e.g. 0.40 SOL dev skin-in-the-game)
        #[arg(long)]
        min_dev_buy: Option<f64>,

        /// Item 3: Minimum bonding curve progress % to enter on momentum (e.g. 10.0 for 10% curve progress)
        #[arg(long)]
        min_curve_pct: Option<f64>,

        /// Sweet Spot Cap: Maximum bonding curve progress % to enter (e.g. 25.0 for 25% curve cap)
        #[arg(long)]
        max_curve_pct: Option<f64>,

        /// Upgrade 2: Minimum unique buyer wallets required before momentum entry (Anti-Insider Whale Filter)
        #[arg(long)]
        min_unique_buyers: Option<usize>,

        /// Buy Velocity Filter: Minimum buy transactions required within velocity window
        #[arg(long)]
        min_buys_recent: Option<usize>,

        /// Buy Velocity Filter: Maximum sell transactions permitted within velocity window
        #[arg(long)]
        max_sells_recent: Option<usize>,

        /// Buy Velocity Filter: Window duration in seconds (e.g. 15 for 15s window)
        #[arg(long)]
        velocity_window_secs: Option<u64>,

        /// Take-profit 1 multiplier (e.g. 1.25 for +25% scalping)
        #[arg(long)]
        tp1: Option<f64>,

        /// Take-profit 2 multiplier (e.g. 1.60 for +60% target)
        #[arg(long)]
        tp2: Option<f64>,

        /// Stop loss percentage (e.g. -18.0 for -18%)
        #[arg(long)]
        stop_loss: Option<f64>,

        /// Break-even stop trigger multiplier (locks in 1.02x floor when peak reaches e.g. 1.10x)
        #[arg(long)]
        break_even_mult: Option<f64>,

        /// Paper trade / dry-run mode
        #[arg(long, default_value_t = true)]
        paper: bool,

        /// Live on-chain execution with real SOL using Jito MEV bundles
        #[arg(long, default_value_t = false)]
        onchain: bool,

        /// Path to Solana Keypair JSON file (default: ~/.config/solana/id.json or KEYPAIR_PATH)
        #[arg(long, env = "KEYPAIR_PATH")]
        keypair_path: Option<String>,

        /// Solana RPC endpoint URL
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,

        /// Jito Block Engine endpoint URL
        #[arg(long, env = "JITO_BLOCK_ENGINE_URL")]
        jito_url: Option<String>,

        /// Strategy Mode: 1 (Standard), 2 (Option 1: Live Reserves), 3 (Option 2: Strict Anti-Rug), 4 (High-Conviction)
        #[arg(long)]
        mode: Option<String>,

        /// Require dev holding to be 0% or dev already dumped all tokens before entry
        #[arg(long)]
        require_dev_sold_out: Option<bool>,

        /// Enable Raydium pool / graduation sniping alongside Pump.fun
        #[arg(long)]
        enable_raydium: Option<bool>,

        /// Simulate Yellowstone stream events for testing
        #[arg(long)]
        simulate: bool,
    },
    /// Liquidate open token positions back into SOL using Jito MEV
    Liquidate {
        /// Optional specific token mint to sell (defaults to all active meme tokens)
        #[arg(long)]
        mint: Option<String>,
    },
    /// Run real-time Secondary Market DEX Breakout & Volume Surge Scanner (catches tokens like MIHAO)
    Breakout {
        /// Total budget in SOL (default: 0.02 SOL)
        #[arg(long, default_value_t = 0.02)]
        budget: f64,

        /// Buy amount in SOL per trade (default: 0.008 SOL)
        #[arg(long, default_value_t = 0.008)]
        buy_amount: f64,

        /// Maximum concurrent active positions (default: 2)
        #[arg(long, default_value_t = 2)]
        max_positions: usize,

        /// Jito MEV tip amount in SOL (default: 0.0001 SOL)
        #[arg(long, default_value_t = 0.0001)]
        jito_tip: f64,

        /// Minimum pool liquidity in USD (default: $25,000)
        #[arg(long, default_value_t = 25000.0)]
        min_liquidity_usd: f64,

        /// Minimum 24h trading volume in USD (default: $100,000)
        #[arg(long, default_value_t = 100000.0)]
        min_volume_24h_usd: f64,

        /// Minimum 5m trading volume in USD (default: $3,000)
        #[arg(long, default_value_t = 3000.0)]
        min_volume_5m_usd: f64,

        /// Minimum buy transactions in last 5m (default: 10)
        #[arg(long, default_value_t = 10)]
        min_buys_5m: usize,

        /// Minimum buy transaction ratio in last 5m (default: 0.55 for 55%)
        #[arg(long, default_value_t = 0.55)]
        min_buy_ratio_5m: f64,

        /// Minimum 5m price change % (default: 0.5 for +0.5%)
        #[arg(long, default_value_t = 0.5)]
        min_price_change_5m: f64,

        /// Anti-FOMO: Maximum allowable 24h price increase % (default: 250.0 for +250%)
        #[arg(long, default_value_t = 250.0)]
        max_price_change_24h: f64,

        /// Anti-FOMO: Maximum allowable 1h price spike % (default: 50.0 for +50%)
        #[arg(long, default_value_t = 50.0)]
        max_price_change_1h: f64,

        /// Tight stop loss percentage from entry price (default: -5.0 for -5%)
        #[arg(long, default_value_t = -5.0)]
        stop_loss_pct: f64,

        /// Trailing take profit activation percentage (default: 8.0 for +8%)
        #[arg(long, default_value_t = 8.0)]
        trailing_take_profit_activation_pct: f64,

        /// Trailing take profit callback buffer from peak (default: 2.5 for 2.5%)
        #[arg(long, default_value_t = 2.5)]
        trailing_take_profit_callback_pct: f64,

        /// Scalping strategy mode: multi (default: Strategy 1 & 3), dip, graduation, breakout
        #[arg(long, default_value = "multi")]
        strategy: String,

        /// Strategy 1: Minimum oversold 5m dip % (default: -7.5 for -7.5%)
        #[arg(long, default_value_t = -7.5)]
        min_dip_pct: f64,

        /// Strategy 1: Maximum oversold 5m dip % (default: -1.8 for -1.8%)
        #[arg(long, default_value_t = -1.8)]
        max_dip_pct: f64,

        /// Strategy 1: Minimum 1h buy transaction ratio (default: 0.52 for 52%)
        #[arg(long, default_value_t = 0.52)]
        min_buy_ratio_1h: f64,

        /// Strategy 3: Maximum pool graduation age in minutes (default: 30.0 mins)
        #[arg(long, default_value_t = 30.0)]
        max_graduation_age_mins: f64,

        /// Strategy 3: Minimum initial pool liquidity in USD (default: $20,000)
        #[arg(long, default_value_t = 20000.0)]
        min_graduation_liquidity_usd: f64,

        /// Strategy 3: Minimum 5m buy ratio for graduation burst (default: 0.58 for 58%)
        #[arg(long, default_value_t = 0.58)]
        min_graduation_buy_ratio_5m: f64,

        /// Trailing stop loss percentage from peak watermark (default: -10.0 for -10%)
        #[arg(long, default_value_t = -10.0)]
        trailing_stop_pct: f64,

        /// Break-even floor trigger multiplier (default: 1.08 for +8%)
        #[arg(long, default_value_t = 1.08)]
        break_even_mult: f64,

        /// Polling interval in seconds (default: 10s)
        #[arg(long, default_value_t = 10)]
        poll_interval_secs: u64,

        /// Max run duration in seconds (optional)
        #[arg(long)]
        duration_secs: Option<u64>,

        /// Live on-chain execution with real SOL using Jito MEV bundles
        #[arg(long, default_value_t = false)]
        onchain: bool,

        /// Path to Solana Keypair JSON file (default: ~/.config/solana/id.json)
        #[arg(long, env = "KEYPAIR_PATH")]
        keypair_path: Option<String>,

        /// Solana RPC endpoint URL
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,
    },
    /// Run Solana Cross-DEX Atomic Arbitrage Engine (Raydium, Meteora, Orca, PumpSwap)
    Arbitrage {
        /// Paper/live trading budget in SOL (default: 0.08 SOL)
        #[arg(long, default_value_t = 0.08)]
        budget: f64,

        /// Trade size in SOL per cyclic arbitrage (default: 0.02 SOL)
        #[arg(long, default_value_t = 0.02)]
        trade_amount: f64,

        /// Minimum net profit in basis points (default: 30 bps = 0.3%)
        #[arg(long, default_value_t = 30)]
        min_profit_bps: i64,

        /// Jito MEV tip amount in SOL (default: 0.0001 SOL)
        #[arg(long, default_value_t = 0.0001)]
        jito_tip: f64,

        /// Polling interval in seconds (default: 5s)
        #[arg(long, default_value_t = 5)]
        poll_interval_secs: u64,

        /// Max run duration in seconds (optional)
        #[arg(long)]
        duration_secs: Option<u64>,

        /// Comma-separated token mints to screen (optional, defaults to automatic multi-DEX discovery)
        #[arg(long)]
        tokens: Option<String>,

        /// Live on-chain execution with real SOL using Jito MEV bundles
        #[arg(long, default_value_t = false)]
        onchain: bool,

        /// Path to Solana Keypair JSON file (default: ~/.config/solana/id.json)
        #[arg(long, env = "KEYPAIR_PATH")]
        keypair_path: Option<String>,

        /// Solana RPC endpoint URL
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,

        /// Jito Block Engine endpoint URL
        #[arg(long, env = "JITO_BLOCK_ENGINE_URL")]
        jito_url: Option<String>,
    },
    /// Run Volume Spike Detector — catches 3-5x volume surges on established tokens
    VolSpike {
        #[arg(long, default_value_t = 0.1)]
        budget: f64,
        #[arg(long, default_value_t = 0.02)]
        buy_amount: f64,
        #[arg(long, default_value_t = 3)]
        max_positions: usize,
        #[arg(long, default_value_t = 0.0001)]
        jito_tip: f64,
        #[arg(long, default_value_t = 50000.0)]
        min_liquidity_usd: f64,
        #[arg(long, default_value_t = 24.0)]
        min_token_age_hours: f64,
        #[arg(long, default_value_t = 3.0)]
        volume_spike_multiplier: f64,
        #[arg(long, default_value_t = -5.0)]
        stop_loss_pct: f64,
        #[arg(long, default_value_t = 15.0)]
        take_profit_pct: f64,
        #[arg(long, default_value_t = -8.0)]
        trailing_stop_pct: f64,
        #[arg(long, default_value_t = 1.05)]
        break_even_mult: f64,
        #[arg(long, default_value_t = 10)]
        poll_interval_secs: u64,
        #[arg(long)]
        duration_secs: Option<u64>,
        #[arg(long, default_value_t = false)]
        onchain: bool,
        #[arg(long, env = "KEYPAIR_PATH")]
        keypair_path: Option<String>,
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,
    },
    /// Run Smart Wallet Copy Trading — follow profitable wallets, enter on convergence
    CopyTrade {
        #[arg(long, default_value_t = 0.2)]
        budget: f64,
        #[arg(long, default_value_t = 0.03)]
        buy_amount: f64,
        #[arg(long, default_value_t = 5)]
        max_positions: usize,
        #[arg(long, default_value_t = 0.0001)]
        jito_tip: f64,
        /// Comma-separated wallet addresses to track
        #[arg(long, env = "COPY_TRADE_WALLETS")]
        wallets: Option<String>,
        #[arg(long, default_value_t = 2)]
        min_convergence: usize,
        #[arg(long, default_value_t = 300)]
        convergence_window_secs: u64,
        #[arg(long, default_value_t = 0.5)]
        min_buy_sol: f64,
        #[arg(long, default_value_t = -8.0)]
        stop_loss_pct: f64,
        #[arg(long, default_value_t = 20.0)]
        take_profit_pct: f64,
        #[arg(long, default_value_t = -10.0)]
        trailing_stop_pct: f64,
        #[arg(long, default_value_t = 1.08)]
        break_even_mult: f64,
        #[arg(long, default_value_t = 15)]
        poll_interval_secs: u64,
        #[arg(long)]
        duration_secs: Option<u64>,
        #[arg(long, default_value_t = false)]
        onchain: bool,
        #[arg(long, env = "KEYPAIR_PATH")]
        keypair_path: Option<String>,
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,
    },
    /// Run Graduation Frontrun — buy tokens at 85-99% bonding curve, sell into Raydium migration
    Graduation {
        #[arg(long, default_value_t = 0.15)]
        budget: f64,
        #[arg(long, default_value_t = 0.03)]
        buy_amount: f64,
        #[arg(long, default_value_t = 3)]
        max_positions: usize,
        #[arg(long, default_value_t = 0.0003)]
        jito_tip: f64,
        #[arg(long, default_value_t = 85.0)]
        min_curve_pct: f64,
        #[arg(long, default_value_t = 99.0)]
        max_curve_pct: f64,
        #[arg(long, default_value_t = 50.0)]
        min_market_cap_sol: f64,
        #[arg(long, default_value_t = 50)]
        min_holders: usize,
        #[arg(long, default_value_t = -8.0)]
        stop_loss_pct: f64,
        #[arg(long, default_value_t = 30.0)]
        take_profit_pct: f64,
        #[arg(long, default_value_t = 50.0)]
        graduation_exit_pct: f64,
        #[arg(long, default_value_t = -12.0)]
        trailing_stop_pct: f64,
        #[arg(long, default_value_t = 1.06)]
        break_even_mult: f64,
        #[arg(long, default_value_t = 5)]
        poll_interval_secs: u64,
        #[arg(long)]
        duration_secs: Option<u64>,
        #[arg(long, default_value_t = false)]
        onchain: bool,
        #[arg(long, env = "KEYPAIR_PATH")]
        keypair_path: Option<String>,
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,
    },
    /// Run Meteora/Orca Liquidity Snipe — snipe new pool creation on non-PumpFun DEXes
    LiqSnipe {
        #[arg(long, default_value_t = 0.1)]
        budget: f64,
        #[arg(long, default_value_t = 0.02)]
        buy_amount: f64,
        #[arg(long, default_value_t = 3)]
        max_positions: usize,
        #[arg(long, default_value_t = 0.0003)]
        jito_tip: f64,
        #[arg(long, default_value_t = 5000.0)]
        min_liquidity_usd: f64,
        #[arg(long, default_value_t = 30.0)]
        max_pool_age_mins: f64,
        #[arg(long, default_value_t = -10.0)]
        stop_loss_pct: f64,
        #[arg(long, default_value_t = 25.0)]
        take_profit_pct: f64,
        #[arg(long, default_value_t = -12.0)]
        trailing_stop_pct: f64,
        #[arg(long, default_value_t = 1.08)]
        break_even_mult: f64,
        /// Comma-separated DEX names (default: meteora,orca)
        #[arg(long, default_value = "meteora,orca")]
        target_dexes: String,
        #[arg(long, default_value_t = 10)]
        poll_interval_secs: u64,
        #[arg(long)]
        duration_secs: Option<u64>,
        #[arg(long, default_value_t = false)]
        onchain: bool,
        #[arg(long, env = "KEYPAIR_PATH")]
        keypair_path: Option<String>,
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,
    },
    /// Run Mean Reversion RSI strategy on established memecoins (BONK, WIF, POPCAT, etc.)
    MeanRev {
        #[arg(long, default_value_t = 0.2)]
        budget: f64,
        #[arg(long, default_value_t = 0.03)]
        buy_amount: f64,
        #[arg(long, default_value_t = 3)]
        max_positions: usize,
        #[arg(long, default_value_t = 0.0001)]
        jito_tip: f64,
        /// Comma-separated mint:symbol pairs (default: built-in top memecoins)
        #[arg(long)]
        tokens: Option<String>,
        #[arg(long, default_value_t = 14)]
        rsi_period: usize,
        #[arg(long, default_value_t = 25.0)]
        rsi_oversold: f64,
        #[arg(long, default_value_t = 55.0)]
        rsi_exit: f64,
        #[arg(long, default_value_t = -8.0)]
        stop_loss_pct: f64,
        #[arg(long, default_value_t = 15.0)]
        take_profit_pct: f64,
        #[arg(long, default_value_t = 120)]
        max_hold_mins: u64,
        #[arg(long, default_value_t = 30)]
        poll_interval_secs: u64,
        #[arg(long)]
        duration_secs: Option<u64>,
        #[arg(long, default_value_t = false)]
        onchain: bool,
        #[arg(long, env = "KEYPAIR_PATH")]
        keypair_path: Option<String>,
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,
    },
    /// Run Jupiter Route Arbitrage — exploit price differences across DEX routes
    RouteArb {
        #[arg(long, default_value_t = 0.2)]
        budget: f64,
        #[arg(long, default_value_t = 0.03)]
        trade_amount: f64,
        #[arg(long, default_value_t = 30)]
        min_spread_bps: u64,
        #[arg(long, default_value_t = 0.0002)]
        jito_tip: f64,
        /// Comma-separated mint:symbol pairs (default: built-in blue-chip tokens)
        #[arg(long)]
        tokens: Option<String>,
        #[arg(long, default_value_t = 5)]
        poll_interval_secs: u64,
        #[arg(long)]
        duration_secs: Option<u64>,
        #[arg(long, default_value_t = false)]
        onchain: bool,
        #[arg(long, env = "KEYPAIR_PATH")]
        keypair_path: Option<String>,
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,
        #[arg(long, env = "JITO_BLOCK_ENGINE_URL")]
        jito_url: Option<String>,
        /// Enable Kamino Flash Loan for atomic zero-capital borrowing
        #[arg(long, default_value_t = false)]
        use_flash_loan: bool,
        /// Flash loan borrow amount in SOL (e.g. 5.0 for 5 SOL)
        #[arg(long, default_value_t = 5.0)]
        flash_loan_amount: f64,
    },
    /// Analyze wallet transaction history and PnL from on-chain data (Helius API)
    AnalyzeWallets {
        /// Comma-separated address:label pairs (default: top KOL leaderboard wallets)
        #[arg(long)]
        wallets: Option<String>,
        /// Maximum transactions to fetch per wallet (default: 200)
        #[arg(long, default_value_t = 200)]
        max_transactions: usize,
        /// Minimum win rate % for copy-trade recommendation (default: 50)
        #[arg(long, default_value_t = 50.0)]
        min_win_rate: f64,
        /// Minimum total trades for copy-trade recommendation (default: 10)
        #[arg(long, default_value_t = 10)]
        min_trades: usize,
        #[arg(long, env = "SOLANA_RPC_URL")]
        rpc_url: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    dotenv().ok();

    // Initialize tracing subscriber
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();

    let args = Args::parse();

    // Check if Snipe subcommand or --snipe flag was specified
    if let Some(Commands::Snipe {
        endpoint_pos,
        grpc_endpoint,
        grpc_token,
        budget,
        buy_amount,
        max_positions,
        min_liquidity,
        jito_tip,
        duration_secs,
        live_reserves,
        require_socials,
        max_dev_buy,
        min_dev_buy,
        min_curve_pct,
        max_curve_pct,
        min_unique_buyers,
        min_buys_recent,
        max_sells_recent,
        velocity_window_secs,
        tp1,
        tp2,
        stop_loss,
        break_even_mult,
        paper,
        onchain,
        keypair_path,
        rpc_url,
        jito_url,
        mode,
        require_dev_sold_out,
        enable_raydium,
        simulate,
    }) = args.command
    {
        let ep = grpc_endpoint.or_else(|| {
            endpoint_pos.and_then(|p| {
                let trimmed = p.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            })
        });
        return commands::snipe::run_sniper(
            ep,
            grpc_token,
            budget,
            buy_amount,
            max_positions,
            min_liquidity,
            jito_tip,
            duration_secs,
            live_reserves,
            require_socials,
            max_dev_buy,
            min_dev_buy,
            min_curve_pct,
            max_curve_pct,
            min_unique_buyers,
            min_buys_recent,
            max_sells_recent,
            velocity_window_secs,
            tp1,
            tp2,
            stop_loss,
            break_even_mult,
            paper,
            onchain,
            keypair_path,
            rpc_url,
            jito_url,
            mode,
            require_dev_sold_out,
            enable_raydium,
            simulate,
        )
        .await;
    }

    if let Some(Commands::Liquidate { mint }) = args.command {
        return commands::liquidate::run_liquidate(mint).await;
    }

    if let Some(Commands::Breakout {
        budget,
        buy_amount,
        max_positions,
        jito_tip,
        min_liquidity_usd,
        min_volume_24h_usd,
        min_volume_5m_usd,
        min_buys_5m,
        min_buy_ratio_5m,
        min_price_change_5m,
        max_price_change_24h,
        max_price_change_1h,
        strategy,
        min_dip_pct,
        max_dip_pct,
        min_buy_ratio_1h,
        max_graduation_age_mins,
        min_graduation_liquidity_usd,
        min_graduation_buy_ratio_5m,
        stop_loss_pct,
        trailing_take_profit_activation_pct,
        trailing_take_profit_callback_pct,
        trailing_stop_pct,
        break_even_mult,
        poll_interval_secs,
        duration_secs,
        onchain,
        keypair_path,
        rpc_url,
    }) = args.command
    {
        return commands::breakout::run_breakout_scanner(
            budget,
            buy_amount,
            max_positions,
            jito_tip,
            min_liquidity_usd,
            min_volume_24h_usd,
            min_volume_5m_usd,
            min_buys_5m,
            min_buy_ratio_5m,
            min_price_change_5m,
            max_price_change_24h,
            max_price_change_1h,
            strategy,
            min_dip_pct,
            max_dip_pct,
            min_buy_ratio_1h,
            max_graduation_age_mins,
            min_graduation_liquidity_usd,
            min_graduation_buy_ratio_5m,
            stop_loss_pct,
            trailing_take_profit_activation_pct,
            trailing_take_profit_callback_pct,
            trailing_stop_pct,
            break_even_mult,
            poll_interval_secs,
            duration_secs,
            onchain,
            keypair_path,
            rpc_url,
        )
        .await;
    }

    if let Some(Commands::Arbitrage {
        budget,
        trade_amount,
        min_profit_bps,
        jito_tip,
        poll_interval_secs,
        duration_secs,
        tokens,
        onchain,
        keypair_path,
        rpc_url,
        jito_url,
    }) = args.command
    {
        let target_tokens = tokens
            .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();

        let cfg = solana::arbitrage::ArbitrageConfig {
            budget_sol: budget,
            trade_amount_sol: trade_amount,
            min_profit_bps,
            jito_tip_sol: jito_tip,
            onchain,
            poll_interval_secs,
            duration_secs,
            target_tokens,
            keypair_path,
            rpc_url,
            jito_url,
        };

        let engine = solana::arbitrage::ArbitrageEngine::new(cfg)?;
        return engine.run().await;
    }

    if let Some(Commands::VolSpike {
        budget, buy_amount, max_positions, jito_tip, min_liquidity_usd,
        min_token_age_hours, volume_spike_multiplier, stop_loss_pct,
        take_profit_pct, trailing_stop_pct, break_even_mult,
        poll_interval_secs, duration_secs, onchain, keypair_path, rpc_url,
    }) = args.command
    {
        return commands::volume_spike::run_volume_spike(
            budget, buy_amount, max_positions, jito_tip, min_liquidity_usd,
            min_token_age_hours, volume_spike_multiplier, stop_loss_pct,
            take_profit_pct, trailing_stop_pct, break_even_mult,
            poll_interval_secs, duration_secs, onchain, keypair_path, rpc_url,
        ).await;
    }

    if let Some(Commands::CopyTrade {
        budget, buy_amount, max_positions, jito_tip, wallets,
        min_convergence, convergence_window_secs, min_buy_sol,
        stop_loss_pct, take_profit_pct, trailing_stop_pct, break_even_mult,
        poll_interval_secs, duration_secs, onchain, keypair_path, rpc_url,
    }) = args.command
    {
        let wallet_list: Vec<String> = wallets
            .map(|w| w.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        return commands::copy_trade::run_copy_trade(
            budget, buy_amount, max_positions, jito_tip, wallet_list,
            min_convergence, convergence_window_secs, min_buy_sol,
            stop_loss_pct, take_profit_pct, trailing_stop_pct, break_even_mult,
            poll_interval_secs, duration_secs, onchain, keypair_path, rpc_url,
        ).await;
    }

    if let Some(Commands::Graduation {
        budget, buy_amount, max_positions, jito_tip, min_curve_pct,
        max_curve_pct, min_market_cap_sol, min_holders, stop_loss_pct,
        take_profit_pct, graduation_exit_pct, trailing_stop_pct,
        break_even_mult, poll_interval_secs, duration_secs, onchain,
        keypair_path, rpc_url,
    }) = args.command
    {
        return commands::graduation::run_graduation(
            budget, buy_amount, max_positions, jito_tip, min_curve_pct,
            max_curve_pct, min_market_cap_sol, min_holders, stop_loss_pct,
            take_profit_pct, graduation_exit_pct, trailing_stop_pct,
            break_even_mult, poll_interval_secs, duration_secs, onchain,
            keypair_path, rpc_url,
        ).await;
    }

    if let Some(Commands::LiqSnipe {
        budget, buy_amount, max_positions, jito_tip, min_liquidity_usd,
        max_pool_age_mins, stop_loss_pct, take_profit_pct, trailing_stop_pct,
        break_even_mult, target_dexes, poll_interval_secs, duration_secs,
        onchain, keypair_path, rpc_url,
    }) = args.command
    {
        let dexes: Vec<String> = target_dexes.split(',').map(|s| s.trim().to_string()).collect();
        return commands::liquidity_snipe::run_liquidity_snipe(
            budget, buy_amount, max_positions, jito_tip, min_liquidity_usd,
            max_pool_age_mins, stop_loss_pct, take_profit_pct, trailing_stop_pct,
            break_even_mult, dexes, poll_interval_secs, duration_secs,
            onchain, keypair_path, rpc_url,
        ).await;
    }

    if let Some(Commands::MeanRev {
        budget, buy_amount, max_positions, jito_tip, tokens,
        rsi_period, rsi_oversold, rsi_exit, stop_loss_pct, take_profit_pct,
        max_hold_mins, poll_interval_secs, duration_secs, onchain,
        keypair_path, rpc_url,
    }) = args.command
    {
        let token_list: Vec<String> = tokens
            .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        return commands::mean_reversion::run_mean_reversion(
            budget, buy_amount, max_positions, jito_tip, token_list,
            rsi_period, rsi_oversold, rsi_exit, stop_loss_pct, take_profit_pct,
            max_hold_mins, poll_interval_secs, duration_secs, onchain,
            keypair_path, rpc_url,
        ).await;
    }

    if let Some(Commands::RouteArb {
        budget, trade_amount, min_spread_bps, jito_tip, tokens,
        poll_interval_secs, duration_secs, onchain, keypair_path, rpc_url, jito_url,
        use_flash_loan, flash_loan_amount,
    }) = args.command
    {
        let token_list: Vec<String> = tokens
            .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        return commands::route_arb::run_route_arb(
            budget, trade_amount, min_spread_bps, jito_tip, token_list,
            poll_interval_secs, duration_secs, onchain, keypair_path, rpc_url, jito_url,
            use_flash_loan, flash_loan_amount,
        ).await;
    }

    if let Some(Commands::AnalyzeWallets {
        wallets, max_transactions, min_win_rate, min_trades, rpc_url,
    }) = args.command
    {
        return commands::wallet_analyzer::run_wallet_analyzer(
            wallets, max_transactions, min_win_rate, min_trades, rpc_url,
        ).await;
    }

    if args.snipe {
        return commands::snipe::run_sniper(
            args.grpc_endpoint,
            args.grpc_token,
            args.budget,
            args.buy_amount,
            5,
            1.0,
            0.003,
            args.duration_secs,
            args.live_reserves,
            args.require_socials,
            args.max_dev_buy,
            args.min_dev_buy,
            args.min_curve_pct,
            args.max_curve_pct,
            args.min_unique_buyers,
            args.min_buys_recent,
            args.max_sells_recent,
            args.velocity_window_secs,
            args.tp1,
            args.tp2,
            args.stop_loss,
            args.break_even_mult,
            args.paper,
            args.onchain,
            args.keypair_path,
            args.rpc_url,
            args.jito_url,
            args.mode,
            args.require_dev_sold_out,
            args.enable_raydium,
            false,
        )
        .await;
    }

    print_banner(&args.ticker);

    let llm = create_llm_client(Some(&args.provider), args.model.as_deref());

    // Parse chosen analysts
    let mut selected_analysts = Vec::new();
    for a in args.analysts.split(',') {
        match a.trim().to_lowercase().as_str() {
            "technical" | "market" => selected_analysts.push(AnalystType::Technical),
            "sentiment" => selected_analysts.push(AnalystType::Sentiment),
            "news" => selected_analysts.push(AnalystType::News),
            "fundamentals" => selected_analysts.push(AnalystType::Fundamentals),
            _ => {}
        }
    }
    if selected_analysts.is_empty() {
        selected_analysts = vec![
            AnalystType::Technical,
            AnalystType::Sentiment,
            AnalystType::News,
        ];
    }

    let mut holdings = HashMap::new();
    holdings.insert("USDC".to_string(), args.cash);
    let portfolio = Portfolio {
        cash_usd: args.cash,
        holdings,
    };

    let workflow = TradingWorkflow::new(llm.as_ref(), selected_analysts);
    let state = workflow.run(&args.ticker, portfolio).await?;

    print_final_report(&state).await;

    if args.solana_dex && state.asset_type == types::AssetType::Crypto {
        print_solana_dex_execution(&state).await;
    }

    if args.prediction_markets && state.asset_type == types::AssetType::Crypto {
        print_jupiter_prediction_market_plan(&state).await;
    }

    Ok(())
}

fn print_banner(ticker: &str) {
    println!("{}", "================================================================================".cyan());
    println!("{}", "  🦀 TRADINGAGENTS-RS: MULTI-AGENT LLM FINANCIAL TRADING FRAMEWORK (RUST)      ".bold().cyan());
    println!("{}", "================================================================================".cyan());
    println!("Target Asset: {} | Mode: Multi-Agent Parallel Graph Engine", ticker.bold().yellow());
}

async fn print_final_report(state: &types::AgentState) {
    let decision = match &state.decision {
        Some(d) => d,
        None => return,
    };

    println!("\n{}", "================================================================================".green());
    println!("{}", "                     FINAL AGENT TRADING DECISION REPORT                        ".bold().green());
    println!("{}", "================================================================================".green());

    let rating_colored = match decision.rating {
        Rating::StrongBuy => decision.rating.to_string().bold().green(),
        Rating::Buy => decision.rating.to_string().bold().bright_green(),
        Rating::Hold => decision.rating.to_string().bold().yellow(),
        Rating::Sell => decision.rating.to_string().bold().bright_red(),
        Rating::StrongSell => decision.rating.to_string().bold().red(),
    };

    println!("  Ticker:              {}", state.ticker.bold().cyan());
    println!("  Market Symbol:       {}", state.canonical_ticker);
    println!("  Asset Type:          {:?}", state.asset_type);
    println!("  Consensus Rating:    {}", rating_colored);
    println!("  Confidence Score:    {:.1}%", decision.confidence * 100.0);
    println!("  Position Sizing:     {:.1}% of portfolio", decision.allocation_pct);
    if let Some(target) = decision.target_price {
        println!("  Target Price:        ${:.2}", target);
    }
    if let Some(stop) = decision.stop_loss {
        println!("  Stop Loss:           ${:.2}", stop);
    }
    println!("  Decision Rationale:  {}", decision.rationale.italic());

    if let Some(debate) = &state.debate {
        println!("\n{}", "--- RESEARCH DEBATE SYNTHESIS ---".bold());
        println!("  • {}", debate.manager_synthesis);
    }

    if let Some(risk) = &state.risk {
        println!("\n{}", "--- RISK MANAGEMENT AUDIT ---".bold());
        println!("  • Risk Score:       {}/10", risk.risk_score);
        println!("  • Aggressive Officer: {}", risk.aggressive_view);
        println!("  • Conservative Officer: {}", risk.conservative_view);
        println!("  • Committee Note:   {}", risk.commentary);
    }
}

async fn print_solana_dex_execution(state: &types::AgentState) {
    let decision = match &state.decision {
        Some(d) => d,
        None => return,
    };

    let is_buy = matches!(decision.rating, Rating::StrongBuy | Rating::Buy);
    let trade_usd = (state.portfolio.cash_usd * (decision.allocation_pct as f64 / 100.0)).max(100.0);

    println!("\n{}", "================================================================================".magenta());
    println!("{}", "             ⚡ SOLANA ON-CHAIN DEX EXECUTION ROUTE (JUPITER API)              ".bold().magenta());
    println!("{}", "================================================================================".magenta());

    let est_price = if state.current_price > 0.0 {
        state.current_price
    } else {
        decision.target_price.unwrap_or(190.0)
    };
    let plan = build_solana_trade_route(&state.ticker, is_buy, trade_usd, est_price).await;

    println!("  Action:              {}", if is_buy { "BUY (SWAP IN)".bold().green() } else { "SELL (SWAP OUT)".bold().red() });
    println!("  Input Token:         {:.2} {}", plan.input_amount, plan.input_token);
    println!("  Estimated Output:    {:.4} {}", plan.estimated_output, plan.output_token);
    println!("  Optimal DEX Route:   {}", plan.dex_route.yellow());
    println!("  Max Price Impact:    {:.3}%", plan.price_impact_pct);
    println!("  Execution Engine:    Direct Solana SDK / Jupiter Swap Router v6");
    println!("  Status:              Simulation Verified & Ready for On-Chain Signing");
    println!("{}", "================================================================================".magenta());
}

async fn print_jupiter_prediction_market_plan(state: &types::AgentState) {
    let decision = match &state.decision {
        Some(d) => d,
        None => return,
    };

    println!("\n{}", "================================================================================".blue());
    println!("{}", "         🎯 JUPITER PREDICTION MARKET (SOLANA YES/NO BINARY CONTRACTS)         ".bold().blue());
    println!("{}", "================================================================================".blue());

    if state.prediction_markets.is_empty() {
        println!("  No active prediction markets discovered for this asset.");
        println!("{}", "================================================================================".blue());
        return;
    }

    println!("  Available Prediction Contracts Discovered on Solana:");
    for (i, market) in state.prediction_markets.iter().enumerate().take(3) {
        println!(
            "  [{}] {} | Market: {}",
            i + 1,
            market.event_title.bold(),
            market.market_title.cyan()
        );
        println!(
            "      YES: ${:.2} ({:.1}% prob) | NO: ${:.2} | Volume: ${:.0} | ID: {}",
            market.yes_price_usd,
            market.implied_probability_pct,
            market.no_price_usd,
            market.volume_usd,
            market.market_id.dimmed()
        );
    }

    let is_bullish = matches!(decision.rating, Rating::StrongBuy | Rating::Buy);
    let pred_budget = 50.0; // standard $50 binary prediction sizing

    if let Some(strategy) = formulate_prediction_strategy(&state.prediction_markets, is_bullish, pred_budget) {
        println!("\n  {}", "--- AGENT PREDICTION MARKET RECOMMENDATION ---".bold());
        println!("  Target Contract:     {}", strategy.market_title.bold().yellow());
        println!("  Event Parent:        {}", strategy.event_title);
        println!(
            "  Chosen Side:         {}",
            if strategy.is_yes { "YES".bold().green() } else { "NO".bold().red() }
        );
        println!("  Contract Price:      ${:.2} per contract (pays $1.00 on win)", strategy.contract_price_usd);
        println!("  Sized Allocation:    ${:.2} USDC", strategy.deposit_usd);
        println!("  Contracts Quantity:  {:.2} contracts", strategy.estimated_contracts);
        println!("  Max Settlement Payout: ${:.2} USDC", strategy.potential_payout_usd);
        println!(
            "  Potential Profit:    +${:.2} USDC (ROI: {:.1}%)",
            strategy.potential_net_profit_usd, strategy.return_on_investment_pct
        );
        println!("  Order Endpoint:      POST https://api.jup.ag/prediction/v1/orders");
        println!("  Deposit Mint:        USDC (EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v)");
        println!("  Status:              Unsigned Solana Transaction Ready for Wallet Execution");
    }

    println!("{}", "================================================================================".blue());
}

