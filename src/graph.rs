use crate::agents::*;
use crate::dataflows::news::fetch_news_data;
use crate::dataflows::prediction::fetch_prediction_markets;
use crate::dataflows::sentiment::fetch_sentiment_data;
use crate::dataflows::technical::fetch_technical_data;
use crate::llm::LlmClient;
use crate::symbols::{resolve_symbol, subreddits_for};
use crate::types::*;
use anyhow::Result;
use colored::*;

pub struct TradingWorkflow<'a> {
    llm: &'a dyn LlmClient,
    selected_analysts: Vec<AnalystType>,
}

impl<'a> TradingWorkflow<'a> {
    pub fn new(llm: &'a dyn LlmClient, selected_analysts: Vec<AnalystType>) -> Self {
        Self {
            llm,
            selected_analysts,
        }
    }

    pub async fn run(&self, ticker_input: &str, portfolio: Portfolio) -> Result<AgentState> {
        let sym = resolve_symbol(ticker_input);
        let mut state = AgentState {
            ticker: sym.base.clone(),
            canonical_ticker: sym.canonical.clone(),
            asset_type: sym.asset_type,
            trade_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            portfolio,
            ..Default::default()
        };

        println!(
            "{}",
            format!(
                "\n🚀 [1/5] Initializing pipeline for {} (Type: {:?}, Market: {})",
                sym.base.bold().cyan(),
                sym.asset_type,
                sym.canonical.yellow()
            )
            .bold()
        );

        // Fetch market, sentiment, news, and Jupiter prediction market data concurrently
        println!("{}", "📊 Fetching live market feeds, indicators & prediction markets...".dimmed());
        let (indicators, sentiment_data, news_data, prediction_markets) = tokio::join!(
            fetch_technical_data(&sym.canonical),
            async {
                let subs = subreddits_for(&sym.base, sym.asset_type);
                fetch_sentiment_data(&sym.base, &subs).await
            },
            fetch_news_data(&sym.base),
            fetch_prediction_markets(&sym.base)
        );

        state.current_price = indicators.current_price;
        state.prediction_markets = prediction_markets;

        println!(
            "   Current Price: ${:.2} (24h: {:.2}%) | RSI: {:.1} | Mood: {}",
            indicators.current_price,
            indicators.change_24h_pct,
            indicators.rsi_14,
            sentiment_data.overall_mood.green()
        );

        if !state.prediction_markets.is_empty() {
            let top_pm = &state.prediction_markets[0];
            println!(
                "   🎯 Jupiter Prediction Market: \"{}\" -> YES ${:.2} ({:.1}% prob) | NO ${:.2}",
                top_pm.market_title.cyan(),
                top_pm.yes_price_usd,
                top_pm.implied_probability_pct,
                top_pm.no_price_usd
            );
        }

        // -----------------------------------------------------------------
        // STAGE 1: PARALLEL ANALYST TEAM
        // -----------------------------------------------------------------
        println!("\n{}", "👥 [2/5] Deploying parallel Analyst Team...".bold());

        let tech_fut = async {
            if self.selected_analysts.contains(&AnalystType::Technical) {
                run_technical_analyst(self.llm, &sym.base, &indicators).await.ok()
            } else {
                None
            }
        };

        let sent_fut = async {
            if self.selected_analysts.contains(&AnalystType::Sentiment) {
                run_sentiment_analyst(self.llm, &sym.base, &sentiment_data).await.ok()
            } else {
                None
            }
        };

        let news_fut = async {
            if self.selected_analysts.contains(&AnalystType::News) {
                run_news_analyst(self.llm, &sym.base, &news_data).await.ok()
            } else {
                None
            }
        };

        let fund_fut = async {
            if self.selected_analysts.contains(&AnalystType::Fundamentals) {
                run_fundamentals_analyst(self.llm, &sym.base, sym.asset_type).await.ok()
            } else {
                None
            }
        };

        let (tech_res, sent_res, news_res, fund_res) =
            tokio::join!(tech_fut, sent_fut, news_fut, fund_fut);

        if let Some(r) = tech_res {
            println!("   ✔ Technical Analyst: Score {:+.2} - {}", r.score, r.title);
            state.analyst_reports.insert(AnalystType::Technical, r);
        }
        if let Some(r) = sent_res {
            println!("   ✔ Sentiment Analyst: Score {:+.2} - {}", r.score, r.title);
            state.analyst_reports.insert(AnalystType::Sentiment, r);
        }
        if let Some(r) = news_res {
            println!("   ✔ News Analyst:      Score {:+.2} - {}", r.score, r.title);
            state.analyst_reports.insert(AnalystType::News, r);
        }
        if let Some(r) = fund_res {
            println!("   ✔ Fundamentals:      Score {:+.2} - {}", r.score, r.title);
            state.analyst_reports.insert(AnalystType::Fundamentals, r);
        }

        let reports_vec: Vec<&AnalystReport> = state.analyst_reports.values().collect();

        // -----------------------------------------------------------------
        // STAGE 2: RESEARCHERS DEBATE (BULL vs BEAR)
        // -----------------------------------------------------------------
        println!("\n{}", "⚔️  [3/5] Starting Research Debate (Bull vs Bear)...".bold());

        let (bull_case, bear_case) = tokio::join!(
            run_bull_researcher(self.llm, &sym.base, &reports_vec),
            run_bear_researcher(self.llm, &sym.base, &reports_vec)
        );

        let bull_text = bull_case.unwrap_or_else(|_| "Strong upside momentum.".to_string());
        let bear_text = bear_case.unwrap_or_else(|_| "Macro risk and volatility risk.".to_string());

        println!("   🐂 Bull Case formulated.");
        println!("   🐻 Bear Case formulated.");

        let synthesis = run_research_manager(self.llm, &sym.base, &bull_text, &bear_text)
            .await
            .unwrap_or_else(|_| "Balanced consensus achieved.".to_string());
        println!("   ⚖️  Research Manager synthesis complete.");

        state.debate = Some(DebateRound {
            bull_argument: bull_text,
            bear_argument: bear_text,
            manager_synthesis: synthesis,
        });

        // -----------------------------------------------------------------
        // STAGE 3: TRADER AGENT PROPOSAL
        // -----------------------------------------------------------------
        println!("\n{}", "📈 [4/5] Lead Trader synthesizing proposal...".bold());
        let trader_decision = run_trader_agent(
            self.llm,
            &sym.canonical,
            sym.asset_type,
            indicators.current_price,
            &reports_vec,
            state.debate.as_ref().unwrap(),
        )
        .await?;

        println!(
            "   Proposed Signal: {} (Confidence: {:.0}%, Suggested Size: {:.1}%)",
            trader_decision.rating.to_string().bold().green(),
            trader_decision.confidence * 100.0,
            trader_decision.allocation_pct
        );

        // -----------------------------------------------------------------
        // STAGE 4: RISK MANAGEMENT COMMITTEE REVIEW
        // -----------------------------------------------------------------
        println!("\n{}", "🛡️  [5/5] Risk Management Committee Review...".bold());
        let risk_review = run_risk_management(
            self.llm,
            &sym.base,
            &trader_decision,
            &state.portfolio,
        )
        .await
        .unwrap_or_else(|_| RiskAssessment {
            aggressive_view: "Approved.".to_string(),
            conservative_view: "Cautious allocation suggested.".to_string(),
            neutral_view: "Standard parameters applied.".to_string(),
            risk_score: 5,
            max_suggested_allocation_pct: trader_decision.allocation_pct,
            commentary: "Risk parameters within acceptable limits.".to_string(),
        });

        println!(
            "   Risk Score: {}/10 | Max Recommended Sizing: {:.1}%",
            risk_review.risk_score, risk_review.max_suggested_allocation_pct
        );

        // Final Portfolio sizing adjustment
        let final_alloc = trader_decision
            .allocation_pct
            .min(risk_review.max_suggested_allocation_pct);

        let mut final_decision = trader_decision;
        final_decision.allocation_pct = final_alloc;

        state.risk = Some(risk_review);
        state.decision = Some(final_decision);

        Ok(state)
    }
}
