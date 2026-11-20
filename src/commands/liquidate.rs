use colored::*;

use crate::solana::execution::OnChainExecutor;

pub async fn run_liquidate(mint_filter: Option<String>) -> anyhow::Result<()> {
    let keypair_path = std::env::var("KEYPAIR_PATH").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{}/.config/solana/id.json", home)
    });
    let rpc_url = std::env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let jito_url = std::env::var("JITO_BLOCK_ENGINE_URL")
        .unwrap_or_else(|_| "https://amsterdam.mainnet.block-engine.jito.wtf".to_string());

    let executor = OnChainExecutor::from_keypair_file(
        &keypair_path,
        Some(rpc_url),
        Some(jito_url),
    )?;

    let target_mints = if let Some(m) = mint_filter {
        vec![m]
    } else {
        // Discover open token accounts from wallet automatically
        let tokens = executor.get_open_meme_tokens().await.unwrap_or_default();
        if tokens.is_empty() {
            println!("  No open meme token positions found in wallet. Nothing to liquidate.");
            return Ok(());
        }
        tokens.into_iter().map(|(mint, _, _)| mint).collect()
    };

    println!("{}", "================================================================================".yellow());
    println!("{}", "       TRADINGAGENTS-RS: LIQUIDATING OPEN POSITIONS TO RECLAIM SOL              ".bold().yellow());
    println!("{}", "================================================================================".yellow());

    let start_bal = executor.get_balance_sol().await.unwrap_or(0.0);
    println!("  Initial SOL Balance: {:.4} SOL", start_bal);
    println!("  Tokens to liquidate: {}", target_mints.len());

    for m in &target_mints {
        println!("  Attempting to sell / close ATA: {}...", m);
        let mut sold = false;

        match executor.execute_jupiter_swap_jito(m, "So11111111111111111111111111111111111111112", 0, 0.0003, 300).await {
            Ok(res) => {
                println!("    Submitted Jupiter DEX swap: {}", res.signature);
                if let Ok(confirmed) = executor.poll_signature_confirmation(&res.signature, 15, 1000).await {
                    if confirmed {
                        println!("    Swap confirmed successfully on-chain!");
                        sold = true;
                    } else {
                        println!("    Swap pending confirmation...");
                    }
                }
            }
            Err(e) => {
                println!("    Jupiter swap returned: {}. Trying Pump.fun bonding curve...", e);
            }
        }

        if !sold {
            match executor.execute_snipe_sell_jito(m, "", None, 0, 1, 0.0008, true).await {
                Ok(res) => {
                    println!("    Submitted Pump.fun sell: {}", res.signature);
                    let _ = executor.poll_signature_confirmation(&res.signature, 10, 1000).await;
                }
                Err(e) => {
                    println!("    Pump.fun sell returned: {}. Proceeding to close ATA...", e);
                }
            }
        }

        // Wait briefly for balance to update before closing ATA
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

        match executor.close_token_account_jito(m, 0.0003, false).await {
            Ok(res2) => {
                println!("    SPL ATA closed, rent reclaimed: {}", res2.signature);
                let _ = executor.poll_signature_confirmation(&res2.signature, 10, 500).await;
            }
            Err(_) => {
                if let Ok(res2) = executor.close_token_account_jito(m, 0.0003, true).await {
                    println!("    Token-2022 ATA closed, rent reclaimed: {}", res2.signature);
                    let _ = executor.poll_signature_confirmation(&res2.signature, 10, 500).await;
                }
            }
        }
    }

    let end_bal = executor.get_balance_sol().await.unwrap_or(0.0);
    println!("\n  Final SOL Balance: {:.4} SOL (+{:.4} SOL recovered)", end_bal, end_bal - start_bal);
    println!("{}", "================================================================================".yellow());

    Ok(())
}
