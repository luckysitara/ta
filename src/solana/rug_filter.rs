use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RugFilterConfig {
    pub require_freeze_revoked: bool,
    pub require_mint_revoked: bool,
    pub max_creator_supply_pct: f64,
    pub min_liquidity_sol: f64,
    pub reject_honeypots: bool,
    pub require_socials: bool,
    pub max_dev_buy_sol: Option<f64>,
    pub min_dev_buy_sol: Option<f64>,
}

impl Default for RugFilterConfig {
    fn default() -> Self {
        Self {
            require_freeze_revoked: true,
            require_mint_revoked: true,
            max_creator_supply_pct: 25.0, // Reject if creator/insiders hold >25% in block 0
            min_liquidity_sol: 0.5,       // Minimum 0.5 SOL liquidity
            reject_honeypots: true,
            require_socials: false,       // Option 2: Off by default, enabled via --require-socials
            max_dev_buy_sol: None,        // Option 2: Off by default, enabled via --max-dev-buy
            min_dev_buy_sol: None,        // Item 3: Off by default, e.g. 0.40 SOL dev skin-in-the-game
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyStatus {
    Safe,
    Warning(Vec<String>),
    Rejected(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityCheckReport {
    pub token_mint: String,
    pub freeze_authority_revoked: bool,
    pub mint_authority_revoked: bool,
    pub lp_burned_pct: f64,
    pub creator_holding_pct: f64,
    pub initial_sol: f64,
    pub status: SafetyStatus,
    pub execution_latency_us: u64,
}

pub struct RugFilter {
    config: RugFilterConfig,
}

impl RugFilter {
    pub fn new(config: RugFilterConfig) -> Self {
        Self { config }
    }

    /// Run zero-latency heuristic evaluation on newly detected pool parameters (<10us pure memory check).
    pub fn evaluate_fast(
        &self,
        token_mint: &str,
        is_pump_fun: bool,
        initial_sol: f64,
        freeze_auth_revoked: bool,
        mint_auth_revoked: bool,
        creator_holding_pct: f64,
        has_socials: bool,
        dev_buy_sol: Option<f64>,
    ) -> SecurityCheckReport {
        let start = std::time::Instant::now();
        let mut warnings = Vec::new();

        // 1. Freeze Authority Check (Primary Honeypot Vector)
        if self.config.require_freeze_revoked && !freeze_auth_revoked && !is_pump_fun {
            let latency = start.elapsed().as_micros() as u64;
            warn!("[Honeypot Alert] Mint {} has active freeze authority!", token_mint);
            return SecurityCheckReport {
                token_mint: token_mint.to_string(),
                freeze_authority_revoked: false,
                mint_authority_revoked: mint_auth_revoked,
                lp_burned_pct: 0.0,
                creator_holding_pct,
                initial_sol,
                status: SafetyStatus::Rejected("Freeze authority enabled (Honeypot risk)".to_string()),
                execution_latency_us: latency,
            };
        }

        // 2. Mint Authority Check (Unlimited dilution vector)
        if self.config.require_mint_revoked && !mint_auth_revoked && !is_pump_fun {
            let latency = start.elapsed().as_micros() as u64;
            return SecurityCheckReport {
                token_mint: token_mint.to_string(),
                freeze_authority_revoked: freeze_auth_revoked,
                mint_authority_revoked: false,
                lp_burned_pct: 0.0,
                creator_holding_pct,
                initial_sol,
                status: SafetyStatus::Rejected("Mint authority not revoked (Infinite supply risk)".to_string()),
                execution_latency_us: latency,
            };
        }

        // 3. Creator Holding Check (Insider Dump Vector)
        if creator_holding_pct > self.config.max_creator_supply_pct {
            let latency = start.elapsed().as_micros() as u64;
            return SecurityCheckReport {
                token_mint: token_mint.to_string(),
                freeze_authority_revoked: freeze_auth_revoked,
                mint_authority_revoked: mint_auth_revoked,
                lp_burned_pct: 0.0,
                creator_holding_pct,
                initial_sol,
                status: SafetyStatus::Rejected(format!(
                    "Creator holding {:.1}% exceeds max threshold {:.1}%",
                    creator_holding_pct, self.config.max_creator_supply_pct
                )),
                execution_latency_us: latency,
            };
        }

        // 4. Minimum Liquidity Check
        if initial_sol < self.config.min_liquidity_sol && !is_pump_fun {
            warnings.push(format!(
                "Low initial liquidity: {:.2} SOL < {:.2} SOL threshold",
                initial_sol, self.config.min_liquidity_sol
            ));
        }

        // 5. Option 2: Require Verified Socials Check
        if self.config.require_socials && is_pump_fun && !has_socials {
            let latency = start.elapsed().as_micros() as u64;
            return SecurityCheckReport {
                token_mint: token_mint.to_string(),
                freeze_authority_revoked: freeze_auth_revoked,
                mint_authority_revoked: mint_auth_revoked,
                lp_burned_pct: 100.0,
                creator_holding_pct,
                initial_sol,
                status: SafetyStatus::Rejected("No verified socials (Twitter/Telegram) linked in launch metadata".to_string()),
                execution_latency_us: latency,
            };
        }

        // 6. Option 2: Max Dev Initial Buy Check (Anti-Insider Bundles)
        if let Some(max_dev) = self.config.max_dev_buy_sol {
            if let Some(dev_spent) = dev_buy_sol {
                if dev_spent > max_dev {
                    let latency = start.elapsed().as_micros() as u64;
                    return SecurityCheckReport {
                        token_mint: token_mint.to_string(),
                        freeze_authority_revoked: freeze_auth_revoked,
                        mint_authority_revoked: mint_auth_revoked,
                        lp_burned_pct: if is_pump_fun { 100.0 } else { 0.0 },
                        creator_holding_pct,
                        initial_sol,
                        status: SafetyStatus::Rejected(format!(
                            "Dev initial buy {:.2} SOL exceeds max threshold {:.2} SOL (insider bundle risk)",
                            dev_spent, max_dev
                        )),
                        execution_latency_us: latency,
                    };
                }
            }
        }

        // 7. Item 3: Dev Skin-in-the-Game: Minimum Initial Buy Check
        if let Some(min_dev) = self.config.min_dev_buy_sol {
            if is_pump_fun {
                let dev_spent = dev_buy_sol.unwrap_or(0.0);
                if dev_spent < min_dev {
                    let latency = start.elapsed().as_micros() as u64;
                    return SecurityCheckReport {
                        token_mint: token_mint.to_string(),
                        freeze_authority_revoked: freeze_auth_revoked,
                        mint_authority_revoked: mint_auth_revoked,
                        lp_burned_pct: 100.0,
                        creator_holding_pct,
                        initial_sol,
                        status: SafetyStatus::Rejected(format!(
                            "Dev skin-in-the-game insufficient: dev bought {:.2} SOL < {:.2} SOL threshold",
                            dev_spent, min_dev
                        )),
                        execution_latency_us: latency,
                    };
                }
            }
        }

        let latency = start.elapsed().as_micros() as u64;
        let status = if warnings.is_empty() {
            SafetyStatus::Safe
        } else {
            SafetyStatus::Warning(warnings)
        };

        SecurityCheckReport {
            token_mint: token_mint.to_string(),
            freeze_authority_revoked: freeze_auth_revoked,
            mint_authority_revoked: mint_auth_revoked,
            lp_burned_pct: if is_pump_fun { 100.0 } else { 0.0 },
            creator_holding_pct,
            initial_sol,
            status,
            execution_latency_us: latency,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reject_honeypot_active_freeze() {
        let filter = RugFilter::new(RugFilterConfig::default());
        let report = filter.evaluate_fast(
            "FakeMint11111111111111111111111111111111111",
            false,
            5.0,
            false, // freeze NOT revoked!
            true,  // mint revoked
            2.0,   // creator 2%
            true,  // has socials
            None,
        );

        match report.status {
            SafetyStatus::Rejected(reason) => {
                assert!(reason.contains("Freeze authority enabled"));
            }
            _ => panic!("Expected rejection due to active freeze authority"),
        }
    }

    #[test]
    fn test_reject_unrevoked_mint_authority() {
        let filter = RugFilter::new(RugFilterConfig::default());
        let report = filter.evaluate_fast(
            "FakeMint22222222222222222222222222222222222",
            false,
            5.0,
            true,  // freeze revoked
            false, // mint NOT revoked!
            2.0,
            true,
            None,
        );

        match report.status {
            SafetyStatus::Rejected(reason) => {
                assert!(reason.contains("Mint authority not revoked"));
            }
            _ => panic!("Expected rejection due to unrevoked mint authority"),
        }
    }

    #[test]
    fn test_reject_creator_insider_bundle() {
        let filter = RugFilter::new(RugFilterConfig::default());
        let report = filter.evaluate_fast(
            "FakeMint33333333333333333333333333333333333",
            false,
            5.0,
            true,
            true,
            45.0, // Creator holding 45% (exceeds default 25%)
            true,
            None,
        );

        match report.status {
            SafetyStatus::Rejected(reason) => {
                assert!(reason.contains("Creator holding 45.0% exceeds max threshold"));
            }
            _ => panic!("Expected rejection due to creator holding 45%"),
        }
    }

    #[test]
    fn test_safe_pump_fun_launch() {
        let filter = RugFilter::new(RugFilterConfig::default());
        let report = filter.evaluate_fast(
            "PumpMint44444444444444444444444444444444444",
            true, // pump.fun
            30.0,
            true,
            true,
            4.5,
            true,
            Some(0.5),
        );

        assert_eq!(report.status, SafetyStatus::Safe);
        assert_eq!(report.lp_burned_pct, 100.0);
    }

    #[test]
    fn test_option_2_reject_no_socials() {
        let filter = RugFilter::new(RugFilterConfig {
            require_socials: true,
            ..Default::default()
        });
        let report = filter.evaluate_fast(
            "NoSocialMint55555555555555555555555555555555",
            true,
            30.0,
            true,
            true,
            4.0,
            false, // NO SOCIALS!
            Some(0.5),
        );

        match report.status {
            SafetyStatus::Rejected(reason) => {
                assert!(reason.contains("No verified socials"));
            }
            _ => panic!("Expected rejection due to missing socials"),
        }
    }

    #[test]
    fn test_option_2_reject_excessive_dev_buy() {
        let filter = RugFilter::new(RugFilterConfig {
            max_dev_buy_sol: Some(2.0),
            ..Default::default()
        });
        let report = filter.evaluate_fast(
            "HugeDevBuy6666666666666666666666666666666666",
            true,
            30.0,
            true,
            true,
            4.0,
            true,
            Some(5.5), // Dev bought 5.5 SOL (exceeds max 2.0 SOL)
        );

        match report.status {
            SafetyStatus::Rejected(reason) => {
                assert!(reason.contains("Dev initial buy 5.50 SOL exceeds max threshold 2.00 SOL"));
            }
            _ => panic!("Expected rejection due to excessive dev buy"),
        }
    }

    #[test]
    fn test_min_dev_buy_skin_in_the_game() {
        let filter = RugFilter::new(RugFilterConfig {
            min_dev_buy_sol: Some(0.40),
            ..Default::default()
        });

        // 1. Rejected if dev bought too little (0.05 SOL < 0.40 SOL)
        let report_low = filter.evaluate_fast(
            "SpamZeroDevBuy111111111111111111111111111111",
            true,
            30.0,
            true,
            true,
            0.5,
            true,
            Some(0.05),
        );
        match report_low.status {
            SafetyStatus::Rejected(reason) => {
                assert!(reason.contains("Dev skin-in-the-game insufficient: dev bought 0.05 SOL < 0.40 SOL"));
            }
            _ => panic!("Expected rejection due to low dev skin-in-the-game"),
        }

        // 2. Accepted if dev bought >= 0.40 SOL
        let report_ok = filter.evaluate_fast(
            "GoodDevBuy2222222222222222222222222222222222",
            true,
            30.0,
            true,
            true,
            2.0,
            true,
            Some(0.50),
        );
        assert_eq!(report_ok.status, SafetyStatus::Safe);
    }
}
