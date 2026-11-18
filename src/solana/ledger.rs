use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use super::sniper::{PortfolioState, SnipedPosition};
use super::breakout::BreakoutPosition;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerSnapshot {
    pub saved_at: DateTime<Utc>,
    pub engine_type: String,
    pub portfolio: PortfolioState,
    pub sniper_positions: HashMap<String, SnipedPosition>,
    pub breakout_positions: HashMap<String, BreakoutPosition>,
}

pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn from_env_or_default() -> Self {
        let path = std::env::var("LEDGER_PATH")
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                format!("{}/.tradingagents/ledger.json", home)
            });
        Self::new(path)
    }

    fn ensure_parent_dir(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    pub fn save_sniper_state(
        &self,
        portfolio: &PortfolioState,
        positions: &HashMap<String, SnipedPosition>,
    ) -> Result<()> {
        self.ensure_parent_dir()?;

        let snapshot = LedgerSnapshot {
            saved_at: Utc::now(),
            engine_type: "sniper".to_string(),
            portfolio: portfolio.clone(),
            sniper_positions: positions.clone(),
            breakout_positions: HashMap::new(),
        };

        let json = serde_json::to_string_pretty(&snapshot)?;

        let tmp_path = self.path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json)?;
        std::fs::rename(&tmp_path, &self.path)?;

        Ok(())
    }

    pub fn save_breakout_state(
        &self,
        portfolio: &PortfolioState,
        positions: &HashMap<String, BreakoutPosition>,
    ) -> Result<()> {
        self.ensure_parent_dir()?;

        let snapshot = LedgerSnapshot {
            saved_at: Utc::now(),
            engine_type: "breakout".to_string(),
            portfolio: portfolio.clone(),
            sniper_positions: HashMap::new(),
            breakout_positions: positions.clone(),
        };

        let json = serde_json::to_string_pretty(&snapshot)?;

        let tmp_path = self.path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json)?;
        std::fs::rename(&tmp_path, &self.path)?;

        Ok(())
    }

    pub fn load(&self) -> Result<Option<LedgerSnapshot>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let contents = std::fs::read_to_string(&self.path)?;
        let snapshot: LedgerSnapshot = serde_json::from_str(&contents)?;

        info!(
            "Loaded ledger from {} (saved at {}, engine: {}, {} sniper + {} breakout positions)",
            self.path.display(),
            snapshot.saved_at,
            snapshot.engine_type,
            snapshot.sniper_positions.len(),
            snapshot.breakout_positions.len(),
        );

        Ok(Some(snapshot))
    }

    pub fn load_sniper_state(&self) -> Result<Option<(PortfolioState, HashMap<String, SnipedPosition>)>> {
        match self.load()? {
            Some(snap) if snap.engine_type == "sniper" => {
                Ok(Some((snap.portfolio, snap.sniper_positions)))
            }
            Some(snap) => {
                warn!(
                    "Ledger contains '{}' state, not sniper. Ignoring.",
                    snap.engine_type
                );
                Ok(None)
            }
            None => Ok(None),
        }
    }

    pub fn load_breakout_state(&self) -> Result<Option<(PortfolioState, HashMap<String, BreakoutPosition>)>> {
        match self.load()? {
            Some(snap) if snap.engine_type == "breakout" => {
                Ok(Some((snap.portfolio, snap.breakout_positions)))
            }
            Some(snap) => {
                warn!(
                    "Ledger contains '{}' state, not breakout. Ignoring.",
                    snap.engine_type
                );
                Ok(None)
            }
            None => Ok(None),
        }
    }

    pub fn save_generic_state<P: Serialize>(
        &self,
        engine_type: &str,
        portfolio: &PortfolioState,
        positions: &P,
    ) -> Result<()> {
        self.ensure_parent_dir()?;

        let path = self.path.with_file_name(format!(
            "{}.json",
            engine_type,
        ));

        #[derive(Serialize)]
        struct GenericSnapshot<'a, P: Serialize> {
            saved_at: DateTime<Utc>,
            engine_type: &'a str,
            portfolio: &'a PortfolioState,
            positions: &'a P,
        }

        let snapshot = GenericSnapshot {
            saved_at: Utc::now(),
            engine_type,
            portfolio,
            positions,
        };

        let json = serde_json::to_string_pretty(&snapshot)?;
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json)?;
        std::fs::rename(&tmp_path, &path)?;

        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn clear(&self) -> Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
            info!("Cleared ledger at {}", self.path.display());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solana::yellowstone::DexProtocol;

    #[test]
    fn test_ledger_roundtrip_sniper() {
        let dir = std::env::temp_dir().join("tradingagents_test_ledger");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_sniper_ledger.json");
        let ledger = Ledger::new(&path);

        let portfolio = PortfolioState {
            initial_budget_sol: 0.5,
            available_sol: 0.4,
            invested_sol: 0.05,
            realized_pnl_sol: 0.01,
            total_trades: 2,
            winning_trades: 1,
            losing_trades: 1,
        };

        let mut positions = HashMap::new();
        positions.insert(
            "test_key".to_string(),
            SnipedPosition {
                token_mint: "TestMint111111111111111111111111111111111111".to_string(),
                pool_address: "TestPool111111111111111111111111111111111111".to_string(),
                creator: None,
                tokens_held: 1_000_000,
                protocol: DexProtocol::PumpFun,
                entry_slot: 100,
                entry_timestamp: Utc::now(),
                entry_price_sol: 0.00001,
                buy_amount_sol: 0.05,
                remaining_portion_pct: 100.0,
                current_multiplier: 1.5,
                peak_multiplier: 1.8,
                tp1_executed: true,
                tp2_executed: false,
                is_closed: false,
                pnl_sol: 0.02,
                age_ticks: 10,
                momentum_type: 0,
                is_token_2022: false,
            },
        );

        ledger.save_sniper_state(&portfolio, &positions).unwrap();

        let loaded = ledger.load_sniper_state().unwrap().unwrap();
        assert_eq!(loaded.0.initial_budget_sol, 0.5);
        assert_eq!(loaded.0.realized_pnl_sol, 0.01);
        assert_eq!(loaded.1.len(), 1);
        assert_eq!(loaded.1["test_key"].token_mint, "TestMint111111111111111111111111111111111111");

        let _ = std::fs::remove_file(&path);
    }
}
