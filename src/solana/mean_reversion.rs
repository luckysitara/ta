use anyhow::Result;
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use crate::solana::execution::OnChainExecutor;
use crate::solana::sniper::PortfolioState;
use crate::solana::SOL_MINT;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeanReversionSettings {
    pub total_budget_sol: f64,
    pub buy_amount_sol: f64,
    pub max_concurrent_positions: usize,
    pub jito_tip_sol: f64,
    pub tracked_tokens: Vec<TrackedToken>,
    pub rsi_period: usize,
    pub rsi_oversold: f64,
    pub rsi_exit: f64,
    pub price_sample_interval_secs: u64,
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub max_hold_mins: u64,
    pub poll_interval_secs: u64,
    pub is_paper_trading: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedToken {
    pub mint: String,
    pub symbol: String,
}

impl Default for MeanReversionSettings {
    fn default() -> Self {
        Self {
            total_budget_sol: 0.2,
            buy_amount_sol: 0.03,
            max_concurrent_positions: 3,
            jito_tip_sol: 0.0001,
            tracked_tokens: vec![
                TrackedToken { mint: "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263".to_string(), symbol: "BONK".to_string() },
                TrackedToken { mint: "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm".to_string(), symbol: "WIF".to_string() },
                TrackedToken { mint: "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr".to_string(), symbol: "POPCAT".to_string() },
                TrackedToken { mint: "MEW1gQWJ3nEXg2qgERiKu7FAFj79PHvQVREQUzScPP5".to_string(), symbol: "MEW".to_string() },
                TrackedToken { mint: "A8C3xuqscfmyLrte3VmTqrAq8kgMASius9AFNANwpump".to_string(), symbol: "FWOG".to_string() },
                TrackedToken { mint: "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN".to_string(), symbol: "JUP".to_string() },
                TrackedToken { mint: "ukHH6c7mMyiWCf1b9pnWe25TSpkDDt3H5pQZgZ74J82".to_string(), symbol: "BOME".to_string() },
                TrackedToken { mint: "HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3".to_string(), symbol: "PYTH".to_string() },
                TrackedToken { mint: "rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBXr".to_string(), symbol: "RENDER".to_string() },
                TrackedToken { mint: "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R".to_string(), symbol: "RAY".to_string() },
                TrackedToken { mint: "orcaEKTdK7LKz57vaAYr9QeNsVEPfiu6QeMU1kektZE".to_string(), symbol: "ORCA".to_string() },
                TrackedToken { mint: "KMNo3nJsBXfcpJTVhZcXLW7RmTwTt4GVFE7suUBo9sS".to_string(), symbol: "KMNO".to_string() },
                TrackedToken { mint: "DriFtupJYLTosbwoN8koMbEYSx54aFAVLddWsbksjwg7".to_string(), symbol: "DRIFT".to_string() },
                TrackedToken { mint: "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL".to_string(), symbol: "JTO".to_string() },
                TrackedToken { mint: "HeLp6NuQkmYB4pYWo2zYs22mESHXPQYzXbB8n4V98jwC".to_string(), symbol: "AI16Z".to_string() },
                TrackedToken { mint: "CzLSujWBLFsSjncfkh59rUFqvafWcY5tzedWJSuypump".to_string(), symbol: "GOAT".to_string() },
                TrackedToken { mint: "ED5nyyWEzpPPiWimP8vYm7sD7TD3LAt3Q3gRTWHzPJBY".to_string(), symbol: "MOODENG".to_string() },
                TrackedToken { mint: "2qEHjDLDLbuBgRYvsxhc5RefwhumuAZgnusqWqb86476".to_string(), symbol: "PNUT".to_string() },
                TrackedToken { mint: "9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump".to_string(), symbol: "FIDA".to_string() },
                TrackedToken { mint: "Grass7B4RdKfBCjTKgSqnXkqjwiGvQyFbuSCUJr3XXjs".to_string(), symbol: "GRASS".to_string() },
                TrackedToken { mint: "ioNETvc5NiB92q6Zk3YvNsp1qZtQ6Yh9B8y7T78Kpump".to_string(), symbol: "IO".to_string() },
                TrackedToken { mint: "TNSRxcUxoT9xBG3de7PiJyTDYu7kskLqcpddxnEJAS6".to_string(), symbol: "TNSR".to_string() },
                TrackedToken { mint: "ZEUS1a8r3a5hxBRqaHG3ux7A5upF2d84JwEP3STyx3r".to_string(), symbol: "ZEUS".to_string() },
                TrackedToken { mint: "WENWENvqqNya429ubCdXr81ZmD69brwQaaBYY6p3LCU".to_string(), symbol: "WEN".to_string() },
                TrackedToken { mint: "hntyVP6YFm1Hg25TN9WGLqM12b8TQmcknKrdu1oxWux".to_string(), symbol: "HNT".to_string() },
                TrackedToken { mint: "nosXBVoaCTtYdLvKY6Csb4AC8JCdQKKAaWYtx2ZMoo7".to_string(), symbol: "NOS".to_string() },
                TrackedToken { mint: "SHDWyBxihqiCj6YekG2GUr7wqKLeLAMK1gHZck9pL6y".to_string(), symbol: "SHDW".to_string() },
                TrackedToken { mint: "StepAscQoEioFxxWGnh2sLBDFp9d8rvKz2Yp39iDpyT".to_string(), symbol: "STEP".to_string() },
                TrackedToken { mint: "SLNDpmoWTVADgEdndQI29yKitggsuSTvmYUjECzUUCV".to_string(), symbol: "SLND".to_string() },
                TrackedToken { mint: "MNDEFzGvMt87ueuHvVU9VcTqsAP5b3fTGPsHuuPA5ey".to_string(), symbol: "MNDE".to_string() },
                TrackedToken { mint: "BLZEEuZUBVqFhj8adcCFPJvPVCiCyVmh3hkJMrU8KuJA".to_string(), symbol: "BLZE".to_string() },
                TrackedToken { mint: "RLBxxFkseAZ4RgJH3Sqn8jXxhmGoz9jWxDNJMh8LMTQ".to_string(), symbol: "RLB".to_string() },
                TrackedToken { mint: "LinkhB3afbBKb2EQQu7s7umdZceV3wcvAUJhQAfQ23L".to_string(), symbol: "LINK".to_string() },
                TrackedToken { mint: "AavE1kKKnesPw4MuRJmJ9jZs9QzEE8CPxQ3ViczUDfc1".to_string(), symbol: "AAVE".to_string() },
                TrackedToken { mint: "72QvBVwpxqmheEPfaCwWSWqEFsUy3rhWt6JhQBMNTwD1".to_string(), symbol: "ENA".to_string() },
                TrackedToken { mint: "pumpCmXqMfrsAkQ5r49WcJnRayYRqmXz6ae8H7H9Dfn".to_string(), symbol: "PUMP".to_string() },
                TrackedToken { mint: "6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN".to_string(), symbol: "TRUMP".to_string() },
                TrackedToken { mint: "1NJMqVM4PadjuzYmeB7zV7q7DV8oB3ExaQCd9x6KsLz".to_string(), symbol: "INJ".to_string() },
                TrackedToken { mint: "APhcqtzE73es3KAGiVksZFMLGwJDiAey5qZKUrQHEHfS".to_string(), symbol: "SOFID".to_string() },
                TrackedToken { mint: "LAYER4xPpTCb3QL8S9u41EAhAX7mhBn8Q6xMTwY2Yzc".to_string(), symbol: "LAYER".to_string() },
                TrackedToken { mint: "2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv".to_string(), symbol: "PENGU".to_string() },
                TrackedToken { mint: "4qQeZ5LwSz6HuupUu8jCtgXyW1mYQcNbFAW1sWZp89HL".to_string(), symbol: "CAKE".to_string() },
                TrackedToken { mint: "MEFNBXixkEbait3xn9bkm8WsJzXtVsaJEn4c8Sam21u".to_string(), symbol: "ME".to_string() },
                TrackedToken { mint: "rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBof".to_string(), symbol: "RENDER".to_string() },
                TrackedToken { mint: "98sMhvDwXj1RQi5c5Mndm3vPe9cBqPrbLaufMXFNMh5g".to_string(), symbol: "HYPE".to_string() },
                TrackedToken { mint: "WLFinEv6ypjkczcS83FZqFpgFZYwQXutRbxGe7oC16g".to_string(), symbol: "WLFI".to_string() },
                TrackedToken { mint: "SPCXxcqXj6e5dJDVNovHN8744zkbhM2bYudU45BimGb".to_string(), symbol: "SPCX".to_string() },
                TrackedToken { mint: "HsRpHQn6VbyMs5b5j5SV6xQ2VvpvvCCzu19GjytVSCoz".to_string(), symbol: "STRK".to_string() },
                TrackedToken { mint: "72puLt71H93Z9CzHuBRTwFpL4TG3WZUhnoCC7p8gxigu".to_string(), symbol: "USDGO".to_string() },
                TrackedToken { mint: "Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh".to_string(), symbol: "NVDAX".to_string() },
                TrackedToken { mint: "3iQL8BFS2vE7mww4ehAqQHAsbmRNCrPxizWAT2Zfyr9y".to_string(), symbol: "VIRTUAL".to_string() },
                TrackedToken { mint: "A7bdiYdS5GjqGFtxf17ppRHtDKPkkRqbKtR27dxvQXaS".to_string(), symbol: "ZEC".to_string() },
                TrackedToken { mint: "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn".to_string(), symbol: "JITOSOL".to_string() },
                TrackedToken { mint: "Xs3oZwbHvqis4NYcf4YKWmEia2eC84wSiVrcYcTqpH8".to_string(), symbol: "SPCXX".to_string() },
                TrackedToken { mint: "Xs7ZdzSHLU9ftNJsii5fCeJhoRWSC32SQGzGQtePxNu".to_string(), symbol: "COINX".to_string() },
                TrackedToken { mint: "6eftxVbSAunVEoxUWdGhPdxg5UdsJ8Wkwy5w5YFuxouw".to_string(), symbol: "CHZ".to_string() },
                TrackedToken { mint: "EPeUFDgHRxs9xxEPVaL6kfGQvCon7jmAWKVUHuux1Tpz".to_string(), symbol: "BAT".to_string() },
                TrackedToken { mint: "u9nmK5sQovm6ACVCQbbq8xUMpFqdPSYxdxVwXUX4sjY".to_string(), symbol: "ORDI".to_string() },
                TrackedToken { mint: "eEUiUs4JWYZrp72djAGF1A8PhpR6rHphGeGN7GbVLp6".to_string(), symbol: "GALA".to_string() },
                TrackedToken { mint: "85VBFQZC9TZkfaptBWjvUw7YbZjy52A6mjtPGjstQAmQ".to_string(), symbol: "W".to_string() },
                TrackedToken { mint: "XsueG8BtpquVJX9LVLLEGuViXUungE6WmK5YZ3p3bd1".to_string(), symbol: "CRCLX".to_string() },
                TrackedToken { mint: "Dz9mQ9NzkBcCsuGPFJ3r1bS4wgqKMHBPiVuniW8Mbonk".to_string(), symbol: "USELESS".to_string() },
                TrackedToken { mint: "2qEHjDLDLbuBgRYvsxhc5D6uDWAivNFZGan56P1tpump".to_string(), symbol: "PNUT".to_string() },
                TrackedToken { mint: "XsYzw7XHj8K2WHeoeNQVyevMmzjzXfcJ8tQ4FmdN1DL".to_string(), symbol: "ANTX".to_string() },
                TrackedToken { mint: "KeGv7bsfR4MheC1CkmnAVceoApjrkvBhHYjWb67ondo".to_string(), symbol: "TSLAON".to_string() },
                TrackedToken { mint: "XsDoVfqeBukxuZHWhdvWHBhgEHjGNst4MLodqsJHzoB".to_string(), symbol: "TSLAX".to_string() },
                TrackedToken { mint: "CARDSccUMFKoPRZxt5vt3ksUbxEFEcnZ3H2pd3dKxYjp".to_string(), symbol: "CARDS".to_string() },
                TrackedToken { mint: "6AJcP7wuLwmRYLBNbi825wgguaPsWzPBEHcHndpRpump".to_string(), symbol: "VINE".to_string() },
                TrackedToken { mint: "6DNSN2BJsaPFdFFc1zP37kkeNe4Usc1Sqkzr9C9vPWcU".to_string(), symbol: "TBTC".to_string() },
                TrackedToken { mint: "chipCAT7vi5CZtbZsn9z7iMPXvFwyAnKz3QFu8XVuHm".to_string(), symbol: "CHIP".to_string() },
                TrackedToken { mint: "C1MHyoTJpRTeS9AQCyspNVu2EWAYCZwmJ1jNkEArFP1f".to_string(), symbol: "APE".to_string() },
                TrackedToken { mint: "HUMA1821qVDKta3u2ovmfDQeW2fSQouSKE8fkF44wvGw".to_string(), symbol: "HUMA".to_string() },
                TrackedToken { mint: "ZBCNpuD7YMXzTHB2fhGkGi78MNsHGLRXUhRewNRm9RU".to_string(), symbol: "ZBCN".to_string() },
                TrackedToken { mint: "4Njvi3928U3figEF5tf8xvjLC5GqUN33oe4XTJNe7xXC".to_string(), symbol: "T".to_string() },
                TrackedToken { mint: "CeFbGYXDmkyfo1TXXzzZ512mtnCCewNohu6V15vondo".to_string(), symbol: "FXION".to_string() },
                TrackedToken { mint: "C98A4nkJXhpVZNAZdHUA95RpTF3T4whtQubL3YobiUX9".to_string(), symbol: "C98".to_string() },
                TrackedToken { mint: "METvsvVRapdj9cFLzq4Tr43xK4tAjQfwX76z3n6mWQL".to_string(), symbol: "MET".to_string() },
                TrackedToken { mint: "A1t2UviBYwyfYZDJyKY2W6Td8ritgsCriUDuNaAQN49S".to_string(), symbol: "BLESS".to_string() },
                TrackedToken { mint: "Dm5BxyMetG3Aq5PaG1BrG7rBYqEMtnkjvPNMExfacVk7".to_string(), symbol: "ATH".to_string() },
                TrackedToken { mint: "G7vQWurMkMMm2dU3iZpXYFTHT9Biio4F4gZCrwFpKNwG".to_string(), symbol: "BIRB".to_string() },
                TrackedToken { mint: "AAeENcfHbTExuTvs4q7r9Bjax98Dg6BGX3aMph4bTLdK".to_string(), symbol: "RIF".to_string() },
                TrackedToken { mint: "XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp".to_string(), symbol: "AAPLX".to_string() },
                TrackedToken { mint: "XsvNBAYkrDRNhA7wPHQfX3ZUXZyZLdnCQDfHZ56bzpg".to_string(), symbol: "HOODX".to_string() },
                TrackedToken { mint: "XsoCS1TfEyfFhfvj8EtZ528L3CaKBDBRqRapnBbDF2W".to_string(), symbol: "SPYX".to_string() },
                TrackedToken { mint: "6VdpC1hcx7byFtJ5M6Xhspa6Uup5rZHruzXvuJhXAyXm".to_string(), symbol: "ATM".to_string() },
                TrackedToken { mint: "4eDf52YYzL6i6gbZ6FXqrLUPXbtP61f1gPSFM66M4XHe".to_string(), symbol: "SOON".to_string() },
                TrackedToken { mint: "FQgtfugBdpFN7PZ6NdPrZpVLDBrPGxXesi4gVu3vErhY".to_string(), symbol: "BMT".to_string() },
                TrackedToken { mint: "J3NKxxXZcnNiMjKw9hYb2K4LUxgwB6t1FtPtQVsv3KFr".to_string(), symbol: "SPX".to_string() },
                TrackedToken { mint: "27G8MtK7VtTcCHkpASjSDdkWWYfoqT6ggEuKidVJidD4".to_string(), symbol: "JLP".to_string() },
                TrackedToken { mint: "Bybit2vBJGhPF52GBdNaQfUJ6ZpThSgHBobjWZpLPb4B".to_string(), symbol: "BBSOL".to_string() },
                TrackedToken { mint: "AUSD1jCcCyPLybk1YnvPWsHQSrZ46dxwoMniN4N2UEB9".to_string(), symbol: "AUSD".to_string() },
                TrackedToken { mint: "TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo".to_string(), symbol: "TTWO".to_string() },
                TrackedToken { mint: "bioJ9JTqW62MLz7UKHU69gtKhPpGi1BQhccj2kmSvUJ".to_string(), symbol: "BIO".to_string() },
                TrackedToken { mint: "SLXdx4BUt2v9uJQNzWqSfzTJ9UKLUDsvxHFMEEdrfgq".to_string(), symbol: "SLX".to_string() },
                TrackedToken { mint: "4UbvZiomFvXDnZSz6vdHiDNiHozH2ykTEqjhhbVHiv9z".to_string(), symbol: "XUSD".to_string() },
                TrackedToken { mint: "Eh6XEPhSwoLv5wFApukmnaVSHQ6sAnoD9BmgmwQoN2sN".to_string(), symbol: "SUSDE".to_string() },
                TrackedToken { mint: "gEGtLTPNQ7jcg25zTetkbmF7teoDLcrfTnQfmn2ondo".to_string(), symbol: "NVDAON".to_string() },
                TrackedToken { mint: "Xs3eBt7uRfJX8QUs4suhyU8p2M6DoUDrJyWBa8LLZsg".to_string(), symbol: "AMZNX".to_string() },
                TrackedToken { mint: "FUAfBo2jgks6gB4Z4LfZkqSZgzNucisEHqnNebaRxM1P".to_string(), symbol: "MELANIA".to_string() },
                TrackedToken { mint: "AymATz4TCL9sWNEEV9Kvyz45CHVhDZ6kUgjTJPzLpU9P".to_string(), symbol: "XAUT0".to_string() },
                TrackedToken { mint: "WCTk5xWdn5SYg56twGj32sUF3W4WFQ48ogezLBuYTBY".to_string(), symbol: "WCT".to_string() },
                TrackedToken { mint: "8x5VqbHA8D7NkD52uNuS5nnt3PwA8pLD34ymskeSo2Wn".to_string(), symbol: "ZEREBRO".to_string() },
                TrackedToken { mint: "CtzPWv73Sn1dMGVU3ZtLv9yWSyUAanBni19YWDaznnkn".to_string(), symbol: "XBTC".to_string() },
                TrackedToken { mint: "ChVzxWRmrTeSgwd3Ui3UumcN8KX7VK3WaD4KGeSKpypj".to_string(), symbol: "SUSHI".to_string() },
                TrackedToken { mint: "5Y8NV33Vv7WbnLfq3zBcKSdYPrk7g2KoiQoe7M2tcxp5".to_string(), symbol: "ONYC".to_string() },
                TrackedToken { mint: "J6pQQ3FAcJQeWPPGppWRb4nM8jU3wLyYbRrLh7feMfvd".to_string(), symbol: "2Z".to_string() },
                TrackedToken { mint: "bbahNA5vT9WJeYft8tALrH1LXWffjwqVoUbqYa1ondo".to_string(), symbol: "GOOGLON".to_string() },
                TrackedToken { mint: "GJAFwWjJ3vnTsrQVabjBVK2TYB1YtRCQXRDfDgUnpump".to_string(), symbol: "ACT".to_string() },
                TrackedToken { mint: "G8LfyGVsjsLzetJ5RWZVAhMo4H9cb58ET1Z6gEZJQdPM".to_string(), symbol: "AVA".to_string() },
                TrackedToken { mint: "ZEUS1aR7aX8DFFJf5QjWj2ftDDdNTroMNGo8YoQm3Gq".to_string(), symbol: "ZEUS".to_string() },
                TrackedToken { mint: "4Zp52aF4hZi9fzH19xpbWKYKQvgLyCN67KFbrQDqeTKh".to_string(), symbol: "ZAMA".to_string() },
                TrackedToken { mint: "A1KLoBrKBde8Ty9qtNQUtq3C2ortoC3u7twggz7sEto6".to_string(), symbol: "USDY".to_string() },
                TrackedToken { mint: "7i5KKsX2weiTkry7jA4ZwSuXGhs5eJBEjY8vVxR4pfRx".to_string(), symbol: "GMT".to_string() },
                TrackedToken { mint: "Df6yfrKC8kZE3KNkrHERKzAetSxbrWeniQfyJY4Jpump".to_string(), symbol: "CHILLGUY".to_string() },
                TrackedToken { mint: "BZLbGTNCSFfoth2GYDtwr7e4imWzpR5jqcUuGEwr646K".to_string(), symbol: "IO".to_string() },
                TrackedToken { mint: "4vMsoUT2BWatFweudnQM1xedRLfJgJ7hswhcpz4xgBTy".to_string(), symbol: "HONEY".to_string() },
                TrackedToken { mint: "74SBV4zDXxTRgv1pEMoECskKBkZHc2yGPnc7GYVepump".to_string(), symbol: "SWARMS".to_string() },
                TrackedToken { mint: "GzX1ireZDU865FiMaKrdVB1H6AE8LAqWYCg6chrMrfBw".to_string(), symbol: "FRXUSD".to_string() },
                TrackedToken { mint: "14zP2ToQ79XWvc7FQpm4bRnp9d6Mp1rFfsUW3gpLcRX".to_string(), symbol: "AIXBT".to_string() },
                TrackedToken { mint: "DvjbEsdca43oQcw2h3HW1CT7N3x5vRcr3QrvTUHnXvgV".to_string(), symbol: "DOOD".to_string() },
                TrackedToken { mint: "Dfh5DzRgSvvCFDoYc2ciTkMrbDfRKybA4SoFbPmApump".to_string(), symbol: "PIPPIN".to_string() },
                TrackedToken { mint: "Xsv9hRk1z5ystj9MhnA7Lq4vjSsLwzL2nxrwmwtD3re".to_string(), symbol: "GLDX".to_string() },
                TrackedToken { mint: "7Zhxshgt7Ft6pHFYMrHE1epWdWre7sJ1Af1GhEUnitas".to_string(), symbol: "UP".to_string() },
                TrackedToken { mint: "Xs8S1uUs1zvS2p7iwtsG3b6fkhpvmwz4GYU3gWAmWHZ".to_string(), symbol: "QQQX".to_string() },
                TrackedToken { mint: "BeGY8KqKxboEwRbJd1q9H2K829jS4Rc5dEyNMYXCbV5p".to_string(), symbol: "NPC".to_string() },
                TrackedToken { mint: "XsCPL9dNWBMvFtTmwcCA5v3xWPSMEBCszbQdiLLq6aN".to_string(), symbol: "GOOGLX".to_string() },
                TrackedToken { mint: "Xsa62P5mvPszXL1krVUnU5ar38bBSVcWAB6fmPCo5Zu".to_string(), symbol: "METAX".to_string() },
                TrackedToken { mint: "BPxxfRCXkUVhig4HS1Lh7kZqV6SPJhzfEk4x6fVBjPCy".to_string(), symbol: "BP".to_string() },
                TrackedToken { mint: "9PR7nCP9DpcUotnDPVLUBUZKu5WAYkwrCUx9wDnSpump".to_string(), symbol: "BAN".to_string() },
                TrackedToken { mint: "KENJSUYLASHUMfHyy5o4Hp2FdNqZg1AsUPhfH2kYvEP".to_string(), symbol: "GRIFFAIN".to_string() },
                TrackedToken { mint: "HNg5PYJmtqcmzXrv6S9zP1CDKk5BgDuyFBxbvNApump".to_string(), symbol: "ALCH".to_string() },
                TrackedToken { mint: "AvZZF1YaZDziPY2RCK4oJrRVrbN3mTD9NL24hPeaZeUj".to_string(), symbol: "SYRUPUSDC".to_string() },
                TrackedToken { mint: "XsP7xzNPvEHS1m6qfanPUGjNmdnmsLKEoNAnHjdxxyZ".to_string(), symbol: "MSTRX".to_string() },
                TrackedToken { mint: "sUSDai6Y3GxysDEtA9BVcEFTaog6UZpYUVxJiMhAKYE".to_string(), symbol: "SUSDAI".to_string() },
                TrackedToken { mint: "HrYNm6jTQ71LoFphjVKBTdAE4uja7WsmLG8VxB8ondo".to_string(), symbol: "QQQON".to_string() },
                TrackedToken { mint: "DKu9kykSfbN5LBfFXtNNDPaX35o4Fv6vJ9FKk7pZpump".to_string(), symbol: "AVA".to_string() },
                TrackedToken { mint: "LBTCgU4b3wsFKsPwBn1rRZDx5DoFutM6RPiEt1TPDsY".to_string(), symbol: "LBTC".to_string() },
                TrackedToken { mint: "2j5SaS7xy776qCBpyPQbZjyQSAtKiFgrwjfErthnW2ZM".to_string(), symbol: "CLAUDIA".to_string() },
                TrackedToken { mint: "FMQjDvT1GztVxdvYgMBEde4L54fftFGx9m5GmbqeJGM5".to_string(), symbol: "PORTAL".to_string() },
                TrackedToken { mint: "iy11ytbSGcUnrjE6Lfv78TFqxKyUESfku1FugS9ondo".to_string(), symbol: "SLVON".to_string() },
                TrackedToken { mint: "FeR8VBqNRSUD5NtXAj2n3j1dAHkZHfyDktKuLXD4pump".to_string(), symbol: "JELLYJELLY".to_string() },
                TrackedToken { mint: "Fz9edBpaURPPzpKVRR1A8PENYDEgHqwx5D5th28ondo".to_string(), symbol: "MUON".to_string() },
                TrackedToken { mint: "9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump".to_string(), symbol: "ANSEM".to_string() },
                TrackedToken { mint: "3jUf2RTyXp867piSB2dt8uUcNiLDW58asjGtXkRAkBbe".to_string(), symbol: "GUN".to_string() },
                TrackedToken { mint: "3b8X44fLF9ooXaUm3hhSgjpmVs6rZZ3pPoGnGahc3Uu7".to_string(), symbol: "PRIME".to_string() },
                TrackedToken { mint: "wzAyQTorWyoVXuJKj2x8EqKEGJpS13z6EWE9z5Aondo".to_string(), symbol: "SPCXON".to_string() },
                TrackedToken { mint: "3cVuTnMj8FeqK82PDgK4JzKnLP7LM6GauqTyqnxtcSk1".to_string(), symbol: "ASR".to_string() },
                TrackedToken { mint: "ABt79MkRXUsoHuV2CVQT32YMXQhTparKFjmidQxgiQ6E".to_string(), symbol: "ORDER".to_string() },
                TrackedToken { mint: "fDxs5y12E7x7jBwCKBXGqt71uJmCWsAQ3Srkte6ondo".to_string(), symbol: "METAON".to_string() },
                TrackedToken { mint: "5z3EqYQo9HiCEs3R84RCDMu2n7anpDMxRhdK8PSWmrRC".to_string(), symbol: "PONKE".to_string() },
                TrackedToken { mint: "ARXwZkNAtzPfdcoqQiduJn8EPv9fKiDfGn2KyggyDrFs".to_string(), symbol: "ARX".to_string() },
                TrackedToken { mint: "1FWZtdWN7y38BSXGzbs8D6Shk88oL9atDNgbVz9ondo".to_string(), symbol: "AVGOON".to_string() },
                TrackedToken { mint: "7ZCm8WBN9aLa3o47SoYctU6iLdj7wkGG5SV2hE5CgtD5".to_string(), symbol: "ELON".to_string() },
                TrackedToken { mint: "CLoUDKc4Ane7HeQcPpE3YHnznRxhMimJ4MyaUqyHFzAu".to_string(), symbol: "SANC".to_string() },
                TrackedToken { mint: "DEW9dSN6QpWyNthphCpMmAbZP1Q4cEKR9xQXAri98WDP".to_string(), symbol: "SI".to_string() },
                TrackedToken { mint: "CASHx9KJUStyftLFWGvEVf59SGeG9sh5FfcnZMVPCASH".to_string(), symbol: "CASH".to_string() },
                TrackedToken { mint: "DBRiDgJAMsM95moTzJs7M9LnkGErpbv9v6CUR1DXnUu5".to_string(), symbol: "DBR".to_string() },
                TrackedToken { mint: "CCDfDXZxzZtkZLuhY48gyKdXc5KywqpR8xEVHHh8ck1G".to_string(), symbol: "CGPT".to_string() },
                TrackedToken { mint: "keybg184d4vyXeQdFqs4o99YsMg7xBthxTJ6Ky3ondo".to_string(), symbol: "TSMON".to_string() },
                TrackedToken { mint: "BWsnyEa1XtsNRdgPaDoA1WVUonF7BBGZTd2zc72NQsWT".to_string(), symbol: "BLEND".to_string() },
                TrackedToken { mint: "METAewgxyPbgwsseH8T16a39CQ5VyVxZi9zXiDPY18m".to_string(), symbol: "MPLX".to_string() },
                TrackedToken { mint: "SKRbvo6Gf7GondiT3BbTfuRDPqLWei4j2Qy2NPGZhW3".to_string(), symbol: "SKR".to_string() },
                TrackedToken { mint: "6GmAFSYs4gk3FDao5FzzySQpPZaWsa4rUJHacpMpUNgx".to_string(), symbol: "STONK".to_string() },
                TrackedToken { mint: "J3umBWqhSjd13sag1E1aUojViWvPYA5dFNyqpKuX3WXj".to_string(), symbol: "HOME".to_string() },
                TrackedToken { mint: "HfsnTS5qtdStwec9DfBrunRqnAMYMMz1kjv9Hu9ondo".to_string(), symbol: "PLTRON".to_string() },
                TrackedToken { mint: "26f12PmBk77wQV1TzLe8XKkNBvMFggbuypxdtMLzNLzz".to_string(), symbol: "ZIG".to_string() },
                TrackedToken { mint: "9LzCMqDgTKYz9Drzqnpgee3SGa89up3a247ypMj2xrqM".to_string(), symbol: "AUDIO".to_string() },
                TrackedToken { mint: "61V8vBaqAGMpgDQi4JcAwo1dmBGHsyhzodcPqnEVpump".to_string(), symbol: "ARC".to_string() },
                TrackedToken { mint: "5UUH9RTDiSpq6HKS6bp4NdU9PNJpXRXuiw6ShBTBhgH2".to_string(), symbol: "TROLL".to_string() },
                TrackedToken { mint: "63LfDmNb3MQ8mw9MtZ2To9bEA2M71kZUUGq5tiJxcqj9".to_string(), symbol: "GIGA".to_string() },
                TrackedToken { mint: "PEAQjk7SRS6rXHVFFmpRr7zrC4g5ZuEebpwTxvaLr3b".to_string(), symbol: "PEAQ".to_string() },
                TrackedToken { mint: "FmysnoAQ7ifx3La6C7D9cM5tcpoTxEnakbnZPhrC2pJi".to_string(), symbol: "FLOCK".to_string() },
                TrackedToken { mint: "PERLEQKUNUp1dgFZ8EvyXHdN9d6ZQqfGxALDvfs6pDs".to_string(), symbol: "PRL".to_string() },
                TrackedToken { mint: "14Tqdo8V1FhzKsE3W2pFsZCzYPQxxupXRcqw9jv6ondo".to_string(), symbol: "AMZNON".to_string() },
                TrackedToken { mint: "FSz4ouiqXpHuGPcpacZfTzbMjScoj5FfzHkiyu2ondo".to_string(), symbol: "MSTRON".to_string() },
                TrackedToken { mint: "METAwkXcqyXKy1AtsSgJ8JiUHwGCafnZL38n3vYmeta".to_string(), symbol: "META".to_string() },
                TrackedToken { mint: "jupSoLaHXQiZZTSfEWMTRRgpnyFm8f6sZdosWBjx93v".to_string(), symbol: "JUPSOL".to_string() },
                TrackedToken { mint: "123mYEnRLM2LLYsJW3K6oyYh8uP1fngj732iG638ondo".to_string(), symbol: "AAPLON".to_string() },
                TrackedToken { mint: "82DNsTK61ZrgCHP6pfP32Eubcsp9h38d64E6X9ETEBBe".to_string(), symbol: "BAR".to_string() },
                TrackedToken { mint: "k18WJUULWheRkSpSquYGdNNmtuE2Vbw1hpuUi92ondo".to_string(), symbol: "SPYON".to_string() },
                TrackedToken { mint: "HcRLc9VDgjLeK154xDawfb1dmVJ98DoSqcwTHGqiDeJR".to_string(), symbol: "ZCAT".to_string() },
                TrackedToken { mint: "ApqLH7ovTBeEjgBMXH8udnBttwR9k5JEv9pNnVQV7Xsa".to_string(), symbol: "CORN".to_string() },
                TrackedToken { mint: "7dUKUopcNWW6CcU4eRxCHh1uiMh32zDrmGf6ufqhxann".to_string(), symbol: "BABYDOGE".to_string() },
                TrackedToken { mint: "vQoYWru2pbUdcVkUrRH74ktQDJgVjRcDvsoDbUzM5n9".to_string(), symbol: "REKT".to_string() },
                TrackedToken { mint: "5eyib4qghYGHNh7VvxSFGYLFJSanjq9hug9fR52kksnm".to_string(), symbol: "PSG".to_string() },
                TrackedToken { mint: "Bi11Je4MH3PpyCNiuTAepRbsA5U6DK3DJy79ZFthddX".to_string(), symbol: "BILL".to_string() },
                TrackedToken { mint: "2uxaYT1fVrp6Fg2BrxQcyKSW91hefM6dG9krpbeDiirT".to_string(), symbol: "REUSD".to_string() },
                TrackedToken { mint: "FRmH6iRkMr33DLG6zVLR7EM4LojBFAuq6NtFzG6ondo".to_string(), symbol: "MSFTON".to_string() },
                TrackedToken { mint: "4CxjZyMNoF1p6eXHXHv1TWpS2X47CuUmLwtGe18spump".to_string(), symbol: "MIHAO".to_string() },
                TrackedToken { mint: "6xHEyem9hmkGtVq6XGCiQUGpPsHBaoYuYdFNZa5ondo".to_string(), symbol: "CRCLON".to_string() },
                TrackedToken { mint: "JuprjznTrTSp2UFa3ZBUFgwdAmtZCq4MQCwysN55USD".to_string(), symbol: "JUPUSD".to_string() },
                TrackedToken { mint: "Dz8VutERqbHR2aFL5A3s1Ky4dG1unJT1jUFXXPaY9ytX".to_string(), symbol: "WOO".to_string() },
                TrackedToken { mint: "PAYmo6moDF3Ro3X6bU2jwe2UdBnBhv8YjLgL1j4DxGu".to_string(), symbol: "PAYAI".to_string() },
                TrackedToken { mint: "7JA5eZdCzztSfQbJvS8aVVxMFfd81Rs9VvwnocV1mKHu".to_string(), symbol: "GEOD".to_string() },
                TrackedToken { mint: "hWfiw4mcxT8rnNFkk6fsCQSxoxgZ9yVhB6tyeVcondo".to_string(), symbol: "GLDON".to_string() },
                TrackedToken { mint: "1eLZPRsn8bAKmoxsqDMH9Q2m2k7GMNp6RLSQGm8ondo".to_string(), symbol: "ASMLON".to_string() },
                TrackedToken { mint: "14W1itEkV7k1W819mLSknFTaMmkCtPokbF2tRkPUondo".to_string(), symbol: "TQQQON".to_string() },
                TrackedToken { mint: "5u6KDiNJXxX4rGMfYT4BApZQC5CuDNrG6MHkwp1ondo".to_string(), symbol: "COINON".to_string() },
                TrackedToken { mint: "7tgKziACteG26VjV5xKufojKxwTgCFyTwmWUmz5ondo".to_string(), symbol: "CVXON".to_string() },
                TrackedToken { mint: "DuEy8wWrzCUun5ZbbG9hkVqXqqicpTQw8gB7nEAzpCHQ".to_string(), symbol: "FLUID".to_string() },
                TrackedToken { mint: "CWZ6BsdnjkDVTGkmL6bGbJXXig6ceef12KvyGQW14cMt".to_string(), symbol: "ANTFUN".to_string() },
                TrackedToken { mint: "STARqri3xhy6Pyv1EpgdjkPDVcs9FncKcaEP6527krs".to_string(), symbol: "STAR".to_string() },
                TrackedToken { mint: "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So".to_string(), symbol: "MSOL".to_string() },
                TrackedToken { mint: "SonicxvLud67EceaEzCLRnMTBqzYUUYNr93DBkBdDES".to_string(), symbol: "SONIC".to_string() },
                TrackedToken { mint: "FUW4poh6s7uychceF8u1mo7NS65vzjX5vmS7Yi7GYQnz".to_string(), symbol: "ACN".to_string() },
                TrackedToken { mint: "inxKXw9V2NDZE7hDijzpJaKKUb97NEPJDTCEEiYg4yY".to_string(), symbol: "INX".to_string() },
                TrackedToken { mint: "DJ7vji2BU7RjNgktPAKN4L42CiXTFHEt4Eeeyr5FiTmy".to_string(), symbol: "ASP".to_string() },
                TrackedToken { mint: "Ce2gx9KGXJ6C9Mp5b5x1sn9Mg87JwEbrQby4Zqo3pump".to_string(), symbol: "NEET".to_string() },
                TrackedToken { mint: "EchesyfXePKdLtoiZSL8pBe8Myagyy8ZRqsACNCFGnvp".to_string(), symbol: "FIDA".to_string() },
                TrackedToken { mint: "bdh3njeo19d2TBLAKTGvCWdSoArfVw8uZBAJHY4ondo".to_string(), symbol: "HIMSON".to_string() },
                TrackedToken { mint: "HAYQtfJEQ9DbDbaHEhxfGsWbSZ3ywthdsVB3PuB72DYe".to_string(), symbol: "APXUSD".to_string() },
                TrackedToken { mint: "8WbNQtY7QmXMVKJFTSqFudierVZZtbuoyeepZEqJ1B2w".to_string(), symbol: "CITY".to_string() },
                TrackedToken { mint: "WETZjtprkDMCcUxPi9PfWnowMRZkiGGHDb9rABuRZ2U".to_string(), symbol: "WET".to_string() },
                TrackedToken { mint: "oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp".to_string(), symbol: "ORE".to_string() },
                TrackedToken { mint: "Bn9oW3GVKQdgNJG5w5P53dbuZfUtAehwFAniZYry68T8".to_string(), symbol: "QTC".to_string() },
                TrackedToken { mint: "AFJtnuqGMaj5jAo6Pwxo28r1f7XAXXTSA8q3rG3q8b4A".to_string(), symbol: "CARV".to_string() },
                TrackedToken { mint: "cJpUMp5R7rZ6fGeLHbHhrRuJzK9mkyKDjZqNpT3ondo".to_string(), symbol: "INTCON".to_string() },
                TrackedToken { mint: "C9J9vZ8N79GzzxFoRkPWCkGtMKU8akg4FhUk4r9ondo".to_string(), symbol: "IEFAON".to_string() },
                TrackedToken { mint: "HE6bLNs16qgQo2jVSMXDSCnxQahjdVrww7qQPrHrZtEw".to_string(), symbol: "JUV".to_string() },
                TrackedToken { mint: "HuAXPyDWDaMYFKuwQHpqL1oPnj93zdzWmtvFGzCeCUa7".to_string(), symbol: "MASK".to_string() },
                TrackedToken { mint: "MYXqkDYbzr7vjXAz2BapR4AiYRXzoikGirrLoRzondo".to_string(), symbol: "BMNRON".to_string() },
                TrackedToken { mint: "FovBwhoV5KQjZCdhoM6jgXYwXLX3F8vgAfvmLH7ondo".to_string(), symbol: "MRVLON".to_string() },
                TrackedToken { mint: "3orqhCKM5admbcHkHQhRAEKbXhUT5VPgsQqz7fBa6QdF".to_string(), symbol: "FBTC".to_string() },
                TrackedToken { mint: "BNso1VUJnh4zcfpZa6986Ea66P6TCp59hvtNJ8b1X85".to_string(), symbol: "BNSOL".to_string() },
                TrackedToken { mint: "HVbpJAQGNpkgBaYBZQBR1t7yFdvaYVp2vCQQfKKEN4tM".to_string(), symbol: "USDP".to_string() },
                TrackedToken { mint: "RoamA1USA8xjvpTJZ6RvvxyDRzNh6GCA1zVGKSiMVkn".to_string(), symbol: "ROAM".to_string() },
                TrackedToken { mint: "EDzhmkPa5gToYGowjH66xycCRgPmfWNzfqpXdCzsYb8M".to_string(), symbol: "GAL".to_string() },
                TrackedToken { mint: "5YMkXAYccHSGnHn9nob9xEvv6Pvka9DZWH7nTbotTu9E".to_string(), symbol: "HYUSD".to_string() },
                TrackedToken { mint: "8f62NyJGo7He5uWeveTA2JJQf4xzf8aqxkmzxRQ3mxfU".to_string(), symbol: "FIGHT".to_string() },
                TrackedToken { mint: "H5qGPniSX2uCNtAnxr7RpdfFAZcGkr6dknjgpa1AKHe1".to_string(), symbol: "ACM".to_string() },
                TrackedToken { mint: "HQMYCZTDq9g3oZejDRUeQsFtLKgyfvBpD3yHaTnain3L".to_string(), symbol: "EUSD".to_string() },
                TrackedToken { mint: "y6kSRF4i9tfMMjZziPHtQE1PeUS6bWEHTzZMFgXondo".to_string(), symbol: "STRCON".to_string() },
                TrackedToken { mint: "69RX85eQoEsnZvXGmLNjYcWgVkp9r2JjahVm99KbJETU".to_string(), symbol: "HOLO".to_string() },
                TrackedToken { mint: "Eg2ymQ2aQqjMcibnmTt8erC6Tvk9PVpJZCxvVPJz2agu".to_string(), symbol: "PUMPCADE".to_string() },
                TrackedToken { mint: "AyrQpt5xsVYiN4BqgZdd2tZJAWswT9yLUZmP1jKqpump".to_string(), symbol: "JOBCOIN".to_string() },
                TrackedToken { mint: "HgBRWfYxEfvPhtqkaeymCQtHCrKE46qQ43pKe8HCpump".to_string(), symbol: "BERT".to_string() },
                TrackedToken { mint: "BtjXf3LA7Z3jxAcQkKUqKYpLFS5B7GuB7pfjzeas1tdB".to_string(), symbol: "OG".to_string() },
                TrackedToken { mint: "PnjETBCLC318DRejo9cMQKAmET9PvW8AEFGWMNtondo".to_string(), symbol: "PDDON".to_string() },
                TrackedToken { mint: "euro5sNHrZC2wu2RoLvy6xxoVMc3Qnd1vjxoWf4MftA".to_string(), symbol: "EUROP".to_string() },
                TrackedToken { mint: "3joMReCCSESngJEpFLoKR2dNcChjSRCDtybQet5uSpse".to_string(), symbol: "CAT".to_string() },
                TrackedToken { mint: "8SMMso8Muv8d6i4WmMDthKt6TN1ysN6937sx3DKLXZqB".to_string(), symbol: "RHEA".to_string() },
                TrackedToken { mint: "D6xWgRCSHoMEB5fqPwk3p6Stxirn5ytm2WwboSTTx4oE".to_string(), symbol: "PYBOBO".to_string() },
                TrackedToken { mint: "1zJX5gRnjLgmTpq5sVwkq69mNDQkCemqoasyjaPW6jm".to_string(), symbol: "KLED".to_string() },
                TrackedToken { mint: "ErSZNwsamWjR9Hj2o18GLwVyTkooyGbdptjMjuv72X8A".to_string(), symbol: "BRISE".to_string() },
                TrackedToken { mint: "dog1viwbb2vWDpER5FrJ4YFG6gq6XuyFohUe9TXN65u".to_string(), symbol: "DOG".to_string() },
                TrackedToken { mint: "XsXcJ6GZ9kVnjqGsjBnktRcuwMBmvKWh8S93RefZ1rF".to_string(), symbol: "AMDX".to_string() },
                TrackedToken { mint: "5aLhp9VnUEKcsdtkfsf2DUgpJfomx7GmYVny24dHUZoB".to_string(), symbol: "XAUM".to_string() },
                TrackedToken { mint: "4nV5gNwwP68zUDat26ySChREqVaQaLudfJBkSgEzpump".to_string(), symbol: "BOBO".to_string() },
            ],
            rsi_period: 14,
            rsi_oversold: 25.0,
            rsi_exit: 55.0,
            price_sample_interval_secs: 60,
            stop_loss_pct: -8.0,
            take_profit_pct: 15.0,
            max_hold_mins: 120,
            poll_interval_secs: 30,
            is_paper_trading: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeanReversionPosition {
    pub token_address: String,
    pub symbol: String,
    pub entry_price_sol: f64,
    pub entry_amount_sol: f64,
    pub tokens_held: f64,
    pub entry_time: DateTime<Utc>,
    pub entry_rsi: f64,
    pub current_price_sol: f64,
    pub current_rsi: f64,
    pub current_multiplier: f64,
    pub peak_multiplier: f64,
}

#[derive(Debug, Clone)]
struct PricePoint {
    timestamp: DateTime<Utc>,
    price: f64,
}

pub struct MeanReversionEngine {
    settings: MeanReversionSettings,
    executor: Option<OnChainExecutor>,
    positions: Arc<Mutex<HashMap<String, MeanReversionPosition>>>,
    entered_tokens: Arc<Mutex<HashSet<String>>>,
    portfolio: Arc<Mutex<PortfolioState>>,
    http_client: reqwest::Client,
    price_history: Arc<Mutex<HashMap<String, VecDeque<PricePoint>>>>,
}

impl MeanReversionEngine {
    pub fn new(settings: MeanReversionSettings, executor: Option<OnChainExecutor>) -> Self {
        let portfolio = PortfolioState {
            initial_budget_sol: settings.total_budget_sol,
            available_sol: settings.total_budget_sol,
            invested_sol: 0.0,
            realized_pnl_sol: 0.0,
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
        };

        Self {
            settings,
            executor,
            positions: Arc::new(Mutex::new(HashMap::new())),
            entered_tokens: Arc::new(Mutex::new(HashSet::new())),
            portfolio: Arc::new(Mutex::new(portfolio)),
            http_client: reqwest::Client::builder()
                .user_agent("TradingAgents-rs/0.1")
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            price_history: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get_portfolio_state(&self) -> PortfolioState {
        self.portfolio.lock().unwrap().clone()
    }

    pub fn restore_state(&self, portfolio: PortfolioState, positions: HashMap<String, MeanReversionPosition>) {
        let mut port = self.portfolio.lock().unwrap();
        *port = portfolio;
        let mut pos = self.positions.lock().unwrap();
        let mut entered = self.entered_tokens.lock().unwrap();
        for p in positions.values() {
            entered.insert(p.token_address.clone());
        }
        *pos = positions;
    }

    pub fn save_to_ledger(&self, ledger: &super::ledger::Ledger) {
        let port = self.portfolio.lock().unwrap();
        let positions = self.positions.lock().unwrap();
        if let Err(e) = ledger.save_generic_state("mean_reversion", &*port, &*positions) {
            warn!("Failed to save mean_reversion ledger: {}", e);
        }
    }

    async fn fetch_price(&self, token_mint: &str) -> Option<f64> {
        let url = format!("https://api.dexscreener.com/tokens/v1/solana/{}", token_mint);
        let resp = self.http_client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        #[derive(Deserialize)]
        struct Pair {
            #[serde(rename = "priceNative")]
            price_native: Option<String>,
            liquidity: Option<Liq>,
        }
        #[derive(Deserialize)]
        struct Liq {
            usd: Option<f64>,
        }

        let pairs: Vec<Pair> = resp.json().await.ok()?;
        let best = pairs.iter()
            .filter(|p| p.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0) > 10_000.0)
            .filter_map(|p| p.price_native.as_ref()?.parse::<f64>().ok())
            .next()?;

        Some(best)
    }

    fn record_price(&self, mint: &str, price: f64) {
        let mut history = self.price_history.lock().unwrap();
        let entries = history.entry(mint.to_string()).or_insert_with(VecDeque::new);
        entries.push_back(PricePoint {
            timestamp: Utc::now(),
            price,
        });
        let max_entries = self.settings.rsi_period * 3 + 10;
        while entries.len() > max_entries {
            entries.pop_front();
        }
    }

    fn calculate_rsi(&self, mint: &str) -> Option<f64> {
        let history = self.price_history.lock().unwrap();
        let entries = history.get(mint)?;

        // Require at least 2 full periods of data (e.g. 28 samples) to avoid startup noise
        let min_samples = self.settings.rsi_period * 2;
        if entries.len() < min_samples {
            return None;
        }

        let prices: Vec<f64> = entries.iter().map(|p| p.price).collect();
        let len = prices.len();
        let start = len.saturating_sub(self.settings.rsi_period + 1);
        let window = &prices[start..];

        let mut gains = 0.0_f64;
        let mut losses = 0.0_f64;

        for i in 1..window.len() {
            let change = window[i] - window[i - 1];
            if change > 0.0 {
                gains += change;
            } else {
                losses += change.abs();
            }
        }

        let period = self.settings.rsi_period as f64;
        let avg_gain = gains / period;
        let avg_loss = losses / period;

        // If price is completely flat or has zero price action, RSI is neutral 50, not oversold 0!
        if avg_gain == 0.0 && avg_loss == 0.0 {
            return Some(50.0);
        }

        if avg_loss == 0.0 {
            return Some(100.0);
        }

        // If gains are zero but total loss is microscopic (< 0.5% total drop across window), return neutral 50 to avoid startup false triggers
        let total_price = window.first().cloned().unwrap_or(1.0);
        if avg_gain == 0.0 && (losses / total_price) < 0.015 {
            return Some(50.0);
        }

        let rs = avg_gain / avg_loss;
        let rsi = 100.0 - (100.0 / (1.0 + rs));

        Some(rsi)
    }

    pub async fn run_scan_cycle(&self) -> Result<usize> {
        let mut entries = 0;

        for token in &self.settings.tracked_tokens {
            let price = match self.fetch_price(&token.mint).await {
                Some(p) => p,
                None => continue,
            };

            self.record_price(&token.mint, price);

            let rsi = match self.calculate_rsi(&token.mint) {
                Some(r) => r,
                None => continue,
            };

            // Update existing position
            {
                let mut positions = self.positions.lock().unwrap();
                if let Some(pos) = positions.get_mut(&token.mint) {
                    pos.current_price_sol = price;
                    pos.current_rsi = rsi;
                    pos.current_multiplier = price / pos.entry_price_sol;
                    if pos.current_multiplier > pos.peak_multiplier {
                        pos.peak_multiplier = pos.current_multiplier;
                    }
                    continue;
                }
            }

            // Check for entry signal
            if rsi > self.settings.rsi_oversold {
                continue;
            }

            {
                let entered = self.entered_tokens.lock().unwrap();
                if entered.contains(&token.mint) {
                    continue;
                }
            }

            {
                let positions = self.positions.lock().unwrap();
                if positions.len() >= self.settings.max_concurrent_positions {
                    break;
                }
            }

            {
                let port = self.portfolio.lock().unwrap();
                if port.available_sol < self.settings.buy_amount_sol + self.settings.jito_tip_sol {
                    break;
                }
            }

            let tokens = self.settings.buy_amount_sol / price;

            if !self.settings.is_paper_trading {
                if let Some(ref executor) = self.executor {
                    match executor
                        .execute_jupiter_swap_jito(
                            &SOL_MINT,
                            &token.mint,
                            (self.settings.buy_amount_sol * 1_000_000_000.0) as u64,
                            self.settings.jito_tip_sol,
                        500,
                        )
                        .await
                    {
                        Ok(result) => info!("[MEAN_REV] On-chain buy: {}", result.bundle_id),
                        Err(e) => {
                            warn!("[MEAN_REV] Buy failed for {}: {}", token.symbol, e);
                            continue;
                        }
                    }
                }
            }

            let mode = if self.settings.is_paper_trading { "PAPER" } else { "LIVE" };
            println!(
                "  {} [MEAN REVERSION {}] {} | RSI: {:.1} (oversold < {:.0}) | price: {:.9} SOL | cost: {:.4} SOL",
                "📉".green(),
                mode,
                token.symbol.bold().cyan(),
                rsi,
                self.settings.rsi_oversold,
                price,
                self.settings.buy_amount_sol,
            );

            let position = MeanReversionPosition {
                token_address: token.mint.clone(),
                symbol: token.symbol.clone(),
                entry_price_sol: price,
                entry_amount_sol: self.settings.buy_amount_sol,
                tokens_held: tokens,
                entry_time: Utc::now(),
                entry_rsi: rsi,
                current_price_sol: price,
                current_rsi: rsi,
                current_multiplier: 1.0,
                peak_multiplier: 1.0,
            };

            {
                let mut port = self.portfolio.lock().unwrap();
                port.available_sol -= self.settings.buy_amount_sol + self.settings.jito_tip_sol;
                port.invested_sol += self.settings.buy_amount_sol;
                port.total_trades += 1;
            }
            {
                let mut positions = self.positions.lock().unwrap();
                positions.insert(token.mint.clone(), position);
            }
            {
                let mut entered = self.entered_tokens.lock().unwrap();
                entered.insert(token.mint.clone());
            }

            entries += 1;

            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        self.check_exit_signals().await?;

        Ok(entries)
    }

    async fn check_exit_signals(&self) -> Result<()> {
        let mut to_close: Vec<(String, String)> = Vec::new();

        {
            let positions = self.positions.lock().unwrap();
            for (mint, pos) in positions.iter() {
                let pnl_pct = (pos.current_multiplier - 1.0) * 100.0;

                // Compute exact net PnL after all fees (DEX LP fee 0.30% entry + 0.30% exit + slippage ~0.20% + Jito tip & tx fees)
                let round_trip_fee_sol = (self.settings.jito_tip_sol * 2.0) + 0.000030 + (pos.entry_amount_sol * 0.0080);
                let current_sol_value = pos.entry_amount_sol * pos.current_multiplier;
                let net_pnl_sol = current_sol_value - pos.entry_amount_sol - round_trip_fee_sol;
                let net_pnl_pct = (net_pnl_sol / pos.entry_amount_sol) * 100.0;

                if pnl_pct <= self.settings.stop_loss_pct {
                    to_close.push((mint.clone(), format!("STOP_LOSS (Net: {:.2}%)", net_pnl_pct)));
                    continue;
                }

                if net_pnl_pct >= self.settings.take_profit_pct {
                    to_close.push((mint.clone(), format!("TAKE_PROFIT Net +{:.1}%", net_pnl_pct)));
                    continue;
                }

                // Only exit on RSI overbought if the trade is strictly NET positive after all fees
                if pos.current_rsi >= self.settings.rsi_exit && net_pnl_sol > 0.0 {
                    to_close.push((mint.clone(), format!("RSI_PROFIT Net +{:.2}% (+{:.6} SOL) (RSI {:.1})", net_pnl_pct, net_pnl_sol, pos.current_rsi)));
                    continue;
                }

                let hold_mins = (Utc::now() - pos.entry_time).num_minutes() as u64;
                if self.settings.max_hold_mins > 0 && hold_mins >= self.settings.max_hold_mins {
                    to_close.push((mint.clone(), format!("MAX_HOLD {}m", hold_mins)));
                }
            }
        }

        for (mint, reason) in to_close {
            self.close_position(&mint, &reason).await;
        }

        Ok(())
    }

    async fn close_position(&self, token_addr: &str, reason: &str) {
        let pos = {
            let mut positions = self.positions.lock().unwrap();
            match positions.remove(token_addr) {
                Some(p) => p,
                None => return,
            }
        };

        // Allow re-entry after cool-down
        {
            let mut entered = self.entered_tokens.lock().unwrap();
            entered.remove(token_addr);
        }

        let proceeds = pos.tokens_held * pos.current_price_sol;
        let pnl = proceeds - pos.entry_amount_sol;

        if !self.settings.is_paper_trading {
            if let Some(ref executor) = self.executor {
                match executor
                    .execute_jupiter_swap_jito(
                        token_addr,
                        &SOL_MINT,
                        0, // 0 = automatic 100% full ATA balance on-chain
                        self.settings.jito_tip_sol,
                        500,
                    )
                    .await
                {
                    Ok(res) => {
                        info!("[MEAN_REV] On-chain sell confirmed: {}", res.bundle_id);
                    }
                    Err(e) => {
                        error!("[MEAN_REV] On-chain sell failed for {}: {}", pos.symbol, e);
                    }
                }
            }
        }

        let pnl_display = if pnl >= 0.0 { format!("+{:.4}", pnl).green() } else { format!("{:.4}", pnl).red() };
        println!(
            "  {} [MEAN_REV EXIT] {} | {} | RSI: {:.1}->{:.1} | {:.2}x | PnL: {} SOL",
            if pnl >= 0.0 { "✅" } else { "🛑" },
            pos.symbol.bold(), reason.yellow(),
            pos.entry_rsi, pos.current_rsi,
            pos.current_multiplier, pnl_display,
        );

        let mut port = self.portfolio.lock().unwrap();
        port.available_sol += proceeds;
        port.invested_sol -= pos.entry_amount_sol;
        port.realized_pnl_sol += pnl;
        if pnl >= 0.0 { port.winning_trades += 1; } else { port.losing_trades += 1; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rsi_calculation() {
        let settings = MeanReversionSettings {
            rsi_period: 5,
            ..Default::default()
        };
        let engine = MeanReversionEngine::new(settings, None);

        // Record ascending prices: RSI should be high (need 2 * period = 10 samples)
        let prices = vec![1.0, 1.05, 1.1, 1.15, 1.2, 1.25, 1.3, 1.35, 1.4, 1.45, 1.5, 1.55, 1.6];
        for p in &prices {
            engine.record_price("test_token", *p);
        }

        let rsi = engine.calculate_rsi("test_token").unwrap();
        assert!(rsi > 80.0, "RSI should be high for ascending prices, got {}", rsi);
    }

    #[test]
    fn test_rsi_oversold() {
        let settings = MeanReversionSettings {
            rsi_period: 5,
            ..Default::default()
        };
        let engine = MeanReversionEngine::new(settings, None);

        // Record descending prices: RSI should be low (need 2 * period = 10 samples)
        let prices = vec![2.0, 1.95, 1.9, 1.85, 1.8, 1.75, 1.7, 1.65, 1.6, 1.55, 1.5, 1.45, 1.4];
        for p in &prices {
            engine.record_price("test_token", *p);
        }

        let rsi = engine.calculate_rsi("test_token").unwrap();
        assert!(rsi < 20.0, "RSI should be low for descending prices, got {}", rsi);
    }

    #[test]
    fn test_mean_reversion_defaults() {
        let s = MeanReversionSettings::default();
        assert_eq!(s.rsi_period, 14);
        assert_eq!(s.rsi_oversold, 25.0);
        assert_eq!(s.tracked_tokens.len(), 248);
    }
}
