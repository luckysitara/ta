use tokio::sync::mpsc;
use tracing::{info, warn};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum TradeAlert {
    Entry {
        engine: String,
        token_mint: String,
        protocol: String,
        amount_sol: f64,
        entry_price: f64,
        is_paper: bool,
    },
    TakeProfit {
        engine: String,
        token_mint: String,
        tier: String,
        multiplier: f64,
        proceeds_sol: f64,
        profit_sol: f64,
    },
    StopLoss {
        engine: String,
        token_mint: String,
        multiplier: f64,
        loss_sol: f64,
    },
    BreakEvenExit {
        engine: String,
        token_mint: String,
        peak_multiplier: f64,
        pnl_sol: f64,
    },
    RugRejected {
        token_mint: String,
        reason: String,
        latency_us: u64,
    },
    DevDump {
        token_mint: String,
        pool_address: String,
        creator: String,
    },
    EmergencyExit {
        token_mint: String,
        multiplier: f64,
        proceeds_sol: f64,
    },
    SessionSummary {
        engine: String,
        is_paper: bool,
        initial_budget: f64,
        final_value: f64,
        realized_pnl: f64,
        total_trades: usize,
        winning: usize,
        losing: usize,
        win_rate: f64,
    },
}

impl TradeAlert {
    pub fn format_message(&self) -> String {
        match self {
            TradeAlert::Entry { engine, token_mint, protocol, amount_sol, entry_price, is_paper } => {
                let mode = if *is_paper { "PAPER" } else { "LIVE" };
                let mint_short = &token_mint[..token_mint.len().min(12)];
                format!(
                    "🎯 *[{} ENTRY]* \\[{}\\]\n`{}...` on {}\nCost: `{:.4}` SOL | Price: `{:.9}` SOL",
                    mode, engine, mint_short, protocol, amount_sol, entry_price
                )
            }
            TradeAlert::TakeProfit { engine, token_mint, tier, multiplier, proceeds_sol, profit_sol } => {
                let mint_short = &token_mint[..token_mint.len().min(12)];
                format!(
                    "🎉 *[{} HIT]* \\[{}\\]\n`{}...` at `{:.2}x`\nProceeds: `{:.4}` SOL | Profit: `+{:.4}` SOL",
                    tier, engine, mint_short, multiplier, proceeds_sol, profit_sol
                )
            }
            TradeAlert::StopLoss { engine, token_mint, multiplier, loss_sol } => {
                let mint_short = &token_mint[..token_mint.len().min(12)];
                format!(
                    "🛡️ *[STOP LOSS]* \\[{}\\]\n`{}...` at `{:.2}x`\nLoss: `{:.4}` SOL",
                    engine, mint_short, multiplier, loss_sol
                )
            }
            TradeAlert::BreakEvenExit { engine, token_mint, peak_multiplier, pnl_sol } => {
                let mint_short = &token_mint[..token_mint.len().min(12)];
                format!(
                    "🛡️ *[BREAK\\-EVEN EXIT]* \\[{}\\]\n`{}...` peak `{:.2}x`\nPnL: `{:.4}` SOL",
                    engine, mint_short, peak_multiplier, pnl_sol
                )
            }
            TradeAlert::RugRejected { token_mint, reason, latency_us } => {
                let mint_short = &token_mint[..token_mint.len().min(12)];
                format!(
                    "❌ *[RUG REJECTED]*\n`{}...`\nReason: {}\nLatency: `{}µs`",
                    mint_short, reason, latency_us
                )
            }
            TradeAlert::DevDump { token_mint, pool_address: _, creator } => {
                let mint_short = &token_mint[..token_mint.len().min(12)];
                let creator_short = &creator[..creator.len().min(12)];
                format!(
                    "🚨 *[DEV DUMP DETECTED]*\n`{}...`\nCreator: `{}...`",
                    mint_short, creator_short
                )
            }
            TradeAlert::EmergencyExit { token_mint, multiplier, proceeds_sol } => {
                let mint_short = &token_mint[..token_mint.len().min(12)];
                format!(
                    "🚨 *[EMERGENCY EXIT]*\n`{}...` at `{:.2}x`\nSaved: `{:.4}` SOL",
                    mint_short, multiplier, proceeds_sol
                )
            }
            TradeAlert::SessionSummary { engine, is_paper, initial_budget, final_value, realized_pnl, total_trades, winning, losing, win_rate } => {
                let mode = if *is_paper { "PAPER" } else { "LIVE" };
                let pnl_emoji = if *realized_pnl >= 0.0 { "📈" } else { "📉" };
                let roi = if *initial_budget > 0.0 { (realized_pnl / initial_budget) * 100.0 } else { 0.0 };
                format!(
                    "🏁 *[{} SESSION COMPLETE]* \\[{}\\]\n\
                    Initial: `{:.4}` SOL\n\
                    Final: `{:.4}` SOL\n\
                    {} PnL: `{:.4}` SOL \\(`{:.1}%`\\)\n\
                    Trades: `{}` total \\(`{}W`/`{}L`\\)\n\
                    Win Rate: `{:.1}%`",
                    mode, engine, initial_budget, final_value,
                    pnl_emoji, realized_pnl, roi,
                    total_trades, winning, losing, win_rate
                )
            }
        }
    }
}

#[derive(Clone)]
pub struct TelegramNotifier {
    tx: mpsc::UnboundedSender<TradeAlert>,
}

impl TelegramNotifier {
    pub fn from_env() -> Option<Self> {
        let token = std::env::var("TELEGRAM_BOT_TOKEN").ok()?;
        let chat_id = std::env::var("TELEGRAM_CHAT_ID").ok()?;

        if token.is_empty() || chat_id.is_empty() {
            return None;
        }

        let (tx, rx) = mpsc::unbounded_channel();

        let chat_id_log = chat_id.clone();
        tokio::spawn(Self::run_sender(token, chat_id, rx));

        info!("Telegram notifier initialized (chat_id: {})", chat_id_log);
        Some(Self { tx })
    }

    pub fn send(&self, alert: TradeAlert) {
        let _ = self.tx.send(alert);
    }

    async fn run_sender(
        token: String,
        chat_id: String,
        mut rx: mpsc::UnboundedReceiver<TradeAlert>,
    ) {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        let base_url = format!("https://api.telegram.org/bot{}/sendMessage", token);

        while let Some(alert) = rx.recv().await {
            let text = alert.format_message();

            let payload = serde_json::json!({
                "chat_id": chat_id,
                "text": text,
                "parse_mode": "MarkdownV2",
                "disable_web_page_preview": true,
            });

            match client.post(&base_url).json(&payload).send().await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        warn!("Telegram API returned {}: {}", status, body);

                        // Retry without MarkdownV2 on parse failure
                        if body.contains("can't parse") {
                            let fallback = serde_json::json!({
                                "chat_id": chat_id,
                                "text": text.replace('*', "").replace('`', "").replace("\\[", "[").replace("\\]", "]").replace("\\-", "-"),
                                "disable_web_page_preview": true,
                            });
                            let _ = client.post(&base_url).json(&fallback).send().await;
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to send Telegram notification: {}", e);
                }
            }

            // Rate limit: Telegram allows ~30 messages/second per bot
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

// No-op notifier for when Telegram is not configured
impl Default for TelegramNotifier {
    fn default() -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();
        Self { tx }
    }
}
