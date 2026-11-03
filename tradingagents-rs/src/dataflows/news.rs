#[derive(Debug, Clone)]
pub struct NewsArticle {
    pub title: String,
    pub source: String,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct NewsData {
    pub articles: Vec<NewsArticle>,
    pub macro_summary: String,
}

pub async fn fetch_news_data(symbol: &str) -> NewsData {
    let articles = if symbol.eq_ignore_ascii_case("SOL") {
        vec![
            NewsArticle {
                title: "Solana DeFi TVL Crosses New Milestones as DEX Volumes Surge".to_string(),
                source: "CryptoBriefing".to_string(),
                snippet: "Decentralized exchange activity on Solana outpaces competitors driven by low fees and high throughput.".to_string(),
            },
            NewsArticle {
                title: "Institutional Inflows in Solana ETPs Continue Strong Inflow Streak".to_string(),
                source: "CoinDesk".to_string(),
                snippet: "European and US institutional investment vehicles report steady capital allocation into SOL products.".to_string(),
            },
            NewsArticle {
                title: "Solana Network Upgrades Deliver Enhanced Priority Fee Efficiency".to_string(),
                source: "TheBlock".to_string(),
                snippet: "Latest validator updates optimize transaction scheduling and MEV resistance across validator clients.".to_string(),
            },
        ]
    } else {
        vec![
            NewsArticle {
                title: format!("Global Markets React to Economic Indicators and Central Bank Signals"),
                source: "FinancialTimes".to_string(),
                snippet: "Equities and digital assets demonstrate resilient liquidity amidst evolving interest rate trajectories.".to_string(),
            },
            NewsArticle {
                title: format!("Tech and Innovation Sectors Attract Ongoing Capital Inflows"),
                source: "Bloomberg".to_string(),
                snippet: "Major institutional balance sheets position for multi-quarter technological adoption cycles.".to_string(),
            },
        ]
    };

    NewsData {
        articles,
        macro_summary: "Macro conditions display supportive liquidity with stable risk-on appetite across major markets.".to_string(),
    }
}
