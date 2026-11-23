use anyhow::Result;
use colored::Colorize;
use tracing::info;

use crate::solana::execution::OnChainExecutor;
use crate::solana::ledger::Ledger;
use crate::solana::route_arb::{ArbToken, RouteArbEngine, RouteArbSettings};
use crate::solana::telegram::TelegramNotifier;

pub async fn run_route_arb(
    budget: f64,
    trade_amount: f64,
    min_spread_bps: u64,
    jito_tip: f64,
    tokens: Vec<String>,
    poll_interval_secs: u64,
    duration_secs: Option<u64>,
    onchain: bool,
    keypair_path: Option<String>,
    rpc_url: Option<String>,
    jito_url: Option<String>,
    use_flash_loan: bool,
    flash_loan_amount: f64,
) -> Result<()> {
    println!("{}", "================================================================================".cyan());
    println!("{}", "  ⚡ JUPITER ROUTE ARBITRAGE ENGINE                                              ".bold().cyan());
    println!("{}", "================================================================================".cyan());

    let target_tokens = if tokens.is_empty() {
        RouteArbSettings::default().target_tokens
    } else {
        tokens.iter().map(|t| {
            let parts: Vec<&str> = t.split(':').collect();
            if parts.len() == 2 {
                ArbToken { mint: parts[0].to_string(), symbol: parts[1].to_string() }
            } else {
                ArbToken { mint: t.clone(), symbol: t[..t.len().min(8)].to_string() }
            }
        }).collect()
    };

    let settings = RouteArbSettings {
        total_budget_sol: budget,
        trade_amount_sol: trade_amount,
        min_spread_bps,
        jito_tip_sol: jito_tip,
        target_tokens: target_tokens.clone(),
        poll_interval_secs,
        is_paper_trading: !onchain,
        use_flash_loan,
        flash_loan_amount_sol: flash_loan_amount,
        ..Default::default()
    };

    let executor = if onchain {
        let kp = keypair_path.unwrap_or_else(|| {
            std::env::var("KEYPAIR_PATH").unwrap_or_else(|_| "~/.config/solana/id.json".to_string())
        });
        let rpc = rpc_url.unwrap_or_else(|| {
            std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string())
        });
        Some(OnChainExecutor::from_keypair_file(&kp, Some(rpc), jito_url)?)
    } else {
        None
    };

    let engine = RouteArbEngine::new(settings, executor);
    let ledger = Ledger::from_env_or_default();
    let _notifier = TelegramNotifier::from_env();

    let mode = if onchain { "LIVE" } else { "PAPER" };
    println!("  Mode:              {}", mode.bold());
    println!("  Flash Loan:        {} ({:.2} SOL)", if use_flash_loan { "ENABLED (Kamino)".green().bold() } else { "DISABLED".yellow() }, flash_loan_amount);
    println!("  Budget:            {:.4} SOL", budget);
    println!("  Trade Size:        {:.4} SOL", trade_amount);
    println!("  Min Spread:        {} bps ({:.2}%)", min_spread_bps, min_spread_bps as f64 / 100.0);
    println!("  Tokens:            {}", target_tokens.iter().map(|t| t.symbol.as_str()).collect::<Vec<_>>().join(", "));
    println!("  Poll Interval:     {}s", poll_interval_secs);
    println!("{}", "================================================================================".cyan());

    let start = std::time::Instant::now();
    let mut cycle = 0u64;
    let mut total_arbs = 0;

    loop {
        if let Some(dur) = duration_secs {
            if start.elapsed().as_secs() >= dur {
                info!("Duration limit reached.");
                break;
            }
        }

        match engine.run_scan_cycle().await {
            Ok(found) => {
                total_arbs += found;
                if found > 0 {
                    info!("[ROUTE_ARB] Cycle {} — {} arbs executed (total: {})", cycle, found, total_arbs);
                }
            }
            Err(e) => tracing::warn!("[ROUTE_ARB] Scan error: {}", e),
        }

        if cycle % 50 == 0 {
            engine.save_to_ledger(&ledger);
        }

        cycle += 1;
        tokio::time::sleep(std::time::Duration::from_secs(poll_interval_secs)).await;
    }

    engine.save_to_ledger(&ledger);
    let port = engine.get_portfolio_state();
    println!("\n{}", "--- ROUTE ARB SESSION SUMMARY ---".bold());
    println!("  Arbs Executed: {} | W/L: {}/{} | PnL: {:.6} SOL | ROI: {:.2}%",
        port.total_trades, port.winning_trades, port.losing_trades,
        port.realized_pnl_sol, port.return_on_investment_pct());

    Ok(())
}
