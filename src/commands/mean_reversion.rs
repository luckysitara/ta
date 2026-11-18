use anyhow::Result;
use colored::Colorize;
use tracing::info;

use crate::solana::execution::OnChainExecutor;
use crate::solana::ledger::Ledger;
use crate::solana::mean_reversion::{MeanReversionEngine, MeanReversionSettings, TrackedToken};
use crate::solana::telegram::TelegramNotifier;

pub async fn run_mean_reversion(
    budget: f64,
    buy_amount: f64,
    max_positions: usize,
    jito_tip: f64,
    tokens: Vec<String>,
    rsi_period: usize,
    rsi_oversold: f64,
    rsi_exit: f64,
    stop_loss_pct: f64,
    take_profit_pct: f64,
    max_hold_mins: u64,
    poll_interval_secs: u64,
    duration_secs: Option<u64>,
    onchain: bool,
    keypair_path: Option<String>,
    rpc_url: Option<String>,
) -> Result<()> {
    println!("{}", "================================================================================".cyan());
    println!("{}", "  📉 MEAN REVERSION RSI ENGINE                                                  ".bold().cyan());
    println!("{}", "================================================================================".cyan());

    let tracked_tokens = if tokens.is_empty() {
        MeanReversionSettings::default().tracked_tokens
    } else {
        tokens.iter().map(|t| {
            let parts: Vec<&str> = t.split(':').collect();
            if parts.len() == 2 {
                TrackedToken { mint: parts[0].to_string(), symbol: parts[1].to_string() }
            } else {
                TrackedToken { mint: t.clone(), symbol: t[..t.len().min(8)].to_string() }
            }
        }).collect()
    };

    let settings = MeanReversionSettings {
        total_budget_sol: budget,
        buy_amount_sol: buy_amount,
        max_concurrent_positions: max_positions,
        jito_tip_sol: jito_tip,
        tracked_tokens: tracked_tokens.clone(),
        rsi_period,
        rsi_oversold,
        rsi_exit,
        stop_loss_pct,
        take_profit_pct,
        max_hold_mins,
        poll_interval_secs,
        is_paper_trading: !onchain,
        ..Default::default()
    };

    let executor = if onchain {
        let kp = keypair_path.unwrap_or_else(|| {
            std::env::var("KEYPAIR_PATH").unwrap_or_else(|_| "~/.config/solana/id.json".to_string())
        });
        let rpc = rpc_url.unwrap_or_else(|| {
            std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string())
        });
        Some(OnChainExecutor::from_keypair_file(&kp, Some(rpc), None)?)
    } else {
        None
    };

    let engine = MeanReversionEngine::new(settings, executor);
    let ledger = Ledger::from_env_or_default();
    let _notifier = TelegramNotifier::from_env();

    let mode = if onchain { "LIVE" } else { "PAPER" };
    println!("  Mode:              {}", mode.bold());
    println!("  Budget:            {:.4} SOL", budget);
    println!("  Tracked Tokens:    {}", tracked_tokens.iter().map(|t| t.symbol.as_str()).collect::<Vec<_>>().join(", "));
    println!("  RSI Period:        {}", rsi_period);
    println!("  RSI Oversold:      < {:.0}", rsi_oversold);
    println!("  RSI Exit:          > {:.0}", rsi_exit);
    println!("  Max Hold:          {}m", max_hold_mins);
    println!("  Stop Loss:         {:.1}%", stop_loss_pct);
    println!("{}", "================================================================================".cyan());
    println!("  Collecting initial price data ({} samples needed for RSI-{})...", rsi_period + 1, rsi_period);

    let start = std::time::Instant::now();
    let mut cycle = 0u64;

    loop {
        if let Some(dur) = duration_secs {
            if start.elapsed().as_secs() >= dur {
                info!("Duration limit reached.");
                break;
            }
        }

        match engine.run_scan_cycle().await {
            Ok(entries) => {
                if entries > 0 {
                    info!("[MEAN_REV] Cycle {} — {} new entries", cycle, entries);
                }
            }
            Err(e) => tracing::warn!("[MEAN_REV] Scan error: {}", e),
        }

        if cycle % 20 == 0 {
            engine.save_to_ledger(&ledger);
        }

        cycle += 1;
        tokio::time::sleep(std::time::Duration::from_secs(poll_interval_secs)).await;
    }

    engine.save_to_ledger(&ledger);
    let port = engine.get_portfolio_state();
    println!("\n{}", "--- MEAN REVERSION SESSION SUMMARY ---".bold());
    println!("  Trades: {} | W/L: {}/{} | PnL: {:.4} SOL | ROI: {:.1}%",
        port.total_trades, port.winning_trades, port.losing_trades,
        port.realized_pnl_sol, port.return_on_investment_pct());

    Ok(())
}
