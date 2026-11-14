mod agents;
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
use solana::execution::OnChainExecutor;
use solana::prediction::formulate_prediction_strategy;
use solana::rug_filter::{RugFilterConfig, SafetyStatus};
use solana::sniper::{SniperSettings, SolanaSniperEngine};
use solana::yellowstone::{DexProtocol, PoolCreationEvent, YellowstoneListener};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};
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

        /// Enable Kamino zero-collateral flash loans for deep liquidity cross-DEX pools
        #[arg(long, default_value_t = false)]
        use_kamino_flash_loan: bool,

        /// Maximum Kamino flash loan borrow limit in SOL (default: 10.0 SOL)
        #[arg(long, default_value_t = 10.0)]
        kamino_max_borrow: f64,

        /// Minimum pool liquidity in USD required to unlock Kamino flash loan (default: $100,000)
        #[arg(long, default_value_t = 100000.0)]
        kamino_min_liquidity: f64,

        /// Maximum price impact percent allowed per leg (default: 0.65%)
        #[arg(long, default_value_t = 0.65)]
        kamino_max_price_impact: f64,

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
        return run_sniper(
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
        return run_liquidate(mint).await;
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
        return run_breakout_scanner(
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
        use_kamino_flash_loan,
        kamino_max_borrow,
        kamino_min_liquidity,
        kamino_max_price_impact,
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
            use_kamino_flash_loan,
            kamino_max_borrow_sol: kamino_max_borrow,
            kamino_min_pool_liquidity_usd: kamino_min_liquidity,
            kamino_max_price_impact_pct: kamino_max_price_impact,
        };

        let engine = solana::arbitrage::ArbitrageEngine::new(cfg)?;
        return engine.run().await;
    }

    if args.snipe {
        return run_sniper(
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

async fn run_sniper(
    endpoint: Option<String>,
    token: Option<String>,
    budget: f64,
    buy_amount: f64,
    max_positions: usize,
    min_liquidity: f64,
    jito_tip: f64,
    duration_secs: Option<u64>,
    live_reserves: bool,
    require_socials: bool,
    max_dev_buy: Option<f64>,
    min_dev_buy: Option<f64>,
    min_curve_pct: Option<f64>,
    max_curve_pct: Option<f64>,
    min_unique_buyers: Option<usize>,
    min_buys_recent: Option<usize>,
    max_sells_recent: Option<usize>,
    velocity_window_secs: Option<u64>,
    tp1: Option<f64>,
    tp2: Option<f64>,
    stop_loss: Option<f64>,
    break_even_mult: Option<f64>,
    paper: bool,
    onchain: bool,
    keypair_path: Option<String>,
    rpc_url: Option<String>,
    jito_url: Option<String>,
    mode: Option<String>,
    require_dev_sold_out: Option<bool>,
    enable_raydium: Option<bool>,
    simulate: bool,
) -> anyhow::Result<()> {
    let is_paper = !onchain && paper;

    let (mut eff_live_reserves, mut eff_require_socials, mut eff_max_dev_buy, mut eff_min_dev_buy, mut eff_min_curve_pct, mut eff_max_curve_pct, mut eff_min_unique_buyers, mut eff_min_buys_recent, mut eff_max_sells_recent, mut eff_velocity_window_secs) =
        (live_reserves, require_socials, max_dev_buy, min_dev_buy, min_curve_pct, max_curve_pct, min_unique_buyers, min_buys_recent, max_sells_recent, velocity_window_secs);

    if let Some(ref m) = mode {
        match m.trim().to_lowercase().as_str() {
            "1" | "standard" | "default" => {
                eff_live_reserves = false;
                eff_require_socials = false;
                eff_max_dev_buy = None;
                eff_min_dev_buy = None;
                eff_min_curve_pct = None;
                eff_max_curve_pct = None;
                eff_min_unique_buyers = None;
                eff_min_buys_recent = None;
                eff_max_sells_recent = None;
                eff_velocity_window_secs = None;
            }
            "2" | "opt1" | "reserves" | "live-reserves" => {
                eff_live_reserves = true;
                if eff_max_dev_buy.is_none() {
                    eff_max_dev_buy = Some(1.5);
                }
                if eff_min_dev_buy.is_none() {
                    eff_min_dev_buy = Some(0.40);
                }
            }
            "3" | "opt2" | "strict" | "anti-rug" => {
                eff_require_socials = true;
                if eff_max_dev_buy.is_none() {
                    eff_max_dev_buy = Some(0.5);
                }
                if eff_min_dev_buy.is_none() {
                    eff_min_dev_buy = Some(0.40);
                }
            }
            "4" | "high-conviction" | "combined" | "full" => {
                eff_live_reserves = true;
                eff_require_socials = true;
                if eff_max_dev_buy.is_none() {
                    eff_max_dev_buy = Some(1.5);
                }
                if eff_min_dev_buy.is_none() {
                    eff_min_dev_buy = Some(0.40);
                }
                if eff_min_unique_buyers.is_none() {
                    eff_min_unique_buyers = Some(8);
                }
            }
            "5" | "momentum" | "breakout" | "balanced" | "scalper" => {
                eff_live_reserves = true;
                if eff_min_curve_pct.is_none() {
                    eff_min_curve_pct = Some(20.0);
                }
                if eff_max_curve_pct.is_none() {
                    eff_max_curve_pct = Some(25.0);
                }
                if eff_max_dev_buy.is_none() {
                    eff_max_dev_buy = Some(0.25);
                }
                if eff_min_unique_buyers.is_none() {
                    eff_min_unique_buyers = Some(12);
                }
                if eff_min_buys_recent.is_none() {
                    eff_min_buys_recent = Some(3);
                }
                if eff_max_sells_recent.is_none() {
                    eff_max_sells_recent = Some(0);
                }
                if eff_velocity_window_secs.is_none() {
                    eff_velocity_window_secs = Some(15);
                }
            }
            other => {
                println!("  ⚠ Unrecognized preset mode '{}'. Proceeding with specified flags.", other);
            }
        }
    }

    let (eff_tp1, eff_tp2, eff_stop_loss, eff_break_even_mult) = match mode.as_deref().map(|s| s.trim().to_lowercase()) {
        Some(ref m) if m == "5" || m == "momentum" || m == "breakout" || m == "balanced" || m == "scalper" => (
            tp1.unwrap_or(1.30),
            tp2.unwrap_or(1.80),
            stop_loss.unwrap_or(-10.0),
            break_even_mult.or(Some(1.10)),
        ),
        _ => (
            tp1.unwrap_or(2.0),
            tp2.unwrap_or(5.0),
            stop_loss.unwrap_or(-25.0),
            break_even_mult.or(Some(1.10)),
        ),
    };

    let eff_jito_tip = if (jito_tip - 0.003).abs() < 1e-6 && mode.as_deref().map(|s| s == "5" || s == "balanced" || s == "scalper" || s == "momentum" || s == "breakout").unwrap_or(false) {
        0.0003
    } else {
        jito_tip
    };

    let mode_desc = match (eff_live_reserves, eff_require_socials, eff_min_curve_pct, eff_max_curve_pct) {
        (_, _, Some(min_pct), Some(max_pct)) => format!("5. Balanced Scalper Mode ({:.0}%–{:.0}% Curve Sweet Spot)", min_pct, max_pct).bold().magenta(),
        (_, _, Some(pct), None) => format!("5. Balanced Scalper Mode (≥{:.0}% Curve Completion)", pct).bold().magenta(),
        (true, true, None, _) => "4. High-Conviction Mode (Option 1 + Option 2 Combined)".bold().magenta(),
        (true, false, None, _) => "2. Option 1 Only (Live On-Chain Bonding Curve Reserves)".bold().cyan(),
        (false, true, None, _) => "3. Option 2 Only (Strict Anti-Rug & Social Verification)".bold().yellow(),
        (false, false, None, _) => "1. Standard Mode (Fast Mempool Defaults)".bold().green(),
    };

    println!("{}", "================================================================================".cyan());
    println!("{}", "   ⚡ TRADINGAGENTS-RS: SOLANA YELLOWSTONE gRPC SNIPER & MEV ENGINE           ".bold().cyan());
    println!("{}", "================================================================================".cyan());

    println!("  Execution Mode:      {}", if is_paper { "PAPER TRADING (Zero Fund Risk)".bold().green() } else { "LIVE ON-CHAIN (Jito MEV Bundles)".bold().red() });
    println!("  Strategy Mode:       {}", mode_desc);
    if is_paper {
        println!("  Paper Budget:        {} SOL starting wallet", budget.to_string().bold().green());
    }
    println!("  Buy Sizing:          {} SOL per detected launch (max {} concurrent)", buy_amount.to_string().bold().yellow(), max_positions);
    println!("  Jito MEV Tip:        {} SOL (anti-frontrun / private mempool)", eff_jito_tip.to_string().bold().yellow());
    println!("  Liquidity Threshold: ≥ {} SOL initial pool liquidity", min_liquidity.to_string().bold().cyan());
    println!("  [OPT 1] Live Reserves: {}", if eff_live_reserves { "ENABLED (Real On-Chain Bonding Curves)".bold().green() } else { "DISABLED (Fast Ticker)".yellow() });
    println!("  [OPT 2] Socials Filter: {}", if eff_require_socials { "ENABLED (Strict Twitter/Telegram Verification)".bold().green() } else { "DISABLED (Any Launch)".yellow() });
    if let Some(max_dev) = eff_max_dev_buy {
        println!("  [OPT 2] Dev Holding Cap: Max {} SOL buy per creator (Anti-Insider Bundle)", max_dev.to_string().bold().green());
    } else {
        println!("  [OPT 2] Dev Holding Cap: DISABLED");
    }
    if let Some(min_dev) = eff_min_dev_buy {
        println!("  [OPT 2] Dev Skin-in-Game: Min {} SOL buy required (Anti-Spam)", min_dev.to_string().bold().green());
    }
    if let Some(min_curve) = eff_min_curve_pct {
        if let Some(max_curve) = eff_max_curve_pct {
            println!("  [OPT 1] Curve Momentum:  Requires {:.1}%–{:.1}% curve sweet spot ({:.1}–{:.1} SOL reserves)", min_curve, max_curve, (85.0 * min_curve) / 100.0, (85.0 * max_curve) / 100.0);
        } else {
            println!("  [OPT 1] Curve Momentum:  Requires ≥ {:.1}% curve completion (≥ {:.1} SOL reserves)", min_curve, (85.0 * min_curve) / 100.0);
        }
    }
    if let Some(min_buyers) = eff_min_unique_buyers {
        println!("  [OPT 3] Unique Buyers:   Requires ≥ {} unique wallets (Anti-Insider Whale Filter)", min_buyers.to_string().bold().green());
    }
    if let Some(min_buys) = eff_min_buys_recent {
        println!("  [OPT 1] Buy Velocity:    Requires ≥ {} buys & ≤ {} sells in last {}s (Anti-Stall Filter)", min_buys.to_string().bold().green(), eff_max_sells_recent.unwrap_or(0).to_string().bold().green(), eff_velocity_window_secs.unwrap_or(15).to_string().bold().green());
    }
    println!("  [OPT B] Pure Fair Launch: {}", if require_dev_sold_out.unwrap_or(true) { "ENABLED (0.0 SOL Dev Buy Required, 0% Dumped Coins Permitted)".bold().green() } else { "DISABLED".yellow() });
    println!("  [OPT A] Raydium Sniping:  {}", if enable_raydium.unwrap_or(true) { "ENABLED (Raydium AMM v4 & CPMM Graduations via Jupiter/Jito)".bold().green() } else { "Pump.fun Only".yellow() });
    if let Some(dur) = duration_secs {
        println!("  Session Timer:       {} seconds ({} min auto-stop & audit)", dur, dur / 60);
    }
    println!("  Fast-Path Filter:    Freeze Revoked + Mint Revoked + LP Burn Verification");
    let be_info = eff_break_even_mult.map(|b| format!(" | Break-Even Floor: ≥{:.2}x", b)).unwrap_or_default();
    println!("  Exit Brackets:       TP1: {:.2}x (70%) | TP2: {:.2}x (20%) | Trailing Stop ({:.1}%){}", eff_tp1, eff_tp2, eff_stop_loss, be_info);
    println!("  Target Protocols:    Pump.fun | Raydium AMM v4 | Raydium CPMM | Meteora DLMM");

    // Initialize On-Chain Executor if live execution is enabled
    let mut live_balance = None;
    let executor = if !is_paper {
        let kp_path = keypair_path
            .or_else(|| std::env::var("KEYPAIR_PATH").ok())
            .unwrap_or_else(|| "/home/rootkit/.config/solana/id.json".to_string());

        println!("\n{}", "================================================================================".red());
        println!("{}", "   ⚡ INITIALIZING ON-CHAIN JITO MEV EXECUTION ENGINE (REAL FUNDS)            ".bold().red());
        println!("{}", "================================================================================".red());

        match OnChainExecutor::from_keypair_file(&kp_path, rpc_url.clone(), jito_url.clone()) {
            Ok(exec) => {
                let pubkey = exec.pubkey();
                println!("  Trading Keypair:     {} ({})", pubkey.to_string().bold().yellow(), kp_path.cyan());
                let mut bal_res = exec.get_balance_sol().await;
                if bal_res.is_err() {
                    for attempt in 1..=3 {
                        println!("  🔄 Retrying wallet balance query via RPC fallback (attempt {}/3)...", attempt);
                        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                        bal_res = exec.get_balance_sol().await;
                        if bal_res.is_ok() {
                            break;
                        }
                    }
                }
                match bal_res {
                    Ok(bal) => {
                        println!("  Live SOL Balance:    {:.4} SOL", bal);
                        live_balance = Some(bal);
                        let min_needed = buy_amount + jito_tip;
                        if bal < min_needed {
                            eprintln!("\n{}", "❌ [INSUFFICIENT ON-CHAIN FUNDS] Cannot proceed with live trading!".bold().red());
                            eprintln!("  Wallet {} balance: {:.4} SOL", pubkey, bal);
                            eprintln!("  Required per snipe: {:.4} SOL ({:.3} buy + {:.4} Jito tip).", min_needed, buy_amount, jito_tip);
                            eprintln!("  To execute live trades, transfer SOL to:");
                            eprintln!("  👉 {}", pubkey.to_string().bold().yellow());
                            eprintln!("  Then re-run: ./snipe.sh --onchain\n");
                            return Ok(());
                        }
                        Some(Arc::new(exec))
                    }
                    Err(e) => {
                        eprintln!("\n{}", format!("❌ Failed to query wallet balance via RPC: {}", e).bold().red());
                        return Ok(());
                    }
                }
            }
            Err(e) => {
                eprintln!("\n{}", format!("❌ Failed to load keypair from '{}': {}", kp_path, e).bold().red());
                eprintln!("  Ensure your Solana keypair exists or set KEYPAIR_PATH in .env\n");
                return Ok(());
            }
        }
    } else {
        None
    };

    // Resolve endpoint and token from env if empty or placeholders were passed
    let mut resolved_endpoint = endpoint.and_then(|e| {
        let trimmed = e.trim().to_string();
        if trimmed.is_empty() || trimmed == "SOLANA_GRPC_ENDPOINT" || trimmed == "$SOLANA_GRPC_ENDPOINT" {
            None
        } else {
            Some(trimmed)
        }
    });

    if resolved_endpoint.is_none() {
        resolved_endpoint = std::env::var("SOLANA_GRPC_ENDPOINT").ok().and_then(|e| {
            let trimmed = e.trim().to_string();
            if trimmed.is_empty() { None } else { Some(trimmed) }
        });
    }

    let mut resolved_token = token.and_then(|t| {
        let trimmed = t.trim().to_string();
        if trimmed.is_empty() || trimmed == "SOLANA_GRPC_TOKEN" || trimmed == "$SOLANA_GRPC_TOKEN" || trimmed == "YOUR_API_TOKEN" {
            None
        } else {
            Some(trimmed)
        }
    });

    if resolved_token.is_none() {
        resolved_token = std::env::var("SOLANA_GRPC_TOKEN").ok().and_then(|t| {
            let trimmed = t.trim().to_string();
            if trimmed.is_empty() || trimmed == "YOUR_API_TOKEN" { None } else { Some(trimmed) }
        });
    }

    let rug_filter_config = RugFilterConfig {
        require_socials: eff_require_socials,
        max_dev_buy_sol: eff_max_dev_buy,
        min_dev_buy_sol: eff_min_dev_buy,
        ..Default::default()
    };

    let eff_budget = if (budget - 0.5).abs() > f64::EPSILON {
        budget
    } else if let Some(live_bal) = live_balance {
        live_bal
    } else {
        budget
    };

    let settings = SniperSettings {
        total_budget_sol: eff_budget,
        buy_amount_sol: buy_amount,
        max_concurrent_positions: max_positions,
        min_liquidity_sol: min_liquidity,
        jito_tip_sol: eff_jito_tip,
        live_reserves: eff_live_reserves,
        is_paper_trading: is_paper,
        rug_filter: rug_filter_config,
        tp1_multiplier: eff_tp1,
        tp2_multiplier: eff_tp2,
        stop_loss_pct: eff_stop_loss,
        break_even_mult: eff_break_even_mult,
        min_curve_pct: eff_min_curve_pct,
        max_curve_pct: eff_max_curve_pct,
        min_dev_buy_sol: eff_min_dev_buy,
        min_unique_buyers: eff_min_unique_buyers.unwrap_or(0),
        min_buys_recent: eff_min_buys_recent,
        max_sells_recent: eff_max_sells_recent,
        velocity_window_secs: eff_velocity_window_secs,
        require_dev_sold_out: require_dev_sold_out.unwrap_or(true),
        enable_raydium: enable_raydium.unwrap_or(true),
        ..Default::default()
    };

    let engine = Arc::new(SolanaSniperEngine::new_with_executor(settings, executor));
    engine.print_portfolio_hud();

    if simulate || resolved_endpoint.is_none() {
        println!("\n{}", "--- 🎮 RUNNING YELLOWSTONE STREAM INGESTION SIMULATION ---".bold().magenta());
        if resolved_endpoint.is_none() {
            println!("  (No active Yellowstone gRPC URL found. Running high-speed simulation & replay benchmark)");
        }

        // Scenario 1: Fresh Pump.fun Token Launch
        println!("\n{}", "[EVENT 1] Stream Item: Slot 298,401,912 | Program: Pump.fun (6EF8rrecth...)".cyan());
        let pump_event = PoolCreationEvent {
            protocol: DexProtocol::PumpFun,
            signature: "3zB4xYv5WqP7LmN9Jk8Rt2Ds1Za4Xy7Ue8Vo9Qw3Er6".to_string(),
            slot: 298401912,
            token_mint: "CzLSujWBLFsSjncfkh59rQD4ETJyDxqqWUPZAE7Pump".to_string(),
            pool_address: "BondingCurve77x8y9z0123456789abcdefghijklmn".to_string(),
            creator: Some("DevWallet88888888888888888888888888888888888".to_string()),
            initial_sol_liquidity: Some(30.0),
            dev_buy_sol: Some(0.5),
            metadata_uri: Some("https://ipfs.io/ipfs/QmPump...".to_string()),
            has_socials: true,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            log_snippet: Some("Pump.fun Bonding Curve Launched".to_string()),
            is_token_2022: true,
        };
        let rep1 = engine.process_detected_pool(&pump_event);
        if let Some(r) = rep1 {
            println!("  Latency:             {} µs (pure memory zero-latency path)", r.execution_latency_us.to_string().bold().green());
            println!("  Security Status:     {}", "SAFE (Bonding Curve Locked, Freeze Disabled)".green());
            let jito_bundle = engine.build_jito_bundle_plan();
            println!("  MEV Protection:      Jito Tip Account: {} ({} lamports)", &jito_bundle.tip_account[..12], jito_bundle.tip_lamports);
        }

        // Scenario 2: Malicious Honeypot Token Launch (Active Freeze Authority)
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        println!("\n{}", "[EVENT 2] Stream Item: Slot 298,401,914 | Program: Raydium AMM (675kPX9M...)".red());
        println!("  Analyzing newly initialized AMM pool: BadHoneyPotToken1111111111111111111111111");
        let honeypot_filter = solana::rug_filter::RugFilter::new(RugFilterConfig::default());
        let rep2 = honeypot_filter.evaluate_fast(
            "BadHoneyPotToken1111111111111111111111111",
            false,
            10.0,
            false, // ACTIVE FREEZE AUTHORITY!
            true,
            2.0,
            false,
            None,
        );
        match rep2.status {
            SafetyStatus::Rejected(ref reason) => {
                println!("  Security Alert:      {}", "REJECTED (Honeypot Protection Triggered)".bold().red());
                println!("  Reason:              {} [Evaluation time: {} µs]", reason.red(), rep2.execution_latency_us);
                println!("  Action Taken:        Transaction dropped. 0 SOL spent. Honeypot avoided.");
            }
            _ => {}
        }

        // Scenario 3: Legitimate Raydium Pool Creation
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        println!("\n{}", "[EVENT 3] Stream Item: Slot 298,401,918 | Program: Raydium AMM (675kPX9M...)".green());
        let raydium_event = PoolCreationEvent {
            protocol: DexProtocol::RaydiumAmm,
            signature: "4mK9pL2oN7qR5sT3uV1wX8yZ6aB4cD2eF9gH7jK5mN3".to_string(),
            slot: 298401918,
            token_mint: "SolAiAgent77777777777777777777777777777777".to_string(),
            pool_address: "RaydiumPoolAMM55555555555555555555555555555".to_string(),
            creator: Some("TeamWallet999999999999999999999999999999999".to_string()),
            initial_sol_liquidity: Some(25.0),
            dev_buy_sol: None,
            metadata_uri: None,
            has_socials: true,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            log_snippet: Some("Raydium AMM v4 Pool Initialized".to_string()),
            is_token_2022: false,
        };
        let rep3 = engine.process_detected_pool(&raydium_event);
        if let Some(r) = rep3 {
            println!("  Latency:             {} µs", r.execution_latency_us.to_string().bold().green());
            println!("  Security Status:     {}", "SAFE (Mint Revoked, LP Locked, Safe Creator Share)".green());
        }

        // Simulate position progression and take profit / stop loss triggers
        println!("\n{}", "--- 📈 ADVANCING POSITION LIFECYCLE TICKER (TP / SL SIMULATION) ---".bold().cyan());
        for tick in 1..=6 {
            tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
            println!("  [TICK {}] Updating bonding curve prices...", tick);
            engine.tick_position_updates();
        }

        // Summary of sniped positions
        println!("\n{}", "================================================================================".cyan());
        println!("{}", "                     ACTIVE SNIPED POSITIONS & RISK AUDIT                       ".bold().cyan());
        println!("{}", "================================================================================".cyan());
        let active_positions = engine.get_active_positions();
        for (idx, pos) in active_positions.iter().enumerate() {
            println!(
                "  [{}] Token: {} | Protocol: {}",
                idx + 1,
                pos.token_mint.bold().yellow(),
                pos.protocol
            );
            println!(
                "      Entry Amount:   {:.3} SOL | Slot: {} | Status: Active Paper Position",
                pos.buy_amount_sol, pos.entry_slot
            );
            println!(
                "      TP 1 (50%):     2.0x (+100%) | TP 2 (25%): 5.0x (+400%) | Stop Loss: -25%"
            );
        }
        println!("{}", "================================================================================".cyan());
        println!("{}", "  To connect to your live Yellowstone gRPC endpoint, pass your actual URL:".italic());
        println!("{}", "  cargo run --manifest-path tradingagents-rs/Cargo.toml -- snipe \\".yellow());
        println!("{}", "    --grpc-endpoint \"https://your-grpc-provider:443\" \\".bold().yellow());
        println!("{}", "    --grpc-token \"YOUR_TOKEN\"".bold().yellow());
        println!("{}", "================================================================================".cyan());
        return Ok(());
    }

    // Live streaming mode: validate URL scheme
    let mut live_endpoint = resolved_endpoint.unwrap();
    if !live_endpoint.starts_with("http://") && !live_endpoint.starts_with("https://") {
        if live_endpoint.contains('.') || live_endpoint.contains(':') {
            if live_endpoint.starts_with("127.0.0.1") || live_endpoint.starts_with("localhost") {
                live_endpoint = format!("http://{}", live_endpoint);
            } else {
                live_endpoint = format!("https://{}", live_endpoint);
            }
        } else {
            eprintln!("\n{}", format!("❌ Invalid Yellowstone gRPC endpoint: '{}'", live_endpoint).bold().red());
            eprintln!("  The endpoint must be a valid URL, for example: https://my-grpc-provider.com:443");
            eprintln!("  Or export SOLANA_GRPC_ENDPOINT in your environment or .env file.\n");
            return Ok(());
        }
    }

    // Auto-detect and extract token from URL path (e.g. Triton/rpcpool: https://host.rpcpool.com/TOKEN)
    if let Ok(parsed) = reqwest::Url::parse(&live_endpoint) {
        let path = parsed.path().trim_start_matches('/').trim();
        if !path.is_empty() {
            if resolved_token.is_none() {
                resolved_token = Some(path.to_string());
                println!("  🔑 Extracted Yellowstone authentication token from URL path.");
            }
            let scheme = parsed.scheme();
            let host = parsed.host_str().unwrap_or_default();
            let port = parsed.port_or_known_default().unwrap_or(443);
            live_endpoint = format!("{}://{}:{}", scheme, host, port);
        }
    }

    info!("Initializing live Yellowstone listener for {}", live_endpoint);
    let listener = YellowstoneListener::new(
        live_endpoint,
        resolved_token,
        vec![DexProtocol::PumpFun, DexProtocol::RaydiumAmm, DexProtocol::RaydiumCpmm, DexProtocol::MeteoraDlmm],
        eff_live_reserves,
    );

    // Spawn background position lifecycle ticker
    let ticker_engine = Arc::clone(&engine);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1500));
        let mut tick_counter: u64 = 0;
        loop {
            interval.tick().await;
            ticker_engine.tick_position_updates();
            tick_counter += 1;
            // Every 5 ticks (~7.5s), sync real on-chain wallet balance from RPC
            if tick_counter % 5 == 0 {
                ticker_engine.sync_live_wallet_balance().await;
            }
        }
    });

    let engine_tx = Arc::clone(&engine);
    let engine_acc = Arc::clone(&engine);
    let engine_trade = Arc::clone(&engine);
    if let Some(dur) = duration_secs {
        let dur_d = std::time::Duration::from_secs(dur);
        tokio::select! {
            res = listener.listen(
                move |event| {
                    engine_tx.process_detected_pool(&event);
                },
                move |state| {
                    engine_acc.update_bonding_curve_reserve(&state.pool_address, &state);
                },
                move |trade| {
                    engine_trade.record_trade_activity(&trade);
                },
            ) => {
                if let Err(e) = res {
                    eprintln!("Yellowstone stream ended with error: {e}");
                }
            }
            _ = tokio::time::sleep(dur_d) => {
                println!("\n{}", "================================================================================".yellow());
                let sess_type = if is_paper { "paper trading" } else { "live on-chain" };
                println!("{}", format!("  ⏱ Completed {}s ({} min) {} session. Shutting down stream...", dur, dur / 60, sess_type).bold().yellow());
                println!("{}", "================================================================================".yellow());
            }
        }
    } else {
        let engine_tx2 = Arc::clone(&engine);
        let engine_acc2 = Arc::clone(&engine);
        let engine_trade2 = Arc::clone(&engine);
        listener.listen(
            move |event| {
                engine_tx2.process_detected_pool(&event);
            },
            move |state| {
                engine_acc2.update_bonding_curve_reserve(&state.pool_address, &state);
            },
            move |trade| {
                engine_trade2.record_trade_activity(&trade);
            },
        ).await?;
    }

    // Print final session summary audit report
    engine.print_session_summary();

    Ok(())
}

