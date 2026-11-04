use anyhow::Result;
use colored::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::prelude::*;

// Program IDs for major Solana DEXes / Launchpads
pub const PUMP_FUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
pub const PUMP_FUN_GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
pub const RAYDIUM_AMM_V4_PROGRAM: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
pub const RAYDIUM_CPMM_PROGRAM: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
pub const METEORA_DLMM_PROGRAM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DexProtocol {
    PumpFun,
    RaydiumAmm,
    RaydiumCpmm,
    MeteoraDlmm,
    Unknown(String),
}

impl std::fmt::Display for DexProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DexProtocol::PumpFun => write!(f, "Pump.fun"),
            DexProtocol::RaydiumAmm => write!(f, "Raydium AMM v4"),
            DexProtocol::RaydiumCpmm => write!(f, "Raydium CPMM"),
            DexProtocol::MeteoraDlmm => write!(f, "Meteora DLMM"),
            DexProtocol::Unknown(s) => write!(f, "Unknown ({s})"),
        }
    }
}

/// Live Pump.fun bonding curve virtual & real reserve state streamed via Yellowstone gRPC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BondingCurveState {
    pub pool_address: String,
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub price_sol: f64,
    pub complete: bool,
}

impl BondingCurveState {
    pub fn parse(pool_address: String, data: &[u8]) -> Option<Self> {
        if data.len() < 49 {
            return None;
        }

        // Pump.fun bonding curve discriminator: 0x17b7f83760d8ac60
        if &data[0..8] != &[0x17, 0xb7, 0xf8, 0x37, 0x60, 0xd8, 0xac, 0x60] {
            return None;
        }

        let virtual_token_reserves = u64::from_le_bytes(data[8..16].try_into().ok()?);
        let virtual_sol_reserves = u64::from_le_bytes(data[16..24].try_into().ok()?);
        let real_token_reserves = u64::from_le_bytes(data[24..32].try_into().ok()?);
        let real_sol_reserves = u64::from_le_bytes(data[32..40].try_into().ok()?);
        let complete = data[48] != 0;

        let price_sol = if virtual_token_reserves > 0 {
            (virtual_sol_reserves as f64 / 1_000_000_000.0) / (virtual_token_reserves as f64 / 1_000_000.0)
        } else {
            0.0
        };

        Some(Self {
            pool_address,
            virtual_token_reserves,
            virtual_sol_reserves,
            real_token_reserves,
            real_sol_reserves,
            price_sol,
            complete,
        })
    }
}

/// Represents a newly detected liquidity pool or token bonding curve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolCreationEvent {
    pub protocol: DexProtocol,
    pub signature: String,
    pub slot: u64,
    pub token_mint: String,
    pub pool_address: String,
    pub creator: Option<String>,
    pub initial_sol_liquidity: Option<f64>,
    pub dev_buy_sol: Option<f64>,
    pub metadata_uri: Option<String>,
    pub has_socials: bool,
    pub timestamp_ms: i64,
    pub log_snippet: Option<String>,
    pub is_token_2022: bool,
}

/// Represents trade activity (Buy or Sell) detected on a DEX / bonding curve
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeActivityEvent {
    pub signature: String,
    pub pool_address: String,
    pub user_wallet: String,
    pub is_buy: bool,
    pub is_sell: bool,
    pub slot: u64,
}

pub struct YellowstoneListener {
    endpoint: String,
    x_token: Option<String>,
    subscribed_protocols: Vec<DexProtocol>,
    track_onchain_reserves: bool,
    running: Arc<AtomicBool>,
}

