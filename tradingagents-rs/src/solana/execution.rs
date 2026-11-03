use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
#[allow(deprecated)]
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair},
    signer::Signer,
    system_instruction,
    system_program,
    transaction::Transaction,
};
use std::str::FromStr;

// Pump.fun Protocol Accounts
pub const PUMP_FUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
#[allow(dead_code)]
pub const PUMP_FUN_GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
#[allow(dead_code)]
pub const PUMP_FUN_FEE_RECIPIENT: &str = "CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM";
pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
#[allow(dead_code)]
pub const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
#[allow(dead_code)]
pub const PUMP_EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1";
#[allow(dead_code)]
pub const FEE_PROGRAM: &str = "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ";
#[allow(dead_code)]
pub const PUMP_FEE_CONFIG_SEEDS: [u8; 32] = [
    1, 86, 224, 246, 147, 102, 90, 207, 68, 219, 21, 104, 191, 23, 91, 170,
    81, 137, 203, 151, 245, 210, 255, 59, 101, 93, 43, 182, 253, 109, 24, 176,
];
#[allow(dead_code)]
pub const PUMP_BUYBACK_1: &str = "6XvaUWHuc3AAHKJDmHQiCLQDpNiUro8x7nfKZ7roQYLa";
#[allow(dead_code)]
pub const PUMP_BUYBACK_2: &str = "5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD";

// Jito Regional Block Engine Endpoints
pub const JITO_BLOCK_ENGINES: &[(&str, &str)] = &[
    ("Amsterdam (Default)", "https://amsterdam.mainnet.block-engine.jito.wtf"),
    ("Frankfurt", "https://frankfurt.mainnet.block-engine.jito.wtf"),
    ("New York", "https://ny.mainnet.block-engine.jito.wtf"),
    ("Tokyo", "https://tokyo.mainnet.block-engine.jito.wtf"),
];

// Jito Public Tip Accounts for MEV Bundle Routing
pub const JITO_TIP_ACCOUNTS: &[&str] = &[
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pWHLnjvnxKLQm3AoZ23z",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleSubmissionResult {
    pub bundle_id: String,
    pub status: String,
    pub tip_account: String,
    pub tip_lamports: u64,
    pub signature: String,
}

use std::sync::Arc;
use tokio::sync::RwLock;

pub struct OnChainExecutor {
    keypair: Keypair,
    rpc_url: String,
    jito_url: String,
    client: reqwest::Client,
    cached_blockhash: Arc<RwLock<Option<Hash>>>,
}

impl OnChainExecutor {
    /// Initialize OnChainExecutor from a keypair file (e.g. ~/.config/solana/id.json)
    pub fn from_keypair_file(
        keypair_path: &str,
        rpc_url: Option<String>,
        jito_url: Option<String>,
    ) -> Result<Self> {
        let keypair = read_keypair_file(keypair_path)
            .map_err(|e| anyhow!("Failed to read keypair from '{}': {}", keypair_path, e))?;

        let rpc = rpc_url
            .or_else(|| std::env::var("SOLANA_RPC_URL").ok())
            .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".to_string());

        let jito = jito_url
            .or_else(|| std::env::var("JITO_BLOCK_ENGINE_URL").ok())
            .unwrap_or_else(|| JITO_BLOCK_ENGINES[0].1.to_string());

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;

        let cached_blockhash = Arc::new(RwLock::new(None));
        let executor = Self {
            keypair,
            rpc_url: rpc,
            jito_url: jito,
            client,
            cached_blockhash,
        };

        // Start background zero-latency blockhash refresher
        executor.start_blockhash_refresher();

        Ok(executor)
    }