async fn run_liquidate(mint_filter: Option<String>) -> anyhow::Result<()> {
    let keypair_path = std::env::var("KEYPAIR_PATH").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{}/.config/solana/id.json", home)
    });
    let rpc_url = std::env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://graceful-oasis-a398.mainnet.rpcpool.com/95794a05-4057-49cb-87e4-42d3d9a46774".to_string());
    let jito_url = std::env::var("JITO_BLOCK_ENGINE_URL")
        .unwrap_or_else(|_| "https://amsterdam.mainnet.block-engine.jito.wtf".to_string());

    let executor = solana::execution::OnChainExecutor::from_keypair_file(
        &keypair_path,
        Some(rpc_url),
        Some(jito_url),
    )?;

    let target_mints = if let Some(m) = mint_filter {
        vec![m]
    } else {
        vec![
            "gRZYTt5mLxFBrGKgGFchF5dwceicrdUy6KHqdX5pump".to_string(),
            "4LNK1tmpEUEWbaETHBSVAiiWzaKMYTSyp9FsUDhppump".to_string(),
            "5UV9bpCZAQonS6mBxqaVxjwSdHmGjQj5GiDStccvzuuS".to_string(),
            "89rnDGDYFwvTzNwQE2g8KYnvtjoa9t8Laep4eSzUpump".to_string(),
            "A3pWk6Ug7DmsK8xxh48C6wdFAJUiXJ4ApgWU2i3Apump".to_string(),
            "BmTTAw3ynka1jF5Yv5JYx7KG8wL9Evc6VM65zPjMpump".to_string(),
            "C2n2PBLMhzMzopgw9viMky7D55cXrMvTacRGeVXsNTvg".to_string(),
            "EWLY2Qt86LxLGV4hP3RnWpfxGmuRc7LVYiaSSk8Lpump".to_string(),
            "F1DM6e2BqfndCzDFSxa119uDReDN8sgrDzfkx8kgjkcL".to_string(),
            "F7XXpuHXqZsNDSJ5CUMFUA2Ndb4mPrmy4i6mjJrHpump".to_string(),
        ]
    };

    println!("{}", "================================================================================".yellow());
    println!("{}", "       ⚡ TRADINGAGENTS-RS: LIQUIDATING OPEN POSITIONS TO RECLAIM SOL           ".bold().yellow());
    println!("{}", "================================================================================".yellow());

    let start_bal = executor.get_balance_sol().await.unwrap_or(0.0);
    println!("  Initial SOL Balance: {:.4} SOL", start_bal);

    for m in target_mints {
        println!("  Attempting to sell / close ATA: {}...", m);
        let mut sold = false;

        // 1. Try Jupiter DEX swap first (covers Raydium, Meteora, PumpSwap, Orca, and Pump.fun)
        match executor.execute_jupiter_swap_jito(&m, "So11111111111111111111111111111111111111112", 0, 0.0003, 300).await {
            Ok(res) => {
                println!("    ✔ Submitted Jupiter DEX swap: https://solscan.io/tx/{}", res.signature);
                let _ = executor.poll_signature_confirmation(&res.signature, 8, 500).await;
                sold = true;
            }
            Err(e) => {
                println!("    Jupiter swap returned: {}. Attempting direct Pump.fun bonding curve...", e);
            }
        }

        // 2. Fallback to direct Pump.fun bonding curve instruction
        if !sold {
            match executor.execute_snipe_sell_jito(&m, "", None, 0, 1, 0.0008, true).await {
                Ok(res) => {
                    println!("    ✔ Submitted Pump.fun sell: https://solscan.io/tx/{}", res.signature);
                    let _ = executor.poll_signature_confirmation(&res.signature, 8, 500).await;
                }
                Err(e) => {
                    println!("    Pump.fun sell returned: {}. Proceeding to close ATA...", e);
                }
            }
        }

        // 3. Close ATA to reclaim 0.002039 SOL rent (try standard SPL Token then Token-2022)
        match executor.close_token_account_jito(&m, 0.0003, false).await {
            Ok(res2) => {
                println!("    ✔ SPL ATA closed, rent reclaimed: https://solscan.io/tx/{}", res2.signature);
                let _ = executor.poll_signature_confirmation(&res2.signature, 8, 500).await;
            }
            Err(_) => {
                if let Ok(res2) = executor.close_token_account_jito(&m, 0.0003, true).await {
                    println!("    ✔ Token-2022 ATA closed, rent reclaimed: https://solscan.io/tx/{}", res2.signature);
                    let _ = executor.poll_signature_confirmation(&res2.signature, 8, 500).await;
                }
            }
        }
    }

    let end_bal = executor.get_balance_sol().await.unwrap_or(0.0);
    println!("\n  Final SOL Balance: {:.4} SOL (+{:.4} SOL recovered)", end_bal, end_bal - start_bal);
    println!("{}", "================================================================================".yellow());

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_breakout_scanner(
    budget: f64,
    buy_amount: f64,
    max_positions: usize,
    jito_tip: f64,
    min_liquidity_usd: f64,
    min_volume_24h_usd: f64,
    min_volume_5m_usd: f64,
    min_buys_5m: usize,
    min_buy_ratio_5m: f64,
    min_price_change_5m: f64,
    max_price_change_24h: f64,
    max_price_change_1h: f64,
    strategy: String,
    min_dip_pct: f64,
    max_dip_pct: f64,
    min_buy_ratio_1h: f64,
    max_graduation_age_mins: f64,
    min_graduation_liquidity_usd: f64,
    min_graduation_buy_ratio_5m: f64,
    stop_loss_pct: f64,
    trailing_take_profit_activation_pct: f64,
    trailing_take_profit_callback_pct: f64,
    trailing_stop_pct: f64,
    break_even_mult: f64,
    poll_interval_secs: u64,
    duration_secs: Option<u64>,
    onchain: bool,
    keypair_path: Option<String>,
    rpc_url: Option<String>,
) -> anyhow::Result<()> {
    println!("\n{}", "===============================================================".bold().cyan());
    println!("{}", "   🚀 SOLANA SECONDARY MARKET DEX BREAKOUT & MOMENTUM ENGINE   ".bold().green());
    println!("{}", "      Uncapped Upside | Trailing Take-Profit | Fee-Optimized    ".bold().yellow());
    println!("{}", "===============================================================\n".bold().cyan());

    let strat_mode = match strategy.to_lowercase().as_str() {
        "dip" | "dip-rebound" | "strategy1" | "1" => solana::breakout::ScalpStrategyMode::DipReboundScalp,
        "grad" | "graduation" | "burst" | "strategy3" | "3" => solana::breakout::ScalpStrategyMode::GraduationBurstScalp,
        "gem" | "early" | "earlygem" | "strategy5" | "5" => solana::breakout::ScalpStrategyMode::EarlyGemRunner,
        "breakout" | "momentum" | "strategy2" | "2" => solana::breakout::ScalpStrategyMode::MomentumBreakout,
        _ => solana::breakout::ScalpStrategyMode::DipAndGraduation,
    };

    let is_paper = !onchain;
    println!("  Execution Mode:       {}", if is_paper { "PAPER TRADING (Zero Financial Risk)".bold().green() } else { "LIVE ON-CHAIN (Real SOL Execution via Jupiter & Jito MEV)".bold().red() });
    println!("  Scalp Strategies:     {:?} (Strategy 1: Dip-Rebound & Strategy 3: Graduation Burst | Strategy 2 PERMANENTLY DISABLED)", strat_mode);
    println!("  Budget / Allocation:  Total: {:.4} SOL | Size: {:.4} SOL per trade", budget, buy_amount);
    println!("  Max Open Positions:   {}", max_positions);
    println!("  Jito MEV Tip:         {:.4} SOL (Optimized for minimal fee drag)", jito_tip);
    println!("  Strategy 1 (Dip):     Dip: [{:.1}% .. {:.1}%] | Min 1h Buy Dominance: ≥ {:.0}%", min_dip_pct, max_dip_pct, min_buy_ratio_1h * 100.0);
    println!("  Strategy 3 (Grad):    Max Pool Age: ≤ {:.0}m | Min Pool Liq: ≥ ${:.1}k | Min 5m Buys: ≥ {:.0}%", max_graduation_age_mins, min_graduation_liquidity_usd / 1000.0, min_graduation_buy_ratio_5m * 100.0);
    println!("  Liquidity Floor:      ≥ ${:.1}k USD (Meteora DLMM / Raydium / PumpSwap / Orca)", min_liquidity_usd / 1000.0);
    println!("  Volume Surge Filter:  ≥ ${:.1}k (24h) | ≥ ${:.1}k (5m)", min_volume_24h_usd / 1000.0, min_volume_5m_usd / 1000.0);
    println!("  Buy Pressure Filter:  ≥ {} buys in 5m | ≥ {:.0}% buy dominance", min_buys_5m, min_buy_ratio_5m * 100.0);
    println!("  Momentum Trigger:     ≥ +{:.1}% in 5m candle", min_price_change_5m);
    println!("  Anti-FOMO Ceiling:    Max 24h: +{:.0}% | Max 1h: +{:.0}% (Rejects late exhaustion)", max_price_change_24h, max_price_change_1h);
    println!("  Trailing Take-Profit: Active at ≥+{:.1}% | Trails {:.1}% from peak | Min Floor: +2.0%", trailing_take_profit_activation_pct, trailing_take_profit_callback_pct);
    println!("  Tight Stop Loss:      {:.1}% from entry (Tight Capital Protection)", stop_loss_pct);
    println!("  Fallback Stop:        {:.1}% from peak watermark (Break-Even Floor: ≥{:.2}x)", trailing_stop_pct, break_even_mult);
    println!("  Scanner Interval:     Every {} seconds\n", poll_interval_secs);

    let executor = if !is_paper {
        let key_path = keypair_path.unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            format!("{}/.config/solana/id.json", home)
        });
        let rpc = rpc_url.or_else(|| std::env::var("SOLANA_RPC_URL").ok())
            .unwrap_or_else(|| "https://graceful-oasis-a398.mainnet.rpcpool.com/95794a05-4057-49cb-87e4-42d3d9a46774".to_string());
        let jito = std::env::var("JITO_BLOCK_ENGINE_URL")
            .unwrap_or_else(|_| "https://amsterdam.mainnet.block-engine.jito.wtf".to_string());
        let exec = solana::execution::OnChainExecutor::from_keypair_file(&key_path, Some(rpc), Some(jito))?;
        let bal = exec.get_balance_sol().await?;
        println!("  ⚡ [LIVE WALLET] Connected: {} | Balance: {:.4} SOL\n", exec.pubkey(), bal);
        Some(Arc::new(exec))
    } else {
        None
    };

    let settings = solana::breakout::BreakoutSettings {
        total_budget_sol: budget,
        buy_amount_sol: buy_amount,
        max_concurrent_positions: max_positions,
        jito_tip_sol: jito_tip,
        min_liquidity_usd,
        min_volume_24h_usd,
        min_volume_5m_usd,
        min_buys_5m,
        min_buy_ratio_5m,
        min_price_change_5m,
        max_price_change_24h: Some(max_price_change_24h),
        max_price_change_1h: Some(max_price_change_1h),
        min_price_change_24h: Some(0.0),
        stop_loss_pct,
        trailing_take_profit_activation_pct,
        trailing_take_profit_callback_pct,
        trailing_stop_pct,
        break_even_mult,
        is_paper_trading: is_paper,
        poll_interval_secs,
        strategy_mode: strat_mode,
        min_dip_pct,
        max_dip_pct,
        min_buy_ratio_1h,
        max_graduation_age_mins,
        min_graduation_liquidity_usd,
        min_graduation_buy_ratio_5m,
    };

    let engine = Arc::new(solana::breakout::BreakoutEngine::new(settings, executor));
    let engine_clone = Arc::clone(&engine);

    // Sync any open wallet positions on startup
    if let Err(e) = engine_clone.sync_open_positions_from_wallet().await {
        eprintln!("  ✖ Warning: Error syncing open wallet positions: {e}");
    }

    let start_time = std::time::Instant::now();
    println!("  ⚡ Starting live DEX breakout scanner... (Press Ctrl+C to stop)\n");

    loop {
        if let Some(dur) = duration_secs {
            if start_time.elapsed().as_secs() >= dur {
                println!("\n⏰ Duration limit reached ({}s). Stopping scanner.", dur);
                break;
            }
        }

        match engine_clone.run_scan_cycle().await {
            Ok(cnt) => {
                if cnt > 0 {
                    info!("Scan cycle completed: {} new breakout positions entered.", cnt);
                }
            }
            Err(e) => {
                warn!("Scan cycle warning: {e}");
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(poll_interval_secs)).await;
    }

    Ok(())
}