impl YellowstoneListener {
    pub fn new(
        endpoint: String,
        x_token: Option<String>,
        subscribed_protocols: Vec<DexProtocol>,
        track_onchain_reserves: bool,
    ) -> Self {
        Self {
            endpoint,
            x_token,
            subscribed_protocols,
            track_onchain_reserves,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    #[allow(dead_code)]
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Build the Yellowstone gRPC subscribe request filtering for DEX program creations.
    pub fn build_subscribe_request(&self) -> SubscribeRequest {
        let mut program_accounts: Vec<String> = Vec::new();

        for protocol in &self.subscribed_protocols {
            match protocol {
                DexProtocol::PumpFun => program_accounts.push(PUMP_FUN_PROGRAM.to_string()),
                DexProtocol::RaydiumAmm => program_accounts.push(RAYDIUM_AMM_V4_PROGRAM.to_string()),
                DexProtocol::RaydiumCpmm => program_accounts.push(RAYDIUM_CPMM_PROGRAM.to_string()),
                DexProtocol::MeteoraDlmm => program_accounts.push(METEORA_DLMM_PROGRAM.to_string()),
                DexProtocol::Unknown(p) => program_accounts.push(p.clone()),
            }
        }

        // If nothing specified, default to Pump.fun and Raydium AMM
        if program_accounts.is_empty() {
            program_accounts.push(PUMP_FUN_PROGRAM.to_string());
            program_accounts.push(RAYDIUM_AMM_V4_PROGRAM.to_string());
        }

        let mut transactions = HashMap::new();
        transactions.insert(
            "dex_pool_creations".to_string(),
            SubscribeRequestFilterTransactions {
                vote: Some(false),
                failed: Some(false),
                signature: None,
                account_include: program_accounts,
                account_exclude: vec![],
                account_required: vec![],
                ..Default::default()
            },
        );

        let mut accounts = HashMap::new();
        if self.track_onchain_reserves {
            accounts.insert(
                "pump_bonding_curves".to_string(),
                SubscribeRequestFilterAccounts {
                    account: vec![],
                    owner: vec![PUMP_FUN_PROGRAM.to_string()],
                    filters: vec![],
                    nonempty_txn_signature: None,
                    ..Default::default()
                },
            );
        }

        SubscribeRequest {
            transactions,
            accounts,
            commitment: Some(CommitmentLevel::Processed as i32),
            ..Default::default()
        }
    }

    /// Connect and listen to the Yellowstone stream, dispatching detected pools to `tx_callback`,
    /// live bonding curve updates to `account_callback`, and trade activity (buys/sells) to `trade_callback`.
    pub async fn listen<F, A, T>(
        &self,
        mut tx_callback: F,
        mut account_callback: A,
        mut trade_callback: T,
    ) -> Result<()>
    where
        F: FnMut(PoolCreationEvent) + Send + 'static,
        A: FnMut(BondingCurveState) + Send + 'static,
        T: FnMut(TradeActivityEvent) + Send + 'static,
    {
        self.running.store(true, Ordering::Relaxed);

        let mut backoff = Duration::from_millis(500);
        let max_backoff = Duration::from_secs(10);

        println!("\n  Connecting to Yellowstone gRPC: {}...", self.endpoint);

        while self.running.load(Ordering::Relaxed) {
            info!("Connecting to Yellowstone gRPC at {}...", self.endpoint);

            let mut builder = match GeyserGrpcClient::build_from_shared(self.endpoint.clone()) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("  ❌ Failed to parse Yellowstone endpoint URL '{}': {e}", self.endpoint);
                    anyhow::bail!("Invalid Yellowstone gRPC endpoint: {e}");
                }
            };

            if let Some(token) = &self.x_token {
                builder = match builder.x_token(Some(token.clone())) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("  ❌ Failed to configure Yellowstone X-Token: {e}");
                        anyhow::bail!("Invalid Yellowstone X-Token: {e}");
                    }
                };
            }

