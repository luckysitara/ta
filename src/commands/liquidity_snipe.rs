use anyhow::Result;
use colored::Colorize;
use tracing::info;

use crate::solana::execution::OnChainExecutor;
use crate::solana::ledger::Ledger;
use crate::solana::liquidity_snipe::{LiquiditySnipeEngine, LiquiditySnipeSettings};
use crate::solana::telegram::TelegramNotifier;

pub async fn run_liquidity_snipe(
    budget: f64,
    buy_amount: f64,
    max_positions: usize,
    jito_tip: f64,
    min_liquidity_usd: f64,
    max_pool_age_mins: f64,
    stop_loss_pct: f64,
    take_profit_pct: f64,
    trailing_stop_pct: f64,
    break_even_mult: f64,
    target_dexes: Vec<String>,
    poll_interval_secs: u64,
    duration_secs: Option<u64>,
    onchain: bool,
    keypair_path: Option<String>,
    rpc_url: Option<String>,
) -> Result<()> {
    println!("{}", "================================================================================".cyan());
    println!("{}", "  💧 METEORA / ORCA LIQUIDITY SNIPE ENGINE                                      ".bold().cyan());
    println!("{}", "================================================================================".cyan());

    let settings = LiquiditySnipeSettings {
        total_budget_sol: budget,
        buy_amount_sol: buy_amount,
        max_concurrent_positions: max_positions,
        jito_tip_sol: jito_tip,
        min_liquidity_usd,
        max_pool_age_mins,
        stop_loss_pct,
        take_profit_pct,
        trailing_stop_pct,
        break_even_mult,
        poll_interval_secs,
        is_paper_trading: !onchain,
        target_dexes: target_dexes.clone(),
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

    let engine = LiquiditySnipeEngine::new(settings, executor);
    let ledger = Ledger::from_env_or_default();
    let _notifier = TelegramNotifier::from_env();

    let mode = if onchain { "LIVE" } else { "PAPER" };
    println!("  Mode:              {}", mode.bold());
    println!("  Budget:            {:.4} SOL", budget);
    println!("  Target DEXes:      {}", target_dexes.join(", "));
    println!("  Max Pool Age:      {:.0} minutes", max_pool_age_mins);
    println!("  Min Liquidity:     ${:.0}", min_liquidity_usd);
    println!("  Stop Loss:         {:.1}%", stop_loss_pct);
    println!("  Take Profit:       +{:.1}%", take_profit_pct);
    println!("{}", "================================================================================".cyan());

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
                    info!("[LIQ_SNIPE] Cycle {} — {} new entries", cycle, entries);
                }
            }
            Err(e) => tracing::warn!("[LIQ_SNIPE] Scan error: {}", e),
        }

        if cycle % 20 == 0 {
            engine.save_to_ledger(&ledger);
        }

        cycle += 1;
        tokio::time::sleep(std::time::Duration::from_secs(poll_interval_secs)).await;
    }

    engine.save_to_ledger(&ledger);
    let port = engine.get_portfolio_state();
    println!("\n{}", "--- LIQUIDITY SNIPE SESSION SUMMARY ---".bold());
    println!("  Trades: {} | W/L: {}/{} | PnL: {:.4} SOL | ROI: {:.1}%",
        port.total_trades, port.winning_trades, port.losing_trades,
        port.realized_pnl_sol, port.return_on_investment_pct());

    Ok(())
}
