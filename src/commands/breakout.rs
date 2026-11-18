use colored::*;
use std::sync::Arc;
use tracing::{info, warn};

use crate::solana::breakout::{BreakoutEngine, BreakoutSettings, ScalpStrategyMode};
use crate::solana::ledger::Ledger;
use crate::solana::telegram::{TelegramNotifier, TradeAlert};

#[allow(clippy::too_many_arguments)]
pub async fn run_breakout_scanner(
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
    println!("{}", "   SOLANA SECONDARY MARKET DEX BREAKOUT & MOMENTUM ENGINE      ".bold().green());
    println!("{}", "      Uncapped Upside | Trailing Take-Profit | Fee-Optimized   ".bold().yellow());
    println!("{}", "===============================================================\n".bold().cyan());

    let strat_mode = match strategy.to_lowercase().as_str() {
        "dip" | "dip-rebound" | "strategy1" | "1" => ScalpStrategyMode::DipReboundScalp,
        "grad" | "graduation" | "burst" | "strategy3" | "3" => ScalpStrategyMode::GraduationBurstScalp,
        "breakout" | "momentum" | "strategy2" | "2" => ScalpStrategyMode::MomentumBreakout,
        _ => ScalpStrategyMode::DipAndGraduation,
    };

    let is_paper = !onchain;
    println!("  Execution Mode:       {}", if is_paper { "PAPER TRADING (Zero Financial Risk)".bold().green() } else { "LIVE ON-CHAIN (Real SOL Execution via Jupiter & Jito MEV)".bold().red() });
    println!("  Scalp Strategies:     {:?}", strat_mode);
    println!("  Budget / Allocation:  Total: {:.4} SOL | Size: {:.4} SOL per trade", budget, buy_amount);
    println!("  Max Open Positions:   {}", max_positions);
    println!("  Jito MEV Tip:         {:.4} SOL", jito_tip);
    println!("  Strategy 1 (Dip):     Dip: [{:.1}% .. {:.1}%] | Min 1h Buy Dominance: >= {:.0}%", min_dip_pct, max_dip_pct, min_buy_ratio_1h * 100.0);
    println!("  Strategy 3 (Grad):    Max Pool Age: <= {:.0}m | Min Pool Liq: >= ${:.1}k | Min 5m Buys: >= {:.0}%", max_graduation_age_mins, min_graduation_liquidity_usd / 1000.0, min_graduation_buy_ratio_5m * 100.0);
    println!("  Liquidity Floor:      >= ${:.1}k USD", min_liquidity_usd / 1000.0);
    println!("  Volume Surge Filter:  >= ${:.1}k (24h) | >= ${:.1}k (5m)", min_volume_24h_usd / 1000.0, min_volume_5m_usd / 1000.0);
    println!("  Buy Pressure Filter:  >= {} buys in 5m | >= {:.0}% buy dominance", min_buys_5m, min_buy_ratio_5m * 100.0);
    println!("  Momentum Trigger:     >= +{:.1}% in 5m candle", min_price_change_5m);
    println!("  Anti-FOMO Ceiling:    Max 24h: +{:.0}% | Max 1h: +{:.0}%", max_price_change_24h, max_price_change_1h);
    println!("  Trailing Take-Profit: Active at >=+{:.1}% | Trails {:.1}% from peak", trailing_take_profit_activation_pct, trailing_take_profit_callback_pct);
    println!("  Tight Stop Loss:      {:.1}% from entry", stop_loss_pct);
    println!("  Fallback Stop:        {:.1}% from peak watermark (Break-Even Floor: >={:.2}x)", trailing_stop_pct, break_even_mult);
    println!("  Scanner Interval:     Every {} seconds\n", poll_interval_secs);

    // Initialize Telegram notifier
    let notifier = TelegramNotifier::from_env();
    if notifier.is_some() {
        println!("  Telegram Alerts:     {}", "ENABLED".bold().green());
    }

    let executor = if !is_paper {
        let key_path = keypair_path.unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            format!("{}/.config/solana/id.json", home)
        });
        let rpc = rpc_url.or_else(|| std::env::var("SOLANA_RPC_URL").ok())
            .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".to_string());
        let jito = std::env::var("JITO_BLOCK_ENGINE_URL")
            .unwrap_or_else(|_| "https://amsterdam.mainnet.block-engine.jito.wtf".to_string());
        let exec = crate::solana::execution::OnChainExecutor::from_keypair_file(&key_path, Some(rpc), Some(jito))?;
        let bal = exec.get_balance_sol().await?;
        println!("  [LIVE WALLET] Connected: {} | Balance: {:.4} SOL\n", exec.pubkey(), bal);
        Some(Arc::new(exec))
    } else {
        None
    };

    let settings = BreakoutSettings {
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

    let engine = Arc::new(BreakoutEngine::new(settings, executor));
    let engine_clone = Arc::clone(&engine);

    // Restore state from ledger
    let ledger = Ledger::from_env_or_default();
    if let Ok(Some((saved_portfolio, saved_positions))) = ledger.load_breakout_state() {
        let open_count = saved_positions.values().filter(|p| !p.is_closed).count();
        if open_count > 0 {
            println!("  Ledger Restore:      {} open positions recovered from {}", open_count.to_string().bold().green(), ledger.path().display());
            engine.restore_state(saved_portfolio, saved_positions);
        }
    }

    // Sync any open wallet positions on startup
    if let Err(e) = engine_clone.sync_open_positions_from_wallet().await {
        eprintln!("  Warning: Error syncing open wallet positions: {e}");
    }

    let start_time = std::time::Instant::now();
    println!("  Starting live DEX breakout scanner... (Press Ctrl+C to stop)\n");

    let mut cycle_count: u64 = 0;
    loop {
        if let Some(dur) = duration_secs {
            if start_time.elapsed().as_secs() >= dur {
                println!("\n Duration limit reached ({}s). Stopping scanner.", dur);
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

        cycle_count += 1;
        // Save ledger every 6 cycles (~60s at 10s poll)
        if cycle_count % 6 == 0 {
            engine_clone.save_to_ledger(&ledger);
        }

        tokio::time::sleep(std::time::Duration::from_secs(poll_interval_secs)).await;
    }

    // Final save
    engine_clone.save_to_ledger(&ledger);

    // Send session summary via Telegram
    if let Some(ref notifier) = notifier {
        let port = engine_clone.get_portfolio_state();
        notifier.send(TradeAlert::SessionSummary {
            engine: "Breakout".to_string(),
            is_paper,
            initial_budget: port.initial_budget_sol,
            final_value: port.total_portfolio_value_sol(),
            realized_pnl: port.realized_pnl_sol,
            total_trades: port.total_trades,
            winning: port.winning_trades,
            losing: port.losing_trades,
            win_rate: port.win_rate_pct(),
        });
    }

    Ok(())
}