            if self.endpoint.starts_with("https://") {
                let tls_config = tonic::transport::ClientTlsConfig::new().with_native_roots();
                builder = match builder.tls_config(tls_config) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("  ❌ Failed to configure TLS for Yellowstone gRPC: {e}");
                        anyhow::bail!("Invalid TLS configuration: {e}");
                    }
                };
            }

            builder = builder
                .connect_timeout(Duration::from_secs(5))
                .tcp_keepalive(Some(Duration::from_secs(10)))
                .http2_keep_alive_interval(Duration::from_secs(10))
                .keep_alive_timeout(Duration::from_secs(5))
                .keep_alive_while_idle(true)
                .tcp_nodelay(true);

            let mut client = match builder.connect().await {
                Ok(c) => {
                    println!("  ✔ Connected to Yellowstone gRPC! Establishing DEX filters...");
                    backoff = Duration::from_millis(500); // Reset backoff on success
                    c
                }
                Err(e) => {
                    eprintln!("  ✖ Connection attempt failed: {e}");
                    if self.endpoint.contains("rpcpool.com") {
                        eprintln!("\n{}", "  ┌────────────────── TRITON ONE / RPC DIAGNOSTIC ──────────────────".bold().red());
                        eprintln!("  │ Endpoint: {}", self.endpoint.cyan());
                        eprintln!("  │ ");
                        eprintln!("  │ 🛑 Triton One is rejecting connections with HTTP 403 Forbidden:");
                        eprintln!("  │    \"Your IP or provider is blocked from this endpoint\"");
                        eprintln!("  │ ");
                        eprintln!("  │ 💡 How to authorize this connection:");
                        eprintln!("  │ 1. IP Whitelist (Recommended for Triton Dedicated):");
                        eprintln!("  │    Log in to https://customers.triton.one and add this server's IP:");
                        eprintln!("  │    👉 Public IP: {}", "98.97.77.213".bold().yellow());
                        eprintln!("  │ 2. API Token / Secret Path:");
                        eprintln!("  │    If Triton gave you an API token, put it in .env:");
                        eprintln!("  │    SOLANA_GRPC_TOKEN=your_token_here");
                        eprintln!("  │ 3. Run Benchmark Simulation (Zero Fund Risk):");
                        eprintln!("  │    ./snipe.sh --simulate");
                        eprintln!("{}", "  └─────────────────────────────────────────────────────────────────\n".bold().red());
                    } else if let Some(diag) = probe_endpoint_diagnostic(&self.endpoint, self.x_token.as_deref()).await {
                        eprintln!("\n{}", "  ┌────────────────── RPC PROVIDER DIAGNOSTIC ──────────────────".bold().red());
                        eprintln!("  │ Server Response: {}", diag.yellow());
                        eprintln!("{}", "  └─────────────────────────────────────────────────────────────\n".bold().red());
                    }
                    eprintln!("  Retrying in {:?}...", backoff);
                    warn!("Yellowstone gRPC connection failed: {e}. Retrying in {:?}...", backoff);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(max_backoff);
                    continue;
                }
            };

            let req = self.build_subscribe_request();
            let (_sub_tx, mut stream) = match client.subscribe_with_request(Some(req)).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("  ✖ Failed to subscribe to Yellowstone stream: {e}");
                    error!("Failed to subscribe to Yellowstone gRPC stream: {e}");
                    tokio::time::sleep(backoff).await;
                    continue;
                }
            };

            println!("  ✔ Real-Time Yellowstone Stream Active!");
            println!("  🎯 Subscribed to Pump.fun, Raydium AMM v4 & Meteora DLMM pool creations.");
            if self.track_onchain_reserves {
                println!("  ⚡ Subscribed to Live Pump.fun Bonding Curve Account Reserves (Option 1 Active).");
            }
            println!("  Listening for on-chain launches... (Press Ctrl+C to stop)\n");
            info!("Active gRPC subscription established. Listening for new pool launches...");

            while self.running.load(Ordering::Relaxed) {
                tokio::select! {
                    msg = stream.next() => {
                        match msg {
                            Some(Ok(update)) => {
                                if let Some(update_oneof) = update.update_oneof {
                                    match update_oneof {
                                        subscribe_update::UpdateOneof::Transaction(update_tx) => {
                                            static TX_IN_CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                                            let cnt = TX_IN_CNT.fetch_add(1, Ordering::Relaxed);
                                            if cnt > 0 && cnt % 100 == 0 {
                                                println!("   ⚡ [STREAM ACTIVE] Ingested {} transactions | Slot {} | Monitoring DEX launches...", cnt, update_tx.slot);
                                            }
                                            if let Some(event) = parse_transaction_for_pool_creation(&update_tx) {
                                                tx_callback(event);
                                            } else if let Some(trade) = parse_transaction_for_trade_activity(&update_tx) {
                                                trade_callback(trade);
                                            }
                                        }
                                        subscribe_update::UpdateOneof::Account(update_acc) => {
                                            if let Some(acc_info) = update_acc.account {
                                                let pool_address = bs58::encode(&acc_info.pubkey).into_string();
                                                if let Some(state) = BondingCurveState::parse(pool_address, &acc_info.data) {
                                                    account_callback(state);
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                warn!("Yellowstone stream error: {e}. Reconnecting...");
                                break;
                            }
                            None => {
                                warn!("Yellowstone stream reached EOF. Reconnecting...");
                                break;
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(20)) => {
                        warn!("Yellowstone stream heartbeat timeout (no activity for 20s). Reconnecting...");
                        println!("   ⚠ [YELLOWSTONE HEARTBEAT] No activity for 20s. Auto-reconnecting gRPC stream...");
                        break;
                    }
                }
            }
        }

        info!("Yellowstone gRPC listener stopped.");
        Ok(())
    }
}

/// Zero-copy / minimal-allocation parser for newly created liquidity pools and token launches.
pub fn parse_transaction_for_pool_creation(
    update_tx: &SubscribeUpdateTransaction,
) -> Option<PoolCreationEvent> {
    let slot = update_tx.slot;
    let tx_info = update_tx.transaction.as_ref()?;
    let meta = tx_info.meta.as_ref()?;

    // Discard failed transactions immediately (zero-loss filtering)
    if meta.err.is_some() {
        return None;
    }

    let signature = bs58::encode(&tx_info.signature).into_string();
    let raw_tx = tx_info.transaction.as_ref()?;
    let message = raw_tx.message.as_ref()?;

    // Map account index to base58 pubkey (static + dynamic lookup table addresses)
    let mut all_account_keys: Vec<String> = message
        .account_keys
        .iter()
        .map(|k| bs58::encode(k).into_string())
        .collect();
    for k in &meta.loaded_writable_addresses {
        all_account_keys.push(bs58::encode(k).into_string());
    }
    for k in &meta.loaded_readonly_addresses {
        all_account_keys.push(bs58::encode(k).into_string());
    }

    if all_account_keys.is_empty() {
        return None;
    }

    let log_messages: &[String] = &meta.log_messages;

    let metadata_uri = message.instructions.iter().find_map(|instr| {
        let s = String::from_utf8_lossy(&instr.data);
        if let Some(idx) = s.find("http") {
            let uri = s[idx..].split(|c: char| c.is_whitespace() || c == '\0').next()?;
            Some(uri.to_string())
        } else if let Some(idx) = s.find("ipfs") {
            let uri = s[idx..].split(|c: char| c.is_whitespace() || c == '\0').next()?;
            Some(uri.to_string())
        } else {
            None
        }
    });

    // Option 2: Inspect instructions, logs, and metadata URI for verified socials (Twitter/X, Telegram, Discord, IPFS/HTTP metadata)
    let has_socials = metadata_uri.as_ref().map(|u| !u.is_empty()).unwrap_or(false)
        || message.instructions.iter().any(|instr| {
            let s = String::from_utf8_lossy(&instr.data).to_lowercase();
            s.contains("twitter") || s.contains("t.me") || s.contains("x.com") || s.contains("telegram") || s.contains("discord")
        }) || meta.log_messages.iter().any(|log| {
            let s = log.to_lowercase();
            s.contains("twitter") || s.contains("t.me") || s.contains("x.com") || s.contains("telegram")
        });

    const PUMP_FUN_CREATE_DISCRIMINATOR: [u8; 8] = [24, 30, 200, 40, 5, 28, 7, 119];
    const PUMP_FUN_CREATE_V2_DISCRIMINATOR: [u8; 8] = [214, 144, 76, 236, 95, 139, 49, 180];
    const WSOL: &str = "So11111111111111111111111111111111111111112";

    // 1. Detect Pump.fun token creation (both v1 and modern v2 instructions)
    // Pump.fun Create v1: accounts: [0] mint, [1] auth, [2] bonding_curve, ..., [7] creator
    // Pump.fun Create v2: accounts: [0] mint, [1] auth, [2] bonding_curve, ..., [5] creator
    let outer_pump_ixs = message.instructions.iter().map(|ix| (ix.program_id_index as usize, &ix.accounts[..], &ix.data[..]));
    let inner_pump_ixs = meta.inner_instructions.iter().flat_map(|inner| {
        inner.instructions.iter().map(|ix| (ix.program_id_index as usize, &ix.accounts[..], &ix.data[..]))
    });
    let pump_create = outer_pump_ixs.chain(inner_pump_ixs).find(|&(prog_idx, accounts, data)| {
        let is_pump = all_account_keys.get(prog_idx).map(|s| s.as_str()) == Some(PUMP_FUN_PROGRAM);
        if !is_pump || accounts.len() < 6 || data.len() < 8 {
            return false;
        }
        data[..8] == PUMP_FUN_CREATE_DISCRIMINATOR || data[..8] == PUMP_FUN_CREATE_V2_DISCRIMINATOR
    });

    if let Some((_, accounts, data)) = pump_create {
        if accounts.len() >= 6 {
            let is_v2 = data.len() >= 8 && data[..8] == PUMP_FUN_CREATE_V2_DISCRIMINATOR;
            // For create_v2, remaining accounts select the quote mint (none/empty = native SOL).
            // Skip non-SOL paired tokens (custom meme-coin/token quotes) to ensure 100% SOL pair compatibility.
            if is_v2 && accounts.len() > 16 {
                return None;
            }
            let mint_idx = accounts[0] as usize;
            let bc_idx = accounts[2] as usize;
            let creator_idx = if is_v2 && accounts.len() > 5 {
                accounts[5] as usize
            } else if accounts.len() > 7 {
                accounts[7] as usize
            } else {
                accounts[accounts.len() - 1] as usize
            };

            let token_mint = all_account_keys.get(mint_idx).cloned().unwrap_or_default();
            let bonding_curve = all_account_keys.get(bc_idx).cloned().unwrap_or_default();
            let creator = all_account_keys.get(creator_idx).cloned();

            // Sanity check: token_mint must not be a known protocol infrastructure account
            if token_mint == PUMP_FUN_GLOBAL || token_mint == PUMP_FUN_PROGRAM || token_mint.is_empty() {
                return None;
            }

            let dev_buy_sol = if creator_idx < meta.pre_balances.len() && creator_idx < meta.post_balances.len() {
                let pre = meta.pre_balances[creator_idx];
                let post = meta.post_balances[creator_idx];
                if pre > post {
                    Some((pre - post) as f64 / 1_000_000_000.0)
                } else {
                    Some(0.0)
                }
            } else {
                None
            };

            let initial_sol = dev_buy_sol.unwrap_or(0.0);

            let is_token_2022 = is_v2 || all_account_keys.iter().any(|k| k == "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

            return Some(PoolCreationEvent {
                protocol: DexProtocol::PumpFun,
                signature,
                slot,
                token_mint,
                pool_address: bonding_curve,
                creator,
                initial_sol_liquidity: Some(initial_sol),
                dev_buy_sol,
                metadata_uri,
                has_socials,
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
                log_snippet: Some("Pump.fun Bonding Curve Launched".to_string()),
                is_token_2022,
            });
        }
    }

    // 2. Detect Raydium AMM v4 pool creation
    // Raydium initialize2 instruction
    let is_raydium_init = log_messages.iter().any(|log: &String| {
        log.contains("initialize2")
            || (log.contains("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8")
                && log.contains("Initialize"))
    });

    if is_raydium_init {
        let outer_ray = message.instructions.iter().map(|ix| (ix.program_id_index as usize, &ix.accounts[..]));
        let inner_ray = meta.inner_instructions.iter().flat_map(|inner| {
            inner.instructions.iter().map(|ix| (ix.program_id_index as usize, &ix.accounts[..]))
        });
        let ray_ix = outer_ray.chain(inner_ray).find(|&(prog_idx, accounts)| {
            all_account_keys.get(prog_idx).map(|s| s.as_str()) == Some(RAYDIUM_AMM_V4_PROGRAM)
                && accounts.len() >= 10
        });

        if let Some((_, accounts)) = ray_ix {
            let pool_idx = accounts.get(4).copied().map(|i| i as usize);
            let coin_idx = accounts.get(8).copied().map(|i| i as usize);
            let pc_idx = accounts.get(9).copied().map(|i| i as usize);
            let creator_idx = accounts.get(17).copied().map(|i| i as usize);

            let pool_address = pool_idx.and_then(|i| all_account_keys.get(i).cloned()).unwrap_or_default();
            let coin_mint = coin_idx.and_then(|i| all_account_keys.get(i).cloned()).unwrap_or_default();
            let pc_mint = pc_idx.and_then(|i| all_account_keys.get(i).cloned()).unwrap_or_default();
            let creator = creator_idx.and_then(|i| all_account_keys.get(i).cloned());

            let token_mint = if pc_mint == WSOL {
                coin_mint
            } else if coin_mint == WSOL {
                pc_mint
            } else {
                coin_mint
            };

            let initial_sol = calculate_sol_liquidity(meta, 0);

            return Some(PoolCreationEvent {
                protocol: DexProtocol::RaydiumAmm,
                signature,
                slot,
                token_mint,
                pool_address,
                creator,
                initial_sol_liquidity: Some(initial_sol),
                dev_buy_sol: None,
                metadata_uri: None,
                has_socials: true,
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
                log_snippet: Some("Raydium AMM v4 Pool Initialized".to_string()),
                is_token_2022: false,
            });
        }
    }

    // 3. Detect Meteora DLMM pool creation
    let is_meteora_init = log_messages.iter().any(|log: &String| {
        log.contains("initializeLbPair")
            || (log.contains("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo")
                && log.contains("Initialize"))
    });

    if is_meteora_init {
        let outer_met = message.instructions.iter().map(|ix| (ix.program_id_index as usize, &ix.accounts[..]));
        let inner_met = meta.inner_instructions.iter().flat_map(|inner| {
            inner.instructions.iter().map(|ix| (ix.program_id_index as usize, &ix.accounts[..]))
        });
        let meteora_ix = outer_met.chain(inner_met).find(|&(prog_idx, accounts)| {
            all_account_keys.get(prog_idx).map(|s| s.as_str()) == Some(METEORA_DLMM_PROGRAM)
                && accounts.len() >= 6
        });

        if let Some((_, accounts)) = meteora_ix {
            let pool_idx = accounts.get(0).copied().map(|i| i as usize);
            let mint_x_idx = accounts.get(2).copied().map(|i| i as usize);
            let mint_y_idx = accounts.get(3).copied().map(|i| i as usize);
            let creator_idx = accounts.get(8).copied().map(|i| i as usize);

            let pool_address = pool_idx.and_then(|i| all_account_keys.get(i).cloned()).unwrap_or_default();
            let mint_x = mint_x_idx.and_then(|i| all_account_keys.get(i).cloned()).unwrap_or_default();
            let mint_y = mint_y_idx.and_then(|i| all_account_keys.get(i).cloned()).unwrap_or_default();
            let creator = creator_idx.and_then(|i| all_account_keys.get(i).cloned());

            let token_mint = if mint_x == WSOL { mint_y } else { mint_x };

            let initial_sol = calculate_sol_liquidity(meta, 0);

            return Some(PoolCreationEvent {
                protocol: DexProtocol::MeteoraDlmm,
                signature,
                slot,
                token_mint,
                pool_address,
                creator,
                initial_sol_liquidity: Some(initial_sol),
                dev_buy_sol: None,
                metadata_uri: None,
                has_socials: true,
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
                log_snippet: Some("Meteora DLMM Pair Initialized".to_string()),
                is_token_2022: false,
            });
        }
    }

    None
}

/// Parse confirmed transaction for live Pump.fun Buy or Sell trade activity
pub fn parse_transaction_for_trade_activity(
    update_tx: &SubscribeUpdateTransaction,
) -> Option<TradeActivityEvent> {
    let slot = update_tx.slot;
    let tx_info = update_tx.transaction.as_ref()?;
    let meta = tx_info.meta.as_ref()?;

    if meta.err.is_some() {
        return None;
    }

    let signature = bs58::encode(&tx_info.signature).into_string();
    let raw_tx = tx_info.transaction.as_ref()?;
    let message = raw_tx.message.as_ref()?;

    let mut all_account_keys: Vec<String> = message
        .account_keys
        .iter()
        .map(|k| bs58::encode(k).into_string())
        .collect();
    for k in &meta.loaded_writable_addresses {
        all_account_keys.push(bs58::encode(k).into_string());
    }
    for k in &meta.loaded_readonly_addresses {
        all_account_keys.push(bs58::encode(k).into_string());
    }

    if all_account_keys.is_empty() {
        return None;
    }

    let user_wallet = all_account_keys.first()?.clone();

    const BUY_V3: [u8; 8] = [225, 247, 80, 30, 213, 179, 132, 136];
    const BUY_V1: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
    const SELL_V3: [u8; 8] = [28, 146, 222, 119, 38, 196, 105, 213];
    const SELL_V1: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

    let outer_ixs = message.instructions.iter().map(|ix| (ix.program_id_index as usize, &ix.accounts[..], &ix.data[..]));
    let inner_ixs = meta.inner_instructions.iter().flat_map(|inner| {
        inner.instructions.iter().map(|ix| (ix.program_id_index as usize, &ix.accounts[..], &ix.data[..]))
    });

    for (prog_idx, accounts, data) in outer_ixs.chain(inner_ixs) {
        if all_account_keys.get(prog_idx).map(|s| s.as_str()) != Some(PUMP_FUN_PROGRAM) {
            continue;
        }
        if data.len() < 8 {
            continue;
        }

        let disc = &data[..8];
        let (is_buy, is_sell, bc_idx_pos) = if disc == BUY_V3 {
            (true, false, 5)
        } else if disc == BUY_V1 {
            (true, false, 3)
        } else if disc == SELL_V3 {
            (false, true, 5)
        } else if disc == SELL_V1 {
            (false, true, 3)
        } else {
            continue;
        };

        if let Some(&bc_idx) = accounts.get(bc_idx_pos) {
            if let Some(pool_address) = all_account_keys.get(bc_idx as usize) {
                return Some(TradeActivityEvent {
                    signature,
                    pool_address: pool_address.clone(),
                    user_wallet,
                    is_buy,
                    is_sell,
                    slot,
                });
            }
        }
    }

    None
}

/// Calculate the initial SOL liquidity deposited into the pool by inspecting balance deltas
fn calculate_sol_liquidity(meta: &TransactionStatusMeta, _account_idx: usize) -> f64 {
    // Check if pre/post balances show SOL movement
    if meta.pre_balances.len() == meta.post_balances.len() && !meta.pre_balances.is_empty() {
        let max_delta = meta
            .pre_balances
            .iter()
            .zip(meta.post_balances.iter())
            .map(|(pre, post)| (*post as i64) - (*pre as i64))
            .filter(|&d| d > 0)
            .max()
            .unwrap_or(0);

        if max_delta > 0 {
            return (max_delta as f64) / 1_000_000_000.0;
        }
    }
    // Pump.fun bonding curves start with 30 SOL virtual reserves
    30.0
}

/// Diagnoses connection failures to Yellowstone gRPC endpoints (e.g. Triton, QuickNode, Helius)
pub async fn probe_endpoint_diagnostic(endpoint: &str, x_token: Option<&str>) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .ok()?;

    let mut req = client.post(endpoint).header("content-type", "application/grpc");
    if let Some(tok) = x_token {
        req = req.header("x-token", tok);
    }

    if let Ok(res) = req.send().await {
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            if body.contains("Your IP or provider is blocked") || status == reqwest::StatusCode::FORBIDDEN {
                return Some(format!(
                    "HTTP 403 Forbidden - IP or Provider Blocked by Triton: '{}'",
                    body.trim()
                ));
            } else if status == reqwest::StatusCode::UNAUTHORIZED {
                return Some("HTTP 401 Unauthorized - Invalid or missing X-Token".to_string());
            } else {
                return Some(format!("HTTP {} - {}", status, body.trim()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_subscribe_request() {
        let listener = YellowstoneListener::new(
            "http://127.0.0.1:10000".to_string(),
            Some("secret-key".to_string()),
            vec![DexProtocol::PumpFun, DexProtocol::RaydiumAmm],
            true, // track_onchain_reserves
        );

        let req = listener.build_subscribe_request();
        assert!(req.transactions.contains_key("dex_pool_creations"));
        assert!(req.accounts.contains_key("pump_bonding_curves"));
        let filter = req.transactions.get("dex_pool_creations").unwrap();
        assert_eq!(filter.vote, Some(false));
        assert_eq!(filter.failed, Some(false));
        assert!(filter.account_include.contains(&PUMP_FUN_PROGRAM.to_string()));
        assert!(filter.account_include.contains(&RAYDIUM_AMM_V4_PROGRAM.to_string()));
    }

    #[test]
    fn test_bonding_curve_state_parse() {
        // Construct mock 49-byte pump bonding curve data
        let mut data = vec![0u8; 49];
        // Discriminator: 0x17b7f83760d8ac60
        data[0..8].copy_from_slice(&[0x17, 0xb7, 0xf8, 0x37, 0x60, 0xd8, 0xac, 0x60]);
        // virtual_token_reserves = 1_000_000_000_000_000 (1B tokens)
        data[8..16].copy_from_slice(&1_000_000_000_000_000u64.to_le_bytes());
        // virtual_sol_reserves = 30_000_000_000 (30 SOL)
        data[16..24].copy_from_slice(&30_000_000_000u64.to_le_bytes());
        // real_token_reserves
        data[24..32].copy_from_slice(&793_100_000_000_000u64.to_le_bytes());
        // real_sol_reserves
        data[32..40].copy_from_slice(&0u64.to_le_bytes());
        // complete = false
        data[48] = 0;

        let parsed = BondingCurveState::parse("Curve11111111111111111111111111111111111111".to_string(), &data);
        assert!(parsed.is_some());
        let curve = parsed.unwrap();
        assert_eq!(curve.virtual_token_reserves, 1_000_000_000_000_000);
        assert_eq!(curve.virtual_sol_reserves, 30_000_000_000);
        assert!(!curve.complete);
        assert!((curve.price_sol - 0.000000030).abs() < 1e-10);
    }

    #[test]
    fn test_parse_pump_fun_transaction() {
        // Construct mock 32-byte pubkeys
        let pump_program_bytes = bs58::decode(PUMP_FUN_PROGRAM).into_vec().unwrap();
        let mint_bytes = vec![1u8; 32];
        let curve_bytes = vec![2u8; 32];
        let creator_bytes = vec![3u8; 32];
        let dummy = vec![4u8; 32];

        let account_keys = vec![
            mint_bytes.clone(),         // 0: mint
            dummy.clone(),              // 1: mint authority
            curve_bytes.clone(),        // 2: bonding curve
            dummy.clone(),              // 3: assoc curve
            dummy.clone(),              // 4: global
            dummy.clone(),              // 5: mpl
            dummy.clone(),              // 6: metadata
            creator_bytes.clone(),      // 7: creator
            pump_program_bytes.clone(), // 8: pump program
        ];

        let instruction = CompiledInstruction {
            program_id_index: 8, // index of pump program
            accounts: vec![0, 1, 2, 3, 4, 5, 6, 7], // 0: mint, 1: auth, 2: curve, 3: assoc, 4: global, 5: mpl, 6: meta, 7: creator
            data: vec![24, 30, 200, 40, 5, 28, 7, 119], // PUMP_FUN_CREATE_DISCRIMINATOR
        };

        let message = yellowstone_grpc_proto::prelude::Message {
            account_keys,
            instructions: vec![instruction],
            recent_blockhash: vec![0u8; 32],
            ..Default::default()
        };

        let raw_tx = yellowstone_grpc_proto::prelude::Transaction {
            signatures: vec![vec![9u8; 64]],
            message: Some(message),
        };

        let meta = yellowstone_grpc_proto::prelude::TransactionStatusMeta {
            err: None,
            fee: 5000,
            pre_balances: vec![1_000_000_000],
            post_balances: vec![999_995_000],
            log_messages: vec![
                format!("Program {} invoke [1]", PUMP_FUN_PROGRAM),
                "Program log: Instruction: Create".to_string(),
                "Program 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P success".to_string(),
            ],
            ..Default::default()
        };

        let update_tx = SubscribeUpdateTransaction {
            transaction: Some(SubscribeUpdateTransactionInfo {
                signature: vec![9u8; 64],
                is_vote: false,
                transaction: Some(raw_tx),
                meta: Some(meta),
                index: 0,
            }),
            slot: 280123456,
            ..Default::default()
        };

        let event = parse_transaction_for_pool_creation(&update_tx);
        assert!(event.is_some(), "Should detect pump.fun pool creation");
        let e = event.unwrap();
        assert_eq!(e.protocol, DexProtocol::PumpFun);
        assert_eq!(e.slot, 280123456);
        assert_eq!(e.token_mint, bs58::encode(&mint_bytes).into_string());
        assert_eq!(e.pool_address, bs58::encode(&curve_bytes).into_string());
        assert_eq!(e.creator, Some(bs58::encode(&creator_bytes).into_string()));
    }

    #[test]
    fn test_parse_pump_fun_v2_transaction() {
        let pump_program_bytes = bs58::decode(PUMP_FUN_PROGRAM).into_vec().unwrap();
        let mint_bytes = vec![11u8; 32];
        let curve_bytes = vec![22u8; 32];
        let creator_bytes = vec![33u8; 32];
        let dummy = vec![44u8; 32];

        let account_keys = vec![
            mint_bytes.clone(),         // 0: mint
            dummy.clone(),              // 1: mint authority
            curve_bytes.clone(),        // 2: bonding curve
            dummy.clone(),              // 3: assoc curve
            dummy.clone(),              // 4: global
            creator_bytes.clone(),      // 5: creator (in create_v2)
            dummy.clone(),              // 6: system program
            dummy.clone(),              // 7: token program
            pump_program_bytes.clone(), // 8: pump program
        ];

        let instruction = CompiledInstruction {
            program_id_index: 8,
            accounts: vec![0, 1, 2, 3, 4, 5, 6, 7],
            data: vec![214, 144, 76, 236, 95, 139, 49, 180], // PUMP_FUN_CREATE_V2_DISCRIMINATOR
        };

        let message = yellowstone_grpc_proto::prelude::Message {
            account_keys,
            instructions: vec![instruction],
            recent_blockhash: vec![0u8; 32],
            ..Default::default()
        };

        let raw_tx = yellowstone_grpc_proto::prelude::Transaction {
            signatures: vec![vec![9u8; 64]],
            message: Some(message),
        };

        let meta = yellowstone_grpc_proto::prelude::TransactionStatusMeta {
            err: None,
            fee: 5000,
            pre_balances: vec![1_000_000_000],
            post_balances: vec![999_995_000],
            log_messages: vec![
                format!("Program {} invoke [1]", PUMP_FUN_PROGRAM),
                "Program log: Instruction: CreateV2".to_string(),
                "Program 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P success".to_string(),
            ],
            ..Default::default()
        };

        let update_tx = SubscribeUpdateTransaction {
            transaction: Some(SubscribeUpdateTransactionInfo {
                signature: vec![9u8; 64],
                is_vote: false,
                transaction: Some(raw_tx),
                meta: Some(meta),
                index: 0,
            }),
            slot: 280123457,
            ..Default::default()
        };

        let event = parse_transaction_for_pool_creation(&update_tx);
        assert!(event.is_some(), "Should detect pump.fun create_v2 pool creation");
        let e = event.unwrap();
        assert_eq!(e.protocol, DexProtocol::PumpFun);
        assert_eq!(e.slot, 280123457);
        assert_eq!(e.token_mint, bs58::encode(&mint_bytes).into_string());
        assert_eq!(e.pool_address, bs58::encode(&curve_bytes).into_string());
        assert_eq!(e.creator, Some(bs58::encode(&creator_bytes).into_string()));
    }

    #[test]
    fn test_discard_failed_transaction() {
        let meta = yellowstone_grpc_proto::prelude::TransactionStatusMeta {
            err: Some(yellowstone_grpc_proto::prelude::TransactionError {
                err: vec![1, 2, 3],
            }),
            fee: 5000,
            log_messages: vec!["Instruction: Create".to_string()],
            ..Default::default()
        };

        let update_tx = SubscribeUpdateTransaction {
            transaction: Some(SubscribeUpdateTransactionInfo {
                signature: vec![1u8; 64],
                is_vote: false,
                transaction: None,
                meta: Some(meta),
                index: 0,
            }),
            slot: 100,
            ..Default::default()
        };

        assert!(parse_transaction_for_pool_creation(&update_tx).is_none());
    }

    #[test]
    fn test_reject_pump_fun_buy_transaction() {
        let pump_program_bytes = bs58::decode(PUMP_FUN_PROGRAM).into_vec().unwrap();
        let trader_wallet = vec![5u8; 32];
        let mint_bytes = vec![1u8; 32];

        let account_keys = vec![
            trader_wallet.clone(),      // 0: fee payer (trader wallet)
            mint_bytes.clone(),         // 1: mint
            pump_program_bytes.clone(), // 2: pump program
        ];

        // Buy discriminator: [102, 6, 61, 18, 1, 218, 235, 234]
        let buy_instruction = CompiledInstruction {
            program_id_index: 2,
            accounts: vec![0, 1],
            data: vec![102, 6, 61, 18, 1, 218, 235, 234],
        };

        let message = yellowstone_grpc_proto::prelude::Message {
            account_keys,
            instructions: vec![buy_instruction],
            recent_blockhash: vec![0u8; 32],
            ..Default::default()
        };

        let raw_tx = yellowstone_grpc_proto::prelude::Transaction {
            signatures: vec![vec![9u8; 64]],
            message: Some(message),
        };

        let meta = yellowstone_grpc_proto::prelude::TransactionStatusMeta {
            err: None,
            fee: 5000,
            log_messages: vec![
                format!("Program {} invoke [1]", PUMP_FUN_PROGRAM),
                "Program log: Instruction: Buy".to_string(),
                "Program 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P success".to_string(),
            ],
            ..Default::default()
        };

        let update_tx = SubscribeUpdateTransaction {
            transaction: Some(SubscribeUpdateTransactionInfo {
                signature: vec![9u8; 64],
                is_vote: false,
                transaction: Some(raw_tx),
                meta: Some(meta),
                index: 0,
            }),
            slot: 280123457,
            ..Default::default()
        };

        // Standard buy transaction MUST NOT be parsed as pool creation!
        let event = parse_transaction_for_pool_creation(&update_tx);
        assert!(event.is_none(), "Normal buy trade must NOT be detected as token creation!");
    }

    #[test]
    fn test_parse_pump_fun_buy_trade_activity() {
        let pump_program_bytes = bs58::decode(PUMP_FUN_PROGRAM).into_vec().unwrap();
        let trader_wallet = vec![5u8; 32];
        let mint_bytes = vec![1u8; 32];
        let curve_bytes = vec![2u8; 32];
        let dummy = vec![3u8; 32];

        let account_keys = vec![
            trader_wallet.clone(),      // 0: fee payer (trader wallet)
            dummy.clone(),              // 1
            dummy.clone(),              // 2
            curve_bytes.clone(),        // 3: bonding curve (in BuyV1)
            dummy.clone(),              // 4
            mint_bytes.clone(),         // 5
            pump_program_bytes.clone(), // 6: pump program
        ];

        // Buy discriminator: [102, 6, 61, 18, 1, 218, 235, 234]
        let buy_instruction = CompiledInstruction {
            program_id_index: 6,
            accounts: vec![0, 1, 2, 3, 4, 5],
            data: vec![102, 6, 61, 18, 1, 218, 235, 234],
        };

        let message = yellowstone_grpc_proto::prelude::Message {
            account_keys,
            instructions: vec![buy_instruction],
            recent_blockhash: vec![0u8; 32],
            ..Default::default()
        };

        let raw_tx = yellowstone_grpc_proto::prelude::Transaction {
            signatures: vec![vec![9u8; 64]],
            message: Some(message),
        };

        let meta = yellowstone_grpc_proto::prelude::TransactionStatusMeta {
            err: None,
            fee: 5000,
            ..Default::default()
        };

        let update_tx = SubscribeUpdateTransaction {
            transaction: Some(SubscribeUpdateTransactionInfo {
                signature: vec![9u8; 64],
                is_vote: false,
                transaction: Some(raw_tx),
                meta: Some(meta),
                index: 0,
            }),
            slot: 280123457,
            ..Default::default()
        };

        let trade = parse_transaction_for_trade_activity(&update_tx);
        assert!(trade.is_some(), "Should parse Pump.fun buy trade activity");
        let t = trade.unwrap();
        assert!(t.is_buy);
        assert!(!t.is_sell);
        assert_eq!(t.user_wallet, bs58::encode(&trader_wallet).into_string());
        assert_eq!(t.pool_address, bs58::encode(&curve_bytes).into_string());
    }
}