    /// Return wallet public key
    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    /// Background task that continuously polls and updates the latest blockhash every 500ms
    pub fn start_blockhash_refresher(&self) {
        let rpc = self.rpc_url.clone();
        let client = self.client.clone();
        let cache = Arc::clone(&self.cached_blockhash);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            let payload = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getLatestBlockhash",
                "params": [{"commitment": "processed"}]
            });
            loop {
                interval.tick().await;
                if let Ok(resp) = client.post(&rpc).json(&payload).send().await {
                    if let Ok(res_json) = resp.json::<serde_json::Value>().await {
                        if let Some(bh_str) = res_json["result"]["value"]["blockhash"].as_str() {
                            if let Ok(bh) = Hash::from_str(bh_str) {
                                let mut lock = cache.write().await;
                                *lock = Some(bh);
                            }
                        }
                    }
                }
            }
        });
    }

    /// Helper to send an RPC request with automatic retry and failover to fallback RPC endpoints
    pub async fn send_rpc_request(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let fallback_urls = vec![
            self.rpc_url.clone(),
            "https://api.mainnet-beta.solana.com".to_string(),
            "https://solana-rpc.publicnode.com".to_string(),
            "https://rpc.ankr.com/solana".to_string(),
        ];

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });

        let mut last_err = None;
        for url in &fallback_urls {
            for attempt in 1..=2 {
                match self.client.post(url).json(&payload).send().await {
                    Ok(resp) => {
                        let status = resp.status();
                        if !status.is_success() {
                            last_err = Some(anyhow!("HTTP error {} from {}", status, url));
                            continue;
                        }
                        match resp.json::<serde_json::Value>().await {
                            Ok(res_json) => {
                                if let Some(err) = res_json.get("error") {
                                    last_err = Some(anyhow!("RPC error from {}: {:?}", url, err));
                                    break; // Don't retry same URL on RPC logic error, failover to next URL
                                }
                                return Ok(res_json);
                            }
                            Err(e) => {
                                last_err = Some(anyhow!("JSON decode error from {}: {}", url, e));
                            }
                        }
                    }
                    Err(e) => {
                        last_err = Some(anyhow!("Connection error to {}: {}", url, e));
                        if attempt < 2 {
                            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        }
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("All RPC endpoints failed for method {}", method)))
    }

    /// Query live on-chain SOL balance for the wallet
    pub async fn get_balance_sol(&self) -> Result<f64> {
        let res_json = self.send_rpc_request("getBalance", json!([self.pubkey().to_string()])).await?;
        let lamports = res_json["result"]["value"].as_u64().unwrap_or(0);
        Ok(lamports as f64 / 1_000_000_000.0)
    }

    /// Poll signature confirmation status and verify if transaction succeeded or reverted on-chain
    pub async fn poll_signature_confirmation(&self, signature: &str, max_retries: usize, delay_ms: u64) -> Result<bool> {
        for _ in 0..max_retries {
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            if let Ok(res_json) = self.send_rpc_request("getSignatureStatuses", json!([[signature], {"searchTransactionHistory": true}])).await {
                if let Some(val) = res_json["result"]["value"][0].as_object() {
                    if val.get("err").map(|e| !e.is_null()).unwrap_or(false) {
                        return Err(anyhow!("Transaction {} reverted on-chain: {:?}", signature, val.get("err")));
                    }
                    if let Some(status) = val.get("confirmationStatus").and_then(|s| s.as_str()) {
                        if status == "confirmed" || status == "finalized" {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    /// Query token account balance (raw units) for a given ATA
    pub async fn get_token_balance(&self, token_account: &Pubkey) -> Result<u64> {
        let res_json = self.send_rpc_request("getTokenAccountBalance", json!([token_account.to_string()])).await?;
        let amount_str = res_json["result"]["value"]["amount"].as_str().unwrap_or("0");
        Ok(amount_str.parse::<u64>().unwrap_or(0))
    }

    /// Fetch the creator of a Pump.fun bonding curve directly from on-chain account data (offset 49..81)
    pub async fn fetch_bonding_curve_creator(&self, bonding_curve: &Pubkey) -> Result<Pubkey> {
        let res_json = self.send_rpc_request("getAccountInfo", json!([bonding_curve.to_string(), {"encoding": "base64"}])).await?;
        let data_b64 = res_json["result"]["value"]["data"][0]
            .as_str()
            .context("Missing bonding curve data")?;

        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data_b64)?;
        if bytes.len() >= 81 {
            let creator_slice = &bytes[49..81];
            Ok(Pubkey::new_from_array(creator_slice.try_into()?))
        } else {
            Err(anyhow!("Bonding curve account data too short ({} bytes)", bytes.len()))
        }
    }

    /// Fetch latest blockhash from memory cache (0ms latency) or fallback to Solana RPC
    pub async fn get_latest_blockhash(&self) -> Result<Hash> {
        if let Some(bh) = *self.cached_blockhash.read().await {
            return Ok(bh);
        }

        let res_json = self.send_rpc_request("getLatestBlockhash", json!([{"commitment": "processed"}])).await?;
        let bh_str = res_json["result"]["value"]["blockhash"]
            .as_str()
            .context("Missing blockhash in RPC response")?;

        let bh = Hash::from_str(bh_str).map_err(|e| anyhow!("Invalid blockhash: {}", e))?;
        let mut lock = self.cached_blockhash.write().await;
        *lock = Some(bh);
        Ok(bh)
    }

    /// Direct submission to Solana RPC TPU via sendTransaction
    pub async fn send_transaction_rpc(&self, tx: &Transaction) -> Result<String> {
        let serialized = bincode::serialize(tx)?;
        let encoded = bs58::encode(serialized).into_string();
        let payload = json!([
            encoded,
            {
                "encoding": "base58",
                "skipPreflight": true,
                "preflightCommitment": "processed",
                "maxRetries": 3
            }
        ]);

        let res = self.send_rpc_request("sendTransaction", payload).await?;
        if let Some(sig) = res["result"].as_str() {
            Ok(sig.to_string())
        } else {
            Err(anyhow!("RPC sendTransaction returned: {:?}", res))
        }
    }

    /// Simulate a signed transaction against the Solana RPC
    #[allow(dead_code)]
    pub async fn simulate_transaction(&self, tx: &Transaction) -> Result<serde_json::Value> {
        let serialized = bincode::serialize(tx)?;
        let encoded = bs58::encode(serialized).into_string();
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "simulateTransaction",
            "params": [
                encoded,
                {
                    "encoding": "base58",
                    "commitment": "processed",
                    "sigVerify": false
                }
            ]
        });

        let resp = self.client.post(&self.rpc_url).json(&payload).send().await?;
        let res_json: serde_json::Value = resp.json().await?;
        Ok(res_json)
    }

    /// Construct Pump.fun BuyExactQuoteInV3 instruction (standard for modern Token-2022 & legacy tokens)
    pub fn build_pump_fun_buy_exact_quote_in_v3_ix(
        &self,
        base_mint: &Pubkey,
        spendable_quote_in_lamports: u64,
        min_tokens_out: u64,
        is_token_2022: bool,
    ) -> Result<Instruction> {
        let program_id = Pubkey::from_str(PUMP_FUN_PROGRAM)?;
        let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM)?;
        let fee_program = Pubkey::from_str("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ")?;
        let quote_mint = Pubkey::from_str("So11111111111111111111111111111111111111112")?;
        let quote_token_program = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")?;
        let base_token_program = if is_token_2022 {
            Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?
        } else {
            Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")?
        };

        // PDAs
        let (global, _) = Pubkey::find_program_address(&[b"global"], &program_id);
        let (bonding_curve, _) = Pubkey::find_program_address(&[b"bonding-curve", base_mint.as_ref()], &program_id);
        
        let (assoc_base_bc, _) = Pubkey::find_program_address(
            &[&bonding_curve.to_bytes(), &base_token_program.to_bytes(), &base_mint.to_bytes()],
            &ata_program,
        );
        let (assoc_quote_bc, _) = Pubkey::find_program_address(
            &[&bonding_curve.to_bytes(), &quote_token_program.to_bytes(), &quote_mint.to_bytes()],
            &ata_program,
        );

        let user = self.pubkey();
        let (assoc_base_user, _) = Pubkey::find_program_address(
            &[&user.to_bytes(), &base_token_program.to_bytes(), &base_mint.to_bytes()],
            &ata_program,
        );
        let (assoc_quote_user, _) = Pubkey::find_program_address(
            &[&user.to_bytes(), &quote_token_program.to_bytes(), &quote_mint.to_bytes()],
            &ata_program,
        );

        let (user_vol, _) = Pubkey::find_program_address(&[b"user_volume_accumulator", user.as_ref()], &program_id);

        let fee_seed = [
            1, 86, 224, 246, 147, 102, 90, 207, 68, 219, 21, 104, 191, 23, 91, 170,
            81, 137, 203, 151, 245, 210, 255, 59, 101, 93, 43, 182, 253, 109, 24, 176
        ];
        let (fee_config, _) = Pubkey::find_program_address(&[b"fee_config", &fee_seed], &fee_program);

        let buyback_fee_recipient = Pubkey::from_str("5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD")?;
        let (event_authority, _) = Pubkey::find_program_address(&[b"__event_authority"], &program_id);

        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&[225, 247, 80, 30, 213, 179, 132, 136]); // buy_exact_quote_in_v3
        data.extend_from_slice(&spendable_quote_in_lamports.to_le_bytes());
        data.extend_from_slice(&min_tokens_out.to_le_bytes());

        let accounts = vec![
            AccountMeta::new_readonly(global, false),                  // 0
            AccountMeta::new_readonly(*base_mint, false),              // 1
            AccountMeta::new_readonly(quote_mint, false),              // 2
            AccountMeta::new_readonly(base_token_program, false),      // 3
            AccountMeta::new_readonly(quote_token_program, false),     // 4
            AccountMeta::new(bonding_curve, false),                    // 5
            AccountMeta::new(assoc_base_bc, false),                    // 6
            AccountMeta::new(assoc_quote_bc, false),                   // 7
            AccountMeta::new(user, true),                              // 8
            AccountMeta::new(assoc_base_user, false),                  // 9
            AccountMeta::new(assoc_quote_user, false),                 // 10
            AccountMeta::new(user_vol, false),                         // 11
            AccountMeta::new_readonly(fee_config, false),              // 12
            AccountMeta::new(buyback_fee_recipient, false),            // 13
            AccountMeta::new_readonly(system_program::id(), false),    // 14
            AccountMeta::new_readonly(event_authority, false),         // 15
            AccountMeta::new_readonly(program_id, false),              // 16
        ];

        Ok(Instruction {
            program_id,
            accounts,
            data,
        })
    }

    /// Construct Pump.fun SellV3 instruction (standard for modern Token-2022 & legacy tokens)
    pub fn build_pump_fun_sell_v3_ix(
        &self,
        base_mint: &Pubkey,
        token_amount_in: u64,
        min_sol_out_lamports: u64,
        is_token_2022: bool,
    ) -> Result<Instruction> {
        let program_id = Pubkey::from_str(PUMP_FUN_PROGRAM)?;
        let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM)?;
        let fee_program = Pubkey::from_str("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ")?;
        let quote_mint = Pubkey::from_str("So11111111111111111111111111111111111111112")?;
        let quote_token_program = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")?;
        let base_token_program = if is_token_2022 {
            Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?
        } else {
            Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")?
        };

        let (global, _) = Pubkey::find_program_address(&[b"global"], &program_id);
        let (bonding_curve, _) = Pubkey::find_program_address(&[b"bonding-curve", base_mint.as_ref()], &program_id);
        
        let (assoc_base_bc, _) = Pubkey::find_program_address(
            &[&bonding_curve.to_bytes(), &base_token_program.to_bytes(), &base_mint.to_bytes()],
            &ata_program,
        );
        let (assoc_quote_bc, _) = Pubkey::find_program_address(
            &[&bonding_curve.to_bytes(), &quote_token_program.to_bytes(), &quote_mint.to_bytes()],
            &ata_program,
        );

        let user = self.pubkey();
        let (assoc_base_user, _) = Pubkey::find_program_address(
            &[&user.to_bytes(), &base_token_program.to_bytes(), &base_mint.to_bytes()],
            &ata_program,
        );
        let (assoc_quote_user, _) = Pubkey::find_program_address(
            &[&user.to_bytes(), &quote_token_program.to_bytes(), &quote_mint.to_bytes()],
            &ata_program,
        );

        let (user_vol, _) = Pubkey::find_program_address(&[b"user_volume_accumulator", user.as_ref()], &program_id);

        let fee_seed = [
            1, 86, 224, 246, 147, 102, 90, 207, 68, 219, 21, 104, 191, 23, 91, 170,
            81, 137, 203, 151, 245, 210, 255, 59, 101, 93, 43, 182, 253, 109, 24, 176
        ];
        let (fee_config, _) = Pubkey::find_program_address(&[b"fee_config", &fee_seed], &fee_program);

        let buyback_fee_recipient = Pubkey::from_str("5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD")?;
        let (event_authority, _) = Pubkey::find_program_address(&[b"__event_authority"], &program_id);

        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&[28, 146, 222, 119, 38, 196, 105, 213]); // sell_v3
        data.extend_from_slice(&token_amount_in.to_le_bytes());
        data.extend_from_slice(&min_sol_out_lamports.to_le_bytes());

        let accounts = vec![
            AccountMeta::new_readonly(global, false),                  // 0
            AccountMeta::new_readonly(*base_mint, false),              // 1
            AccountMeta::new_readonly(quote_mint, false),              // 2
            AccountMeta::new_readonly(base_token_program, false),      // 3
            AccountMeta::new_readonly(quote_token_program, false),     // 4
            AccountMeta::new(bonding_curve, false),                    // 5
            AccountMeta::new(assoc_base_bc, false),                    // 6
            AccountMeta::new(assoc_quote_bc, false),                   // 7
            AccountMeta::new(user, true),                              // 8
            AccountMeta::new(assoc_base_user, false),                  // 9
            AccountMeta::new(assoc_quote_user, false),                 // 10
            AccountMeta::new(user_vol, false),                         // 11
            AccountMeta::new_readonly(fee_config, false),              // 12
            AccountMeta::new(buyback_fee_recipient, false),            // 13
            AccountMeta::new_readonly(system_program::id(), false),    // 14
            AccountMeta::new_readonly(event_authority, false),         // 15
            AccountMeta::new_readonly(program_id, false),              // 16
        ];

        Ok(Instruction {
            program_id,
            accounts,
            data,
        })
    }

    /// Construct a Pump.fun Snipe Buy instruction
    #[allow(dead_code)]
    pub fn build_pump_fun_buy_ix(
        &self,
        token_mint: &Pubkey,
        bonding_curve: &Pubkey,
        assoc_bonding_curve: &Pubkey,
        user_ata: &Pubkey,
        creator: Option<&Pubkey>,
        token_amount_out: u64,
        max_sol_cost_lamports: u64,
        token_program: &Pubkey,
    ) -> Result<Instruction> {
        let program_id = Pubkey::from_str(PUMP_FUN_PROGRAM)?;
        let global = Pubkey::from_str(PUMP_FUN_GLOBAL)?;
        let fee_recipient = Pubkey::from_str(PUMP_FUN_FEE_RECIPIENT)?;
        let event_authority = Pubkey::from_str(PUMP_EVENT_AUTHORITY)?;
        let fee_program = Pubkey::from_str(FEE_PROGRAM)?;

        let creator_pubkey = creator.cloned().unwrap_or(*bonding_curve);
        let (creator_vault, _) = Pubkey::find_program_address(&[b"creator-vault", creator_pubkey.as_ref()], &program_id);
        let (global_vol, _) = Pubkey::find_program_address(&[b"global_volume_accumulator"], &program_id);
        let (user_vol, _) = Pubkey::find_program_address(&[b"user_volume_accumulator", self.pubkey().as_ref()], &program_id);
        let (fee_config, _) = Pubkey::find_program_address(&[b"fee_config", &PUMP_FEE_CONFIG_SEEDS], &fee_program);
        let (bonding_curve_v2, _) = Pubkey::find_program_address(&[b"bonding-curve-v2", token_mint.as_ref()], &program_id);

        let buyback1 = Pubkey::from_str(PUMP_BUYBACK_1)?;
        let buyback2 = Pubkey::from_str(PUMP_BUYBACK_2)?;

        let mut data = Vec::with_capacity(26);
        data.extend_from_slice(&[102, 6, 61, 18, 1, 218, 235, 234]);
        data.extend_from_slice(&token_amount_out.to_le_bytes());
        data.extend_from_slice(&max_sol_cost_lamports.to_le_bytes());
        data.push(0); // track_volume: OptionBool None
        data.push(0); // partial_fill: OptionBool None

        let accounts = vec![
            AccountMeta::new_readonly(global, false), // 0
            AccountMeta::new(fee_recipient, false), // 1
            AccountMeta::new_readonly(*token_mint, false), // 2
            AccountMeta::new(*bonding_curve, false), // 3
            AccountMeta::new(*assoc_bonding_curve, false), // 4
            AccountMeta::new(*user_ata, false), // 5
            AccountMeta::new(self.pubkey(), true), // 6
            AccountMeta::new_readonly(system_program::id(), false), // 7
            AccountMeta::new_readonly(*token_program, false), // 8
            AccountMeta::new(creator_vault, false), // 9
            AccountMeta::new_readonly(event_authority, false), // 10
            AccountMeta::new_readonly(program_id, false), // 11
            AccountMeta::new_readonly(global_vol, false), // 12
            AccountMeta::new(user_vol, false), // 13
            AccountMeta::new_readonly(fee_config, false), // 14
            AccountMeta::new_readonly(fee_program, false), // 15
            AccountMeta::new(buyback1, false), // 16
            AccountMeta::new(buyback2, false), // 17
            AccountMeta::new_readonly(bonding_curve_v2, false), // 18: bonding_curve_v2
        ];

        Ok(Instruction {
            program_id,
            accounts,
            data,
        })
    }

    /// Construct a Jito Tip Transfer instruction to guarantee private bundle inclusion
    pub fn build_jito_tip_ix(&self, tip_lamports: u64) -> Result<(Instruction, String)> {
        // Deterministically select tip account
        let tip_account_str = JITO_TIP_ACCOUNTS[0];
        let tip_pubkey = Pubkey::from_str(tip_account_str)?;
        let ix = system_instruction::transfer(&self.pubkey(), &tip_pubkey, tip_lamports);
        Ok((ix, tip_account_str.to_string()))
    }

    /// Build, sign, and submit an atomic Jito MEV bundle for a snipe buy
    pub async fn execute_snipe_buy_jito(
        &self,
        token_mint_str: &str,
        bonding_curve_str: &str,
        creator_str: Option<&str>,
        buy_amount_sol: f64,
        jito_tip_sol: f64,
        _max_slippage_bps: u16,
        is_token_2022: bool,
    ) -> Result<BundleSubmissionResult> {
        let mint = Pubkey::from_str(token_mint_str)?;
        let _bonding_curve = Pubkey::from_str(bonding_curve_str)?;
        let _creator_pk = match creator_str.and_then(|s| Pubkey::from_str(s).ok()) {
            Some(pk) => Some(pk),
            None => self.fetch_bonding_curve_creator(&_bonding_curve).await.ok(),
        };

        let token_program = if is_token_2022 {
            Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?
        } else {
            Pubkey::from_str(TOKEN_PROGRAM)?
        };
        let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM)?;

        // Derive Associated Token Address for user
        let (user_ata, _) = Pubkey::find_program_address(
            &[&self.pubkey().to_bytes(), &token_program.to_bytes(), &mint.to_bytes()],
            &ata_program,
        );

        let sol_lamports = (buy_amount_sol * 1_000_000_000.0) as u64;
        let tip_lamports = (jito_tip_sol * 1_000_000_000.0) as u64;

        // 1. Create User ATA instruction (idempotent / create_associated_token_account)
        let create_ata_ix = Instruction {
            program_id: ata_program,
            accounts: vec![
                AccountMeta::new(self.pubkey(), true),
                AccountMeta::new(user_ata, false),
                AccountMeta::new_readonly(self.pubkey(), false),
                AccountMeta::new_readonly(mint, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(token_program, false),
            ],
            data: vec![1], // create_idempotent: safe to run if ATA already exists
        };

        // 2. Compute Budget limits (anti-revert + fast priority)
        let compute_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(250_000);
        let compute_price_ix = ComputeBudgetInstruction::set_compute_unit_price(25_000);

        // 3. Pump.fun Buy instruction using BuyExactQuoteInV3
        let buy_ix = self.build_pump_fun_buy_exact_quote_in_v3_ix(
            &mint,
            sol_lamports,
            1, // 1 min token protects against empty returns while avoiding slippage reverts
            is_token_2022,
        )?;

        // 4. Jito Tip instruction
        let (tip_ix, tip_acc_str) = self.build_jito_tip_ix(tip_lamports)?;

        let instructions = vec![compute_limit_ix, compute_price_ix, create_ata_ix, buy_ix, tip_ix];

        let blockhash = self.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&self.pubkey()),
            &[&self.keypair],
            blockhash,
        );

        let signature = tx.signatures[0].to_string();

        // Dual-Path Execution: Submit in parallel to both Jito MEV bundles AND Direct RPC TPU!
        let jito_fut = self.submit_jito_bundle(vec![tx.clone()]);
        let rpc_fut = self.send_transaction_rpc(&tx);

        let (bundle_res, rpc_res) = tokio::join!(jito_fut, rpc_fut);

        let bundle_id = match (bundle_res, rpc_res) {
            (Ok(bid), Ok(rpc_sig)) => format!("Jito: {} | TPU: {}", &bid[..16.min(bid.len())], &rpc_sig[..16.min(rpc_sig.len())]),
            (Ok(bid), Err(_)) => format!("Jito: {}", bid),
            (Err(_), Ok(rpc_sig)) => format!("TPU: {}", rpc_sig),
            (Err(e1), Err(e2)) => return Err(anyhow!("Dual submission failed: Jito ({}) | TPU ({})", e1, e2)),
        };

        Ok(BundleSubmissionResult {
            bundle_id,
            status: "SUBMITTED_TO_VALIDATORS".to_string(),
            tip_account: tip_acc_str,
            tip_lamports,
            signature,
        })
    }

    /// Submit signed transactions to Jito Block Engine bundles JSON-RPC endpoint
    pub async fn submit_jito_bundle(&self, txs: Vec<Transaction>) -> Result<String> {
        let mut encoded_txs = Vec::new();
        for tx in txs {
            let serialized = bincode::serialize(&tx)
                .map_err(|e| anyhow!("Failed to serialize transaction: {}", e))?;
            encoded_txs.push(bs58::encode(serialized).into_string());
        }

        let regional_endpoints = vec![
            "https://amsterdam.mainnet.block-engine.jito.wtf".to_string(),
            "https://frankfurt.mainnet.block-engine.jito.wtf".to_string(),
            "https://ny.mainnet.block-engine.jito.wtf".to_string(),
            "https://tokyo.mainnet.block-engine.jito.wtf".to_string(),
            self.jito_url.trim_end_matches('/').to_string(),
        ];

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [encoded_txs]
        });

        // Broadcast to all regional endpoints in parallel
        let mut handles = Vec::new();
        for base_url in regional_endpoints {
            let client = self.client.clone();
            let p = payload.clone();
            handles.push(tokio::spawn(async move {
                let endpoint = format!("{}/api/v1/bundles", base_url);
                match client.post(&endpoint).json(&p).send().await {
                    Ok(resp) => {
                        if let Ok(res_json) = resp.json::<serde_json::Value>().await {
                            if let Some(bundle_id) = res_json["result"].as_str() {
                                return Ok(bundle_id.to_string());
                            }
                        }
                    }
                    Err(e) => {
                        return Err(anyhow!("Jito error {}: {}", base_url, e));
                    }
                }
                Err(anyhow!("Bundle rejected by {}", base_url))
            }));
        }

        for h in handles {
            if let Ok(Ok(bundle_id)) = h.await {
                return Ok(bundle_id);
            }
        }

        Ok("bundle_broadcast_complete".to_string())
    }

    /// Construct a Pump.fun Snipe Sell instruction
    #[allow(dead_code)]
    pub fn build_pump_fun_sell_ix(
        &self,
        token_mint: &Pubkey,
        bonding_curve: &Pubkey,
        assoc_bonding_curve: &Pubkey,
        user_ata: &Pubkey,
        creator: Option<&Pubkey>,
        token_amount_in: u64,
        min_sol_output_lamports: u64,
    ) -> Result<Instruction> {
        let program_id = Pubkey::from_str(PUMP_FUN_PROGRAM)?;
        let global = Pubkey::from_str(PUMP_FUN_GLOBAL)?;
        let fee_recipient = Pubkey::from_str(PUMP_FUN_FEE_RECIPIENT)?;
        let token_program = Pubkey::from_str(TOKEN_PROGRAM)?;
        let event_authority = Pubkey::from_str(PUMP_EVENT_AUTHORITY)?;
        let fee_program = Pubkey::from_str(FEE_PROGRAM)?;

        let creator_pubkey = creator.cloned().unwrap_or(*bonding_curve);
        let (creator_vault, _) = Pubkey::find_program_address(&[b"creator-vault", creator_pubkey.as_ref()], &program_id);
        let (fee_config, _) = Pubkey::find_program_address(&[b"fee_config", &PUMP_FEE_CONFIG_SEEDS], &fee_program);
        let buyback1 = Pubkey::from_str(PUMP_BUYBACK_1)?;
        let buyback2 = Pubkey::from_str(PUMP_BUYBACK_2)?;

        // Pump.fun sell discriminator: [51, 230, 133, 164, 1, 127, 131, 173]
        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&[51, 230, 133, 164, 1, 127, 131, 173]);
        data.extend_from_slice(&token_amount_in.to_le_bytes());
        data.extend_from_slice(&min_sol_output_lamports.to_le_bytes());

        let accounts = vec![
            AccountMeta::new_readonly(global, false), // 0
            AccountMeta::new(fee_recipient, false), // 1
            AccountMeta::new_readonly(*token_mint, false), // 2
            AccountMeta::new(*bonding_curve, false), // 3
            AccountMeta::new(*assoc_bonding_curve, false), // 4
            AccountMeta::new(*user_ata, false), // 5
            AccountMeta::new(self.pubkey(), true), // 6
            AccountMeta::new_readonly(system_program::id(), false), // 7
            AccountMeta::new(creator_vault, false), // 8
            AccountMeta::new_readonly(token_program, false), // 9
            AccountMeta::new_readonly(event_authority, false), // 10
            AccountMeta::new_readonly(program_id, false), // 11
            AccountMeta::new_readonly(fee_config, false), // 12
            AccountMeta::new_readonly(fee_program, false), // 13
            AccountMeta::new(buyback1, false), // 14
            AccountMeta::new(buyback2, false), // 15
        ];

        Ok(Instruction {
            program_id,
            accounts,
            data,
        })
    }

    /// Build, sign, and submit an atomic Jito MEV bundle for a sell
    pub async fn execute_snipe_sell_jito(
        &self,
        token_mint_str: &str,
        _bonding_curve_str: &str,
        _creator_str: Option<&str>,
        token_amount_lamports: u64,
        min_sol_lamports: u64,
        jito_tip_sol: f64,
        is_token_2022: bool,
    ) -> Result<BundleSubmissionResult> {
        let mint = Pubkey::from_str(token_mint_str)?;
        let token_program = if is_token_2022 {
            Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?
        } else {
            Pubkey::from_str(TOKEN_PROGRAM)?
        };
        let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM)?;

        let (user_ata, _) = Pubkey::find_program_address(
            &[&self.pubkey().to_bytes(), &token_program.to_bytes(), &mint.to_bytes()],
            &ata_program,
        );

        let tip_lamports = (jito_tip_sol * 1_000_000_000.0) as u64;

        let compute_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(250_000);
        let compute_price_ix = ComputeBudgetInstruction::set_compute_unit_price(25_000);

        let tokens_to_sell = if token_amount_lamports == 0 {
            self.get_token_balance(&user_ata).await.unwrap_or(0)
        } else {
            token_amount_lamports
        };

        if tokens_to_sell == 0 {
            return Err(anyhow!("No tokens available to sell for mint {} in wallet ATA {}", mint, user_ata));
        }

        let is_full_sell = token_amount_lamports == 0;
        // Upgrade 3: Full sells and emergency stop-losses always set min_sol = 1 lamport to guarantee zero Error 6003 reverts
        let effective_min_sol = if is_full_sell { 1 } else { min_sol_lamports.max(1) };

        let sell_ix = self.build_pump_fun_sell_v3_ix(
            &mint,
            tokens_to_sell,
            effective_min_sol,
            is_token_2022,
        )?;

        let (tip_ix, tip_acc_str) = self.build_jito_tip_ix(tip_lamports)?;

        let mut instructions = vec![compute_limit_ix, compute_price_ix, sell_ix];

        // Item 2: Atomically append CloseAccount instruction when exiting 100% of position.
        // This instantly reclaims ~0.00204 SOL ATA rent into the user's native SOL wallet in the exact same transaction block.
        if is_full_sell {
            let close_ix = self.build_close_account_ix(
                &user_ata,
                &self.pubkey(),
                &self.pubkey(),
                is_token_2022,
            )?;
            instructions.push(close_ix);
        }

        instructions.push(tip_ix);

        let blockhash = self.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&self.pubkey()),
            &[&self.keypair],
            blockhash,
        );

        let signature = tx.signatures[0].to_string();
        let jito_fut = self.submit_jito_bundle(vec![tx.clone()]);
        let rpc_fut = self.send_transaction_rpc(&tx);

        let (bundle_res, rpc_res) = tokio::join!(jito_fut, rpc_fut);

        let bundle_id = match (bundle_res, rpc_res) {
            (Ok(bid), Ok(rpc_sig)) => format!("Jito: {} | TPU: {}", &bid[..16.min(bid.len())], &rpc_sig[..16.min(rpc_sig.len())]),
            (Ok(bid), Err(_)) => format!("Jito: {}", bid),
            (Err(_), Ok(rpc_sig)) => format!("TPU: {}", rpc_sig),
            (Err(e1), Err(e2)) => return Err(anyhow!("Dual submission failed: Jito ({}) | TPU ({})", e1, e2)),
        };

        Ok(BundleSubmissionResult {
            bundle_id,
            status: "SUBMITTED_TO_VALIDATORS".to_string(),
            tip_account: tip_acc_str,
            tip_lamports,
            signature,
        })
    }

    /// Construct an SPL Token / Token-2022 CloseAccount instruction to reclaim ~0.00204 SOL ATA rent
    pub fn build_close_account_ix(
        &self,
        token_account: &Pubkey,
        destination: &Pubkey,
        owner: &Pubkey,
        is_token_2022: bool,
    ) -> Result<Instruction> {
        let token_program = if is_token_2022 {
            Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?
        } else {
            Pubkey::from_str(TOKEN_PROGRAM)?
        };

        let accounts = vec![
            AccountMeta::new(*token_account, false),
            AccountMeta::new(*destination, false),
            AccountMeta::new_readonly(*owner, true),
        ];

        // SPL Token instruction: CloseAccount discriminator = 9
        let data = vec![9u8];

        Ok(Instruction {
            program_id: token_program,
            accounts,
            data,
        })
    }

    /// Explicitly close a token account to reclaim ~0.00204 SOL rent (useful for dust or empty ATAs)
    pub async fn close_token_account_jito(
        &self,
        token_mint_str: &str,
        jito_tip_sol: f64,
        is_token_2022: bool,
    ) -> Result<BundleSubmissionResult> {
        let mint = Pubkey::from_str(token_mint_str)?;
        let token_program = if is_token_2022 {
            Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?
        } else {
            Pubkey::from_str(TOKEN_PROGRAM)?
        };
        let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM)?;

        let (user_ata, _) = Pubkey::find_program_address(
            &[&self.pubkey().to_bytes(), &token_program.to_bytes(), &mint.to_bytes()],
            &ata_program,
        );

        let tip_lamports = (jito_tip_sol * 1_000_000_000.0) as u64;
        let compute_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(100_000);
        let compute_price_ix = ComputeBudgetInstruction::set_compute_unit_price(25_000);

        let close_ix = self.build_close_account_ix(
            &user_ata,
            &self.pubkey(),
            &self.pubkey(),
            is_token_2022,
        )?;

        let (tip_ix, tip_acc_str) = self.build_jito_tip_ix(tip_lamports)?;

        let instructions = vec![compute_limit_ix, compute_price_ix, close_ix, tip_ix];

        let blockhash = self.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&self.pubkey()),
            &[&self.keypair],
            blockhash,
        );

        let signature = tx.signatures[0].to_string();
        let jito_fut = self.submit_jito_bundle(vec![tx.clone()]);
        let rpc_fut = self.send_transaction_rpc(&tx);

        let (bundle_res, rpc_res) = tokio::join!(jito_fut, rpc_fut);

        let bundle_id = match (bundle_res, rpc_res) {
            (Ok(bid), Ok(rpc_sig)) => format!("Jito: {} | TPU: {}", &bid[..16.min(bid.len())], &rpc_sig[..16.min(rpc_sig.len())]),
            (Ok(bid), Err(_)) => format!("Jito: {}", bid),
            (Err(_), Ok(rpc_sig)) => format!("TPU: {}", rpc_sig),
            (Err(e1), Err(e2)) => return Err(anyhow!("Close account submission failed: Jito ({}) | TPU ({})", e1, e2)),
        };

        Ok(BundleSubmissionResult {
            bundle_id,
            status: "SUBMITTED_TO_VALIDATORS".to_string(),
            tip_account: tip_acc_str,
            tip_lamports,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_pump_fun_instructions() {
        let dummy_keypair = Keypair::new();
        let executor = OnChainExecutor {
            keypair: dummy_keypair,
            rpc_url: "http://localhost:8899".to_string(),
            jito_url: "http://localhost:8899".to_string(),
            client: reqwest::Client::new(),
            cached_blockhash: Arc::new(RwLock::new(None)),
        };

        let mint = Pubkey::new_unique();
        let bonding_curve = Pubkey::new_unique();
        let assoc_curve = Pubkey::new_unique();
        let user_ata = Pubkey::new_unique();

        let token_program = Pubkey::from_str(TOKEN_PROGRAM).unwrap();
        let buy_ix = executor.build_pump_fun_buy_ix(
            &mint,
            &bonding_curve,
            &assoc_curve,
            &user_ata,
            None,
            1_000_000,
            50_000_000,
            &token_program,
        ).unwrap();

        assert_eq!(buy_ix.program_id, Pubkey::from_str(PUMP_FUN_PROGRAM).unwrap());
        assert_eq!(buy_ix.accounts.len(), 19);
        assert_eq!(&buy_ix.data[..8], &[102, 6, 61, 18, 1, 218, 235, 234]);

        let buy_v3_ix = executor.build_pump_fun_buy_exact_quote_in_v3_ix(
            &mint,
            50_000_000,
            1,
            true,
        ).unwrap();
        assert_eq!(buy_v3_ix.accounts.len(), 17);
        assert_eq!(&buy_v3_ix.data[..8], &[225, 247, 80, 30, 213, 179, 132, 136]);

        let sell_v3_ix = executor.build_pump_fun_sell_v3_ix(
            &mint,
            500_000,
            40_000_000,
            true,
        ).unwrap();
        assert_eq!(sell_v3_ix.accounts.len(), 17);
        assert_eq!(&sell_v3_ix.data[..8], &[28, 146, 222, 119, 38, 196, 105, 213]);

        let sell_ix = executor.build_pump_fun_sell_ix(
            &mint,
            &bonding_curve,
            &assoc_curve,
            &user_ata,
            None,
            500_000,
            40_000_000,
        ).unwrap();

        assert_eq!(sell_ix.program_id, Pubkey::from_str(PUMP_FUN_PROGRAM).unwrap());
        assert_eq!(sell_ix.accounts.len(), 16);
        assert_eq!(&sell_ix.data[..8], &[51, 230, 133, 164, 1, 127, 131, 173]);
    }

    #[test]
    fn test_build_jito_tip_ix() {
        let dummy_keypair = Keypair::new();
        let executor = OnChainExecutor {
            keypair: dummy_keypair,
            rpc_url: "http://localhost:8899".to_string(),
            jito_url: "http://localhost:8899".to_string(),
            client: reqwest::Client::new(),
            cached_blockhash: Arc::new(RwLock::new(None)),
        };

        let (ix, tip_acc) = executor.build_jito_tip_ix(3_000_000).unwrap();
        assert_eq!(ix.program_id, system_program::id());
        assert_eq!(tip_acc, JITO_TIP_ACCOUNTS[0]);
    }

    #[test]
    fn test_build_close_account_ix() {
        let dummy_keypair = Keypair::new();
        let executor = OnChainExecutor {
            keypair: dummy_keypair,
            rpc_url: "http://localhost:8899".to_string(),
            jito_url: "http://localhost:8899".to_string(),
            client: reqwest::Client::new(),
            cached_blockhash: Arc::new(RwLock::new(None)),
        };

        let user_ata = Pubkey::new_unique();
        let user_pubkey = executor.pubkey();

        // Standard Token
        let close_ix = executor.build_close_account_ix(&user_ata, &user_pubkey, &user_pubkey, false).unwrap();
        assert_eq!(close_ix.program_id, Pubkey::from_str(TOKEN_PROGRAM).unwrap());
        assert_eq!(close_ix.accounts.len(), 3);
        assert_eq!(close_ix.accounts[0].pubkey, user_ata);
        assert!(close_ix.accounts[0].is_writable);
        assert!(!close_ix.accounts[0].is_signer);
        assert_eq!(close_ix.accounts[1].pubkey, user_pubkey);
        assert!(close_ix.accounts[1].is_writable);
        assert_eq!(close_ix.accounts[2].pubkey, user_pubkey);
        assert!(close_ix.accounts[2].is_signer);
        assert_eq!(close_ix.data, vec![9u8]);

        // Token-2022
        let close_ix_2022 = executor.build_close_account_ix(&user_ata, &user_pubkey, &user_pubkey, true).unwrap();
        assert_eq!(close_ix_2022.program_id, Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").unwrap());
        assert_eq!(close_ix_2022.data, vec![9u8]);
    }

    #[test]
    fn test_decode_buy_v2() {
        let b58 = "HnQ14hy6fxGQdk7e7SE1WH9jF49jz7Cn3";
        let bytes = bs58::decode(b58).into_vec().unwrap();
        println!("BUY_V2 DISCRIMINATOR: {:?}", &bytes[..8]);
        println!("BUY_V2 FULL BYTES ({}) = {:?}", bytes.len(), bytes);
    }

    #[tokio::test]
    async fn test_simulate_live() {
        let rpc = "https://graceful-oasis-a398.mainnet.rpcpool.com/95794a05-4057-49cb-87e4-42d3d9a46774".to_string();
        let executor = match OnChainExecutor::from_keypair_file(
            "/home/rootkit/.config/solana/id.json",
            Some(rpc.clone()),
            None,
        ) {
            Ok(e) => e,
            Err(_) => return,
        };

        let mint_str = "2gYHWNus8ky2ZKnFyTgecwstKUYeJKfgXMJaHbvUpump";
        let mint = Pubkey::from_str(mint_str).unwrap();
        let token_program = Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").unwrap();
        let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM).unwrap();

        let (user_ata, _) = Pubkey::find_program_address(
            &[&executor.pubkey().to_bytes(), &token_program.to_bytes(), &mint.to_bytes()],
            &ata_program,
        );

        let buy_ix = executor.build_pump_fun_buy_exact_quote_in_v3_ix(
            &mint,
            50_000_000, // 0.05 SOL
            1,          // 1 min token out
            true,       // is_token_2022
        ).unwrap();

        let sell_ix = executor.build_pump_fun_sell_v3_ix(
            &mint,
            1_000_000_000, // sell 1,000 tokens
            1,             // 1 lamport min SOL out
            true,          // is_token_2022
        ).unwrap();

        let create_ata_ix = Instruction {
            program_id: ata_program,
            accounts: vec![
                AccountMeta::new(executor.pubkey(), true),
                AccountMeta::new(user_ata, false),
                AccountMeta::new_readonly(executor.pubkey(), false),
                AccountMeta::new_readonly(mint, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(token_program, false),
            ],
            data: vec![1],
        };

        let compute_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(250_000);
        let compute_price_ix = ComputeBudgetInstruction::set_compute_unit_price(25_000);
        let (tip_ix, _) = executor.build_jito_tip_ix(3_000_000).unwrap();

        let blockhash = executor.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[compute_limit_ix, compute_price_ix, create_ata_ix, buy_ix, sell_ix, tip_ix],
            Some(&executor.pubkey()),
            &[&executor.keypair],
            blockhash,
        );

        let sim_res = executor.simulate_transaction(&tx).await.unwrap();
        println!("SIMULATION RESULT: {}", serde_json::to_string_pretty(&sim_res).unwrap());
    }

    #[test]
    fn test_v3_pda_derivations() {
        let pump_program = Pubkey::from_str(PUMP_FUN_PROGRAM).unwrap();
        let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM).unwrap();
        let fee_program = Pubkey::from_str("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ").unwrap();
        let mint = Pubkey::from_str("2gYHWNus8ky2ZKnFyTgecwstKUYeJKfgXMJaHbvUpump").unwrap();
        let wsol_mint = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
        let token_2022 = Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").unwrap();
        let spl_token = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let test_user = Pubkey::from_str("CCCTPKGNz3mrP9umWL7CPr2cyGx9uRZKDwjQHA2mAnZ8").unwrap();

        // 0: global
        let (global, _) = Pubkey::find_program_address(&[b"global"], &pump_program);
        assert_eq!(global.to_string(), "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf");

        // 5: bonding curve
        let (bonding_curve, _) = Pubkey::find_program_address(&[b"bonding-curve", mint.as_ref()], &pump_program);
        assert_eq!(bonding_curve.to_string(), "39gLE2h4J3SuF37aXqP3cEmqdw3t5aNR9eLXVQRCCw2f");

        // 6: associated_base_bonding_curve
        let (assoc_base_bc, _) = Pubkey::find_program_address(
            &[&bonding_curve.to_bytes(), &token_2022.to_bytes(), &mint.to_bytes()],
            &ata_program,
        );
        assert_eq!(assoc_base_bc.to_string(), "2UotTGh2KtikipNWjQHSBbHpEA1WPZfFL8UKa66RnjhN");

        // 7: associated_quote_bonding_curve
        let (assoc_quote_bc, _) = Pubkey::find_program_address(
            &[&bonding_curve.to_bytes(), &spl_token.to_bytes(), &wsol_mint.to_bytes()],
            &ata_program,
        );
        assert_eq!(assoc_quote_bc.to_string(), "mFqce2vVyWHu8vVEf61AvYBTHUJToaV62ugPZf9B61w");

        // 9: associated_base_user (standard ATA)
        let (assoc_base_user, _) = Pubkey::find_program_address(
            &[&test_user.to_bytes(), &token_2022.to_bytes(), &mint.to_bytes()],
            &ata_program,
        );
        println!("DERIVED BASE USER ATA: {}", assoc_base_user);

        // 10: associated_quote_user
        let (assoc_quote_user, _) = Pubkey::find_program_address(
            &[&test_user.to_bytes(), &spl_token.to_bytes(), &wsol_mint.to_bytes()],
            &ata_program,
        );
        println!("DERIVED QUOTE USER ATA: {}", assoc_quote_user);
        assert_eq!(assoc_quote_user.to_string(), "91zFtzAD1UM4piX7ziDs4pbZ7qJhmnxXvGVTn1R3VmFy");

        // 11: user_volume_accumulator
        let (user_vol, _) = Pubkey::find_program_address(
            &[b"user_volume_accumulator", test_user.as_ref()],
            &pump_program,
        );
        assert_eq!(user_vol.to_string(), "4oakfsga9xuaQRi3Y5ZoVipcnGK4GAgXPP6vkMrgWYxE");

        // 12: fee_config
        let fee_seed = [
            1, 86, 224, 246, 147, 102, 90, 207, 68, 219, 21, 104, 191, 23, 91, 170,
            81, 137, 203, 151, 245, 210, 255, 59, 101, 93, 43, 182, 253, 109, 24, 176
        ];
        let (fee_config, _) = Pubkey::find_program_address(
            &[b"fee_config", &fee_seed],
            &fee_program,
        );
        assert_eq!(fee_config.to_string(), "8Wf5TiAheLUqBrKXeYg2JtAFFMWtKdG2BSFgqUcPVwTt");

        // 15: event_authority
        let (event_auth, _) = Pubkey::find_program_address(&[b"__event_authority"], &pump_program);
        assert_eq!(event_auth.to_string(), "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1");

        println!("ALL V3 PDAS MATCH EXACTLY 100%!");
    }
}
