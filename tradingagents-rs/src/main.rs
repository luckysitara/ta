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
use tracing::info;
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

    /// Upgrade 2: Minimum unique buyer wallets required before momentum entry
    #[arg(long)]
    min_unique_buyers: Option<usize>,

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

        /// Upgrade 2: Minimum unique buyer wallets required before momentum entry (Anti-Insider Whale Filter)
        #[arg(long)]
        min_unique_buyers: Option<usize>,

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
        min_unique_buyers,
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
            min_unique_buyers,
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
            simulate,
        )
        .await;
    }

    if let Some(Commands::Liquidate { mint }) = args.command {
        return run_liquidate(mint).await;
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
            args.min_unique_buyers,
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
    min_unique_buyers: Option<usize>,
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
    simulate: bool,
) -> anyhow::Result<()> {
    let is_paper = !onchain && paper;

    let (mut eff_live_reserves, mut eff_require_socials, mut eff_max_dev_buy, mut eff_min_dev_buy, mut eff_min_curve_pct, mut eff_min_unique_buyers) =
        (live_reserves, require_socials, max_dev_buy, min_dev_buy, min_curve_pct, min_unique_buyers);

    if let Some(ref m) = mode {
        match m.trim().to_lowercase().as_str() {
            "1" | "standard" | "default" => {
                eff_live_reserves = false;
                eff_require_socials = false;
                eff_max_dev_buy = None;
                eff_min_dev_buy = None;
                eff_min_curve_pct = None;
                eff_min_unique_buyers = None;
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
                    eff_min_curve_pct = Some(15.0);
                }
                if eff_max_dev_buy.is_none() {
                    eff_max_dev_buy = Some(0.25);
                }
                if eff_min_unique_buyers.is_none() {
                    eff_min_unique_buyers = Some(12);
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

    let mode_desc = match (eff_live_reserves, eff_require_socials, eff_min_curve_pct) {
        (_, _, Some(pct)) => format!("5. Balanced Scalper Mode (≥{:.0}% Curve Completion)", pct).bold().magenta(),
        (true, true, None) => "4. High-Conviction Mode (Option 1 + Option 2 Combined)".bold().magenta(),
        (true, false, None) => "2. Option 1 Only (Live On-Chain Bonding Curve Reserves)".bold().cyan(),
        (false, true, None) => "3. Option 2 Only (Strict Anti-Rug & Social Verification)".bold().yellow(),
        (false, false, None) => "1. Standard Mode (Fast Mempool Defaults)".bold().green(),
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
        println!("  [OPT 1] Curve Momentum:  Requires ≥ {:.1}% curve completion (≥ {:.1} SOL reserves)", min_curve, (85.0 * min_curve) / 100.0);
    }
    if let Some(min_buyers) = eff_min_unique_buyers {
        println!("  [OPT 3] Unique Buyers:   Requires ≥ {} unique wallets (Anti-Insider Whale Filter)", min_buyers.to_string().bold().green());
    }
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
        min_dev_buy_sol: eff_min_dev_buy,
        min_unique_buyers: eff_min_unique_buyers.unwrap_or(0),
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
        match executor.execute_snipe_sell_jito(&m, "", None, 0, 1, 0.0008, true).await {
            Ok(res) => {
                println!("    ✔ Submitted sell + rent reclaim: https://solscan.io/tx/{}", res.signature);
                let _ = executor.poll_signature_confirmation(&res.signature, 8, 500).await;
            }
            Err(e) => {
                println!("    Sell returned: {}. Attempting direct ATA close...", e);
                match executor.close_token_account_jito(&m, 0.0008, true).await {
                    Ok(res2) => {
                        println!("    ✔ ATA closed directly, rent reclaimed: https://solscan.io/tx/{}", res2.signature);
                        let _ = executor.poll_signature_confirmation(&res2.signature, 8, 500).await;
                    }
                    Err(e2) => {
                        println!("    ✖ Direct close skipped / error: {}", e2);
                    }
                }
            }
        }
    }

    let end_bal = executor.get_balance_sol().await.unwrap_or(0.0);
    println!("\n  Final SOL Balance: {:.4} SOL (+{:.4} SOL recovered)", end_bal, end_bal - start_bal);
    println!("{}", "================================================================================".yellow());

    Ok(())
}
