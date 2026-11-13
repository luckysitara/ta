use crate::dataflows::news::NewsData;
use crate::dataflows::sentiment::SentimentData;
use crate::dataflows::technical::TechnicalIndicators;
use crate::llm::{complete_json, LlmClient};
use crate::types::*;
use anyhow::Result;
use chrono::Utc;

// ==========================================
// 1. ANALYSTS
// ==========================================

pub async fn run_technical_analyst(
    llm: &dyn LlmClient,
    ticker: &str,
    indicators: &TechnicalIndicators,
) -> Result<AnalystReport> {
    let system = "You are an elite quantitative Technical Analyst at a top trading firm. \
Analyze the provided indicators (price, moving averages, RSI, MACD, Bollinger Bands) \
and generate a JSON report with fields: analyst ('technical'), title, summary, key_points (array of strings), score (-1.0 to 1.0). Return ONLY valid JSON.";

    let prompt = format!(
        "Asset: {}\nCurrent Price: ${:.2}\n24h Change: {:.2}%\n24h High: ${:.2} | Low: ${:.2}\n\
24h Volume: ${:.0}\nSMA-20: ${:.2} | SMA-50: ${:.2}\nRSI-14: {:.1}\n\
MACD: {:.3} | Signal: {:.3} | Histogram: {:.3}\nUpper BB: ${:.2} | Lower BB: ${:.2}\n\
Provide your technical evaluation.",
        ticker,
        indicators.current_price,
        indicators.change_24h_pct,
        indicators.high_24h,
        indicators.low_24h,
        indicators.volume_24h,
        indicators.sma_20,
        indicators.sma_50,
        indicators.rsi_14,
        indicators.macd,
        indicators.macd_signal,
        indicators.macd_histogram,
        indicators.upper_bollinger,
        indicators.lower_bollinger
    );

    complete_json::<AnalystReport>(llm, Some(system), &prompt).await
}

pub async fn run_sentiment_analyst(
    llm: &dyn LlmClient,
    ticker: &str,
    sentiment: &SentimentData,
) -> Result<AnalystReport> {
    let system = "You are a Sentiment & Crowd Psychology Analyst specializing in financial and crypto markets. \
Assess community discussions, StockTwits streams, and forum mood. \
Generate a JSON report with fields: analyst ('sentiment'), title, summary, key_points (array of strings), score (-1.0 to 1.0). Return ONLY valid JSON.";

    let posts_formatted = sentiment
        .reddit_posts
        .iter()
        .map(|p| format!("- {}", p))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "Asset: {}\nOverall Mood: {}\nStockTwits Bullish %: {:.1}%\nRecent Posts:\n{}\n\
Provide your sentiment analysis report.",
        ticker, sentiment.overall_mood, sentiment.bullish_pct, posts_formatted
    );

    complete_json::<AnalystReport>(llm, Some(system), &prompt).await
}

pub async fn run_news_analyst(
    llm: &dyn LlmClient,
    ticker: &str,
    news: &NewsData,
) -> Result<AnalystReport> {
    let system = "You are a Senior News & Macro Analyst. \
Evaluate recent headlines, macro trends, and narrative catalysts. \
Generate a JSON report with fields: analyst ('news'), title, summary, key_points (array of strings), score (-1.0 to 1.0). Return ONLY valid JSON.";

    let articles_formatted = news
        .articles
        .iter()
        .map(|a| format!("- [{}] {}: {}", a.source, a.title, a.snippet))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "Asset: {}\nMacro Backdrop: {}\nRecent Headlines:\n{}\n\
Provide your news and macro report.",
        ticker, news.macro_summary, articles_formatted
    );

    complete_json::<AnalystReport>(llm, Some(system), &prompt).await
}

pub async fn run_fundamentals_analyst(
    llm: &dyn LlmClient,
    ticker: &str,
    asset_type: AssetType,
) -> Result<AnalystReport> {
    let system = "You are a Fundamental & On-Chain Valuation Analyst. \
For crypto, evaluate tokenomics, ecosystem TVL, developer activity, and network revenue. \
For stocks, evaluate revenue growth, cash flow, and valuation multiples. \
Generate a JSON report with fields: analyst ('fundamentals'), title, summary, key_points (array of strings), score (-1.0 to 1.0). Return ONLY valid JSON.";

    let prompt = match asset_type {
        AssetType::Crypto => format!(
            "Evaluate fundamental on-chain health for {}: TVL growth, active daily wallets, protocol fees, ecosystem velocity, and validator decentralization.",
            ticker
        ),
        AssetType::Stock => format!(
            "Evaluate corporate fundamentals for {}: revenue trajectory, balance sheet health, operating margins, and competitive moat.",
            ticker
        ),
    };

    complete_json::<AnalystReport>(llm, Some(system), &prompt).await
}

// ==========================================
// 2. RESEARCHERS DEBATE (BULL vs BEAR)
// ==========================================

