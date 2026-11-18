use anyhow::Result;
use colored::Colorize;

use crate::solana::wallet_analyzer::WalletAnalyzer;

const DEFAULT_WALLETS: &[(&str, &str)] = &[
    ("CyaE1VxvBrahnPWkqm5VsdCvyS2QmNht2UFrKJHga54o", "Cented"),
    ("Bi4rd5FH5bYEN8scZ7wevxNZyNmKHdaBcvewdPFxYdLt", "Theo"),
    ("2fg5QD1eD7rzNNCsvnhmXFm5hqNgwTTG8p7kQ6f3rx6f", "Cupsey"),
    ("4vw54BmAogeRV3vPKWyFet5yf8DTLcREzdSzx4rw9Ud9", "Decu"),
    ("J6TDXvarvpBdPXTaTU8eJbtso1PUCYKGkVtMKUUY8iEa", "Pain"),
    ("B32QbbdDAyhvUQzjcaM5j6ZVKwjCxAwGH5Xgvb9SJqnC", "Kadenox"),
    ("ardinRsN1mNYVeoJWTBsWeYeXvuR9UUDGMsCDKpb6AT", "Trunoest"),
    ("BTf4A2exGK9BCVDNzy65b9dUzXgMqB4weVkvTMFQsadd", "Kev"),
    ("G6fUXjMKPJzCY1rveAE6Qm7wy5U3vZgKDJmN1VPAdiZC", "Clukz"),
    ("Av3xWHJ5EsoLZag6pr7LKbrGgLRTaykXomDD5kBhL9YQ", "Heyitsyolo"),
    ("4BdKaxN8G6ka4GYtQQWk4G4dZRUTX2vQH9GcXdBREFUk", "Jijo"),
    ("BCagckXeMChUKrHEd6fKFA1uiWDtcmCXMsqaheLiUPJd", "Dv"),
    ("8rvAsDKeAcEjEkiZMug9k8v1y8mW6gQQiMobd89Uy7qR", "Casino"),
];

pub async fn run_wallet_analyzer(
    wallets: Option<String>,
    max_transactions: usize,
    min_win_rate: f64,
    min_trades: usize,
    rpc_url: Option<String>,
) -> Result<()> {
    println!("{}", "================================================================================".cyan());
    println!("{}", "  WALLET ANALYZER — ON-CHAIN TRANSACTION ANALYSIS                               ".bold().cyan());
    println!("{}", "================================================================================".cyan());

    let helius_key = std::env::var("HELIUS_API_KEY").unwrap_or_default();
    if helius_key.is_empty() {
        println!("  {} HELIUS_API_KEY not set — analysis requires Helius parsed transactions API", "WARNING:".yellow().bold());
        println!("  Get a free key at https://helius.dev");
        anyhow::bail!("HELIUS_API_KEY required for wallet analysis");
    }

    let wallet_list: Vec<(String, String)> = if let Some(ref w) = wallets {
        w.split(',').map(|entry| {
            let parts: Vec<&str> = entry.split(':').collect();
            if parts.len() == 2 {
                (parts[0].trim().to_string(), parts[1].trim().to_string())
            } else {
                let addr = parts[0].trim();
                let label = if addr.len() > 8 { &addr[..8] } else { addr };
                (addr.to_string(), label.to_string())
            }
        }).collect()
    } else {
        DEFAULT_WALLETS.iter()
            .map(|(a, l)| (a.to_string(), l.to_string()))
            .collect()
    };

    println!("  Wallets to analyze: {}", wallet_list.len());
    println!("  Max txns/wallet:    {}", max_transactions);
    println!("  Min win rate:       {:.0}%", min_win_rate);
    println!("  Min trades:         {}", min_trades);
    println!("{}", "================================================================================".cyan());
    println!("  Fetching on-chain data from Helius...\n");

    let analyzer = WalletAnalyzer::new(rpc_url, max_transactions);
    let results = analyzer.analyze_all(&wallet_list).await;

    WalletAnalyzer::print_report(&results);

    let recommended = WalletAnalyzer::recommend_for_copy_trade(&results, min_win_rate, min_trades);

    if !recommended.is_empty() {
        println!("\n{}", "  RECOMMENDED FOR COPY TRADING".bold().green());
        println!("  (PnL > 0, win rate >= {:.0}%, trades >= {})\n", min_win_rate, min_trades);

        let addrs: Vec<&str> = recommended.iter().map(|w| w.address.as_str()).collect();
        println!("  Run copy-trade with these wallets:");
        println!("  {}", "─".repeat(70));
        println!("  cargo run -- copy-trade \\");
        println!("    --wallets \"{}\" \\", addrs.join(","));
        println!("    --min-convergence 2 \\");
        println!("    --budget 0.2 --buy-amount 0.03");
        println!();

        for w in &recommended {
            println!("    {} — {:+.4} SOL | {:.1}% WR | {} trades",
                w.label.bold(), w.net_pnl_sol, w.win_rate,
                w.total_buys + w.total_sells);
        }
    } else {
        println!("\n  {} No wallets met the criteria (PnL>0, WR>={:.0}%, trades>={}).",
            "NOTE:".yellow(), min_win_rate, min_trades);
        println!("  Try lowering --min-win-rate or --min-trades.");
    }

    println!("{}", "\n================================================================================".cyan());

    Ok(())
}
