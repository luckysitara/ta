use crate::types::AssetType;

const CRYPTO_BASES: &[&str] = &[
    "SOL", "BTC", "ETH", "XRP", "ADA", "DOGE", "LTC", "BCH", "DOT", "AVAX", "LINK", "JUP", "RAY", "BONK"
];

const CRYPTO_QUOTES: &[&str] = &["USDT", "USDC", "USD"];

#[allow(dead_code)]
pub struct ResolvedSymbol {
    pub raw: String,
    pub canonical: String, // e.g. SOL-USD
    pub base: String,      // e.g. SOL
    pub asset_type: AssetType,
}

pub fn resolve_symbol(raw: &str) -> ResolvedSymbol {
    let raw_clean = raw.trim().to_uppercase();
    let compact = raw_clean.replace("-", "").replace("/", "");

    // Check if it matches known crypto bases
    for &base in CRYPTO_BASES {
        if compact == base {
            return ResolvedSymbol {
                raw: raw.to_string(),
                canonical: format!("{}-USD", base),
                base: base.to_string(),
                asset_type: AssetType::Crypto,
            };
        }
        for &quote in CRYPTO_QUOTES {
            if compact == format!("{}{}", base, quote) {
                return ResolvedSymbol {
                    raw: raw.to_string(),
                    canonical: format!("{}-USD", base),
                    base: base.to_string(),
                    asset_type: AssetType::Crypto,
                };
            }
        }
    }

    if raw_clean.contains("-USD") || raw_clean.contains("USDT") {
        let base = raw_clean.split('-').next().unwrap_or(&raw_clean).to_string();
        return ResolvedSymbol {
            raw: raw.to_string(),
            canonical: format!("{}-USD", base),
            base,
            asset_type: AssetType::Crypto,
        };
    }

    // Default to Stock if not matched as crypto
    ResolvedSymbol {
        raw: raw.to_string(),
        canonical: raw_clean.clone(),
        base: raw_clean,
        asset_type: AssetType::Stock,
    }
}

pub fn subreddits_for(base_symbol: &str, asset_type: AssetType) -> Vec<&'static str> {
    match asset_type {
        AssetType::Crypto => match base_symbol.to_uppercase().as_str() {
            "SOL" => vec!["solana", "CryptoCurrency", "CryptoMarkets"],
            "BTC" => vec!["Bitcoin", "CryptoCurrency", "CryptoMarkets"],
            "ETH" => vec!["ethereum", "CryptoCurrency", "CryptoMarkets"],
            _ => vec!["CryptoCurrency", "CryptoMarkets"],
        },
        AssetType::Stock => vec!["wallstreetbets", "stocks", "investing"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_sol() {
        let sym = resolve_symbol("SOL");
        assert_eq!(sym.base, "SOL");
        assert_eq!(sym.canonical, "SOL-USD");
        assert_eq!(sym.asset_type, AssetType::Crypto);

        let subs = subreddits_for(&sym.base, sym.asset_type);
        assert!(subs.contains(&"solana"));
    }

    #[test]
    fn test_resolve_stock() {
        let sym = resolve_symbol("NVDA");
        assert_eq!(sym.base, "NVDA");
        assert_eq!(sym.asset_type, AssetType::Stock);
    }
}