pub async fn run_bull_researcher(
    llm: &dyn LlmClient,
    ticker: &str,
    reports: &[&AnalystReport],
) -> Result<String> {
    let system = "You are the Bullish Researcher at a quantitative hedge fund. \
Your mandate is to advocate the strongest data-driven upside case for this asset based on the analyst findings. \
Highlight catalysts, technical breakout setup, and asymmetric reward.";

    let reports_text = format_reports_for_debate(reports);
    let prompt = format!(
        "Asset: {}\nAnalyst Reports:\n{}\nFormulate your concise, convincing BULL thesis.",
        ticker, reports_text
    );

    llm.complete(Some(system), &prompt).await
}

pub async fn run_bear_researcher(
    llm: &dyn LlmClient,
    ticker: &str,
    reports: &[&AnalystReport],
) -> Result<String> {
    let system = "You are the Bearish Researcher at a quantitative hedge fund. \
Your mandate is to critically stress-test the asset, identifying potential bull traps, liquidity risks, downside vulnerabilities, and macro headwinds.";

    let reports_text = format_reports_for_debate(reports);
    let prompt = format!(
        "Asset: {}\nAnalyst Reports:\n{}\nFormulate your concise, rigorous BEAR counter-argument.",
        ticker, reports_text
    );

    llm.complete(Some(system), &prompt).await
}

pub async fn run_research_manager(
    llm: &dyn LlmClient,
    ticker: &str,
    bull_case: &str,
    bear_case: &str,
) -> Result<String> {
    let system = "You are the Head of Research. \
Synthesize the debate between Bull and Bear researchers. Determine which thesis carries superior evidential weight and outline the balanced consensus.";

    let prompt = format!(
        "Asset: {}\nBull Case:\n{}\n\nBear Case:\n{}\n\nProvide your executive research synthesis and directional bias.",
        ticker, bull_case, bear_case
    );

    llm.complete(Some(system), &prompt).await
}

// ==========================================
// 3. TRADER AGENT
// ==========================================

pub async fn run_trader_agent(
    llm: &dyn LlmClient,
    ticker: &str,
    _asset_type: AssetType,
    current_price: f64,
    reports: &[&AnalystReport],
    debate: &DebateRound,
) -> Result<TradeDecision> {
    let system = "You are the Lead Portfolio Trader. \
Review the Analyst findings and Research debate to issue a precise trade recommendation. \
Output ONLY a JSON object with: \
ticker, asset_type ('stock' or 'crypto'), rating ('STRONG_BUY', 'BUY', 'HOLD', 'SELL', 'STRONG_SELL'), \
confidence (0.0 to 1.0), allocation_pct (suggested portfolio percentage 0 to 100), \
target_price (number), stop_loss (number), rationale (string). Return ONLY valid JSON.";

    let reports_text = format_reports_for_debate(reports);
    let prompt = format!(
        "Asset: {}\nCurrent Price: ${:.2}\n\nAnalyst Reports:\n{}\n\nResearch Debate:\nBull: {}\nBear: {}\nSynthesis: {}\n\nFormulate your final trade decision.",
        ticker, current_price, reports_text, debate.bull_argument, debate.bear_argument, debate.manager_synthesis
    );

    #[derive(serde::Deserialize)]
    struct RawDecision {
        ticker: String,
        asset_type: AssetType,
        rating: Rating,
        confidence: f32,
        allocation_pct: f32,
        target_price: Option<f64>,
        stop_loss: Option<f64>,
        rationale: String,
    }

    let raw: RawDecision = complete_json(llm, Some(system), &prompt).await?;

    Ok(TradeDecision {
        ticker: raw.ticker,
        asset_type: raw.asset_type,
        rating: raw.rating,
        confidence: raw.confidence,
        allocation_pct: raw.allocation_pct,
        target_price: raw.target_price,
        stop_loss: raw.stop_loss,
        rationale: raw.rationale,
        timestamp: Utc::now(),
    })
}

// ==========================================
// 4. RISK MANAGEMENT
// ==========================================

pub async fn run_risk_management(
    llm: &dyn LlmClient,
    ticker: &str,
    trader_decision: &TradeDecision,
    portfolio: &Portfolio,
) -> Result<RiskAssessment> {
    let system = "You are the Risk Management Committee (Aggressive, Conservative, and Neutral risk officers). \
Review the trader's proposal against volatility, portfolio concentration, and downside parameters. \
Generate a JSON object with: aggressive_view, conservative_view, neutral_view, risk_score (1 to 10), max_suggested_allocation_pct (number), commentary. Return ONLY valid JSON.";

    let prompt = format!(
        "Asset: {}\nProposed Rating: {}\nProposed Allocation: {:.1}%\nTarget Price: {:?} | Stop Loss: {:?}\n\
Portfolio Cash: ${:.2}\nRationale: {}\n\nPerform full multi-perspective risk assessment.",
        ticker,
        trader_decision.rating,
        trader_decision.allocation_pct,
        trader_decision.target_price,
        trader_decision.stop_loss,
        portfolio.cash_usd,
        trader_decision.rationale
    );

    complete_json::<RiskAssessment>(llm, Some(system), &prompt).await
}

fn format_reports_for_debate(reports: &[&AnalystReport]) -> String {
    reports
        .iter()
        .map(|r| {
            format!(
                "[{:?}] (Score: {:.2})\nTitle: {}\nSummary: {}\nKey Points:\n- {}",
                r.analyst,
                r.score,
                r.title,
                r.summary,
                r.key_points.join("\n- ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
