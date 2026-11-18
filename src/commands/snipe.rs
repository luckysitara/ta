use colored::*;
use std::sync::Arc;
use tracing::info;

use crate::solana::execution::OnChainExecutor;
use crate::solana::ledger::Ledger;
use crate::solana::rug_filter::{RugFilterConfig, SafetyStatus};
use crate::solana::sniper::{SniperSettings, SolanaSniperEngine};
use crate::solana::telegram::{TelegramNotifier, TradeAlert};
use crate::solana::yellowstone::{DexProtocol, YellowstoneListener};

#[allow(clippy::too_many_arguments)]
pub async fn run_sniper(
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
                println!("  Warning: Unrecognized preset mode '{}'. Proceeding with specified flags.", other);
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
        (_, _, Some(pct), None) => format!("5. Balanced Scalper Mode (>=={:.0}% Curve Completion)", pct).bold().magenta(),
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
    println!("  Liquidity Threshold: >= {} SOL initial pool liquidity", min_liquidity.to_string().bold().cyan());
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
            println!("  [OPT 1] Curve Momentum:  Requires >= {:.1}% curve completion (>= {:.1} SOL reserves)", min_curve, (85.0 * min_curve) / 100.0);
        }
    }
    if let Some(min_buyers) = eff_min_unique_buyers {
        println!("  [OPT 3] Unique Buyers:   Requires >= {} unique wallets (Anti-Insider Whale Filter)", min_buyers.to_string().bold().green());
    }
    if let Some(min_buys) = eff_min_buys_recent {
        println!("  [OPT 1] Buy Velocity:    Requires >= {} buys & <= {} sells in last {}s (Anti-Stall Filter)", min_buys.to_string().bold().green(), eff_max_sells_recent.unwrap_or(0).to_string().bold().green(), eff_velocity_window_secs.unwrap_or(15).to_string().bold().green());
    }
    println!("  [OPT B] Pure Fair Launch: {}", if require_dev_sold_out.unwrap_or(true) { "ENABLED (0.0 SOL Dev Buy Required, 0% Dumped Coins Permitted)".bold().green() } else { "DISABLED".yellow() });
    println!("  [OPT A] Raydium Sniping:  {}", if enable_raydium.unwrap_or(true) { "ENABLED (Raydium AMM v4 & CPMM Graduations via Jupiter/Jito)".bold().green() } else { "Pump.fun Only".yellow() });
    if let Some(dur) = duration_secs {
        println!("  Session Timer:       {} seconds ({} min auto-stop & audit)", dur, dur / 60);
    }
    println!("  Fast-Path Filter:    Freeze Revoked + Mint Revoked + LP Burn Verification");
    let be_info = eff_break_even_mult.map(|b| format!(" | Break-Even Floor: >={:.2}x", b)).unwrap_or_default();
    println!("  Exit Brackets:       TP1: {:.2}x (70%) | TP2: {:.2}x (20%) | Trailing Stop ({:.1}%){}", eff_tp1, eff_tp2, eff_stop_loss, be_info);
    println!("  Target Protocols:    Pump.fun | Raydium AMM v4 | Raydium CPMM | Meteora DLMM");

    // Initialize Telegram notifier
    let notifier = TelegramNotifier::from_env();
    if notifier.is_some() {
        println!("  Telegram Alerts:     {}", "ENABLED".bold().green());
    }

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
                        println!("  Retrying wallet balance query via RPC fallback (attempt {}/3)...", attempt);
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
                            eprintln!("\n{}", "INSUFFICIENT ON-CHAIN FUNDS - Cannot proceed with live trading!".bold().red());
                            eprintln!("  Wallet {} balance: {:.4} SOL", pubkey, bal);
                            eprintln!("  Required per snipe: {:.4} SOL ({:.3} buy + {:.4} Jito tip).", min_needed, buy_amount, jito_tip);
                            return Ok(());
                        }
                        Some(Arc::new(exec))
                    }
                    Err(e) => {
                        eprintln!("\n{}", format!("Failed to query wallet balance via RPC: {}", e).bold().red());
                        return Ok(());
                    }
                }
            }
            Err(e) => {
                eprintln!("\n{}", format!("Failed to load keypair from '{}': {}", kp_path, e).bold().red());
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

    // Restore state from ledger if available
    let ledger = Ledger::from_env_or_default();
    if let Ok(Some((saved_portfolio, saved_positions))) = ledger.load_sniper_state() {
        let open_count = saved_positions.values().filter(|p| !p.is_closed).count();
        if open_count > 0 {
            println!("  Ledger Restore:      {} open positions recovered from {}", open_count.to_string().bold().green(), ledger.path().display());
            engine.restore_state(saved_portfolio, saved_positions);
        }
    }

    engine.print_portfolio_hud();

    if simulate || resolved_endpoint.is_none() {
        println!("\n{}", "--- RUNNING YELLOWSTONE STREAM INGESTION SIMULATION ---".bold().magenta());
        if resolved_endpoint.is_none() {
            println!("  (No active Yellowstone gRPC URL found. Running high-speed simulation & replay benchmark)");
        }

        let pump_event = crate::solana::yellowstone::PoolCreationEvent {
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

        println!("\n{}", "[EVENT 1] Stream Item: Slot 298,401,912 | Program: Pump.fun".cyan());
        let rep1 = engine.process_detected_pool(&pump_event);
        if let Some(r) = rep1 {
            println!("  Latency:             {} us", r.execution_latency_us.to_string().bold().green());
            let jito_bundle = engine.build_jito_bundle_plan();
            println!("  MEV Protection:      Jito Tip Account: {} ({} lamports)", &jito_bundle.tip_account[..12], jito_bundle.tip_lamports);
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        println!("\n{}", "[EVENT 2] Stream Item: Slot 298,401,914 | Program: Raydium AMM".red());
        let honeypot_filter = crate::solana::rug_filter::RugFilter::new(RugFilterConfig::default());
        let rep2 = honeypot_filter.evaluate_fast(
            "BadHoneyPotToken1111111111111111111111111",
            false, 10.0, false, true, 2.0, false, None,
        );
        if let SafetyStatus::Rejected(ref reason) = rep2.status {
            println!("  Security Alert:      {}", "REJECTED (Honeypot Protection Triggered)".bold().red());
            println!("  Reason:              {}", reason.red());
        }

        println!("\n{}", "--- ADVANCING POSITION LIFECYCLE TICKER ---".bold().cyan());
        for tick in 1..=6 {
            tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
            println!("  [TICK {}] Updating bonding curve prices...", tick);
            engine.tick_position_updates();
        }

        // Save state to ledger
        engine.save_to_ledger(&ledger);
        engine.print_session_summary();

        // Send session summary via Telegram
        if let Some(ref notifier) = notifier {
            let port = engine.get_portfolio_state();
            notifier.send(TradeAlert::SessionSummary {
                engine: "Sniper".to_string(),
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
            eprintln!("\n{}", format!("Invalid Yellowstone gRPC endpoint: '{}'", live_endpoint).bold().red());
            return Ok(());
        }
    }

    if let Ok(parsed) = reqwest::Url::parse(&live_endpoint) {
        let path = parsed.path().trim_start_matches('/').trim();
        if !path.is_empty() {
            if resolved_token.is_none() {
                resolved_token = Some(path.to_string());
                println!("  Extracted Yellowstone authentication token from URL path.");
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

    // Spawn background position lifecycle ticker + periodic ledger saves
    let ticker_engine = Arc::clone(&engine);
    let ledger_path = ledger.path().to_path_buf();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1500));
        let mut tick_counter: u64 = 0;
        let save_ledger = Ledger::new(&ledger_path);
        loop {
            interval.tick().await;
            ticker_engine.tick_position_updates();
            tick_counter += 1;
            if tick_counter % 5 == 0 {
                ticker_engine.sync_live_wallet_balance().await;
            }
            // Save ledger every ~30 seconds (20 ticks * 1.5s)
            if tick_counter % 20 == 0 {
                ticker_engine.save_to_ledger(&save_ledger);
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
                println!("{}", format!("  Timer: Completed {}s ({} min) {} session. Shutting down stream...", dur, dur / 60, sess_type).bold().yellow());
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

    // Final save and summary
    engine.save_to_ledger(&ledger);
    engine.print_session_summary();

    if let Some(ref notifier) = notifier {
        let port = engine.get_portfolio_state();
        notifier.send(TradeAlert::SessionSummary {
            engine: "Sniper".to_string(),
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
