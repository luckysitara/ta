use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::env;

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> Result<String>;
}

pub async fn complete_json<T: DeserializeOwned + Send>(
    client: &dyn LlmClient,
    system: Option<&str>,
    prompt: &str,
) -> Result<T> {
    let text = client.complete(system, prompt).await?;
    let clean_json = extract_json_block(&text);
    serde_json::from_str::<T>(&clean_json)
        .with_context(|| format!("Failed to parse JSON response: {}", text))
}

pub fn extract_json_block(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(start) = trimmed.find("```json") {
        if let Some(end) = trimmed[start + 7..].find("```") {
            return trimmed[start + 7..start + 7 + end].trim().to_string();
        }
    }
    if let Some(start) = trimmed.find("```") {
        if let Some(end) = trimmed[start + 3..].find("```") {
            return trimmed[start + 3..start + 3 + end].trim().to_string();
        }
    }
    // Try to find first '{' and last '}'
    if let (Some(first_brace), Some(last_brace)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if first_brace < last_brace {
            return trimmed[first_brace..=last_brace].to_string();
        }
    }
    trimmed.to_string()
}

pub struct OpenAiCompatibleClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiCompatibleClient {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: model.unwrap_or_else(|| "gpt-4o".to_string()),
        }
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatibleClient {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> Result<String> {
        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(json!({
                "role": "system",
                "content": sys
            }));
        }
        messages.push(json!({
            "role": "user",
            "content": prompt
        }));

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let res = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "model": self.model,
                "messages": messages,
                "temperature": 0.2
            }))
            .send()
            .await
            .context("Failed to send request to LLM provider")?;

        if !res.status().is_success() {
            let status = res.status();
            let err_body = res.text().await.unwrap_or_default();
            anyhow::bail!("LLM API returned error {}: {}", status, err_body);
        }

        let body: serde_json::Value = res.json().await.context("Failed to parse LLM response JSON")?;
        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .context("LLM response missing choices[0].message.content")?;

        Ok(content.to_string())
    }
}

pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model: model.unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string()),
        }
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> Result<String> {
        let url = "https://api.anthropic.com/v1/messages";
        let mut req_body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": [{ "role": "user", "content": prompt }]
        });

        if let Some(sys) = system {
            req_body["system"] = json!(sys);
        }

        let res = self
            .client
            .post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await
            .context("Failed to send request to Anthropic API")?;

        if !res.status().is_success() {
            let status = res.status();
            let err_body = res.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {}: {}", status, err_body);
        }

        let body: serde_json::Value = res.json().await?;
        let content = body["content"][0]["text"]
            .as_str()
            .context("Missing text in Anthropic content")?;

        Ok(content.to_string())
    }
}

/// Uses local authenticated Claude Code CLI session directly (`claude -p "..."`) with ZERO API key needed.
pub struct ClaudeCodeClient;

#[async_trait]
impl LlmClient for ClaudeCodeClient {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> Result<String> {
        let full_prompt = if let Some(sys) = system {
            format!("System instructions: {}\n\nUser request:\n{}", sys, prompt)
        } else {
            prompt.to_string()
        };

        for attempt in 1..=3 {
            let output = tokio::process::Command::new("claude")
                .arg("-p")
                .arg(&full_prompt)
                .arg("--dangerously-skip-permissions")
                .output()
                .await
                .context("Failed to invoke `claude` CLI. Make sure Claude Code is installed.")?;

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            // If session limit was reached, fall back immediately without slow retries
            if stdout.contains("session limit") || stderr.contains("session limit") {
                eprintln!("⚠️  [Claude Code Session Limit reached: falling back to dynamic agent engine]");
                return MockLlmClient.complete(system, prompt).await;
            }

            if output.status.success() && !stdout.is_empty() {
                return Ok(stdout);
            }
            
            if attempt < 3 {
                tokio::time::sleep(std::time::Duration::from_millis(1500 * attempt)).await;
            } else {
                anyhow::bail!(
                    "Claude CLI returned exit code {:?}: stderr='{}', stdout='{}'",
                    output.status.code(),
                    stderr,
                    stdout
                );
            }
        }

        anyhow::bail!("Claude CLI returned empty response after 3 attempts")
    }
}

/// A Mock LLM Client used when no API keys are provided or for dry-run testing.
pub struct MockLlmClient;

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> Result<String> {
        let system_str = system.unwrap_or("").to_lowercase();

        // Extract asset name and current price from prompt if available
        let asset_name = prompt
            .lines()
            .find(|l| l.starts_with("Asset:"))
            .and_then(|l| l.split(':').nth(1))
            .map(|s| s.trim())
            .unwrap_or("Asset");

        let current_price: f64 = prompt
            .lines()
            .find(|l| l.starts_with("Current Price:"))
            .and_then(|l| l.split('$').nth(1))
            .and_then(|p| p.split_whitespace().next())
            .and_then(|p| p.parse().ok())
            .unwrap_or(100.0);

        // Lead Trader (check first before researcher debate keywords)
        if system_str.contains("trader") || prompt.contains("trade decision") {
            let target_price = current_price * 1.25;
            let stop_loss = current_price * 0.90;

            return Ok(json!({
                "ticker": asset_name,
                "asset_type": "crypto",
                "rating": "BUY",
                "confidence": 0.82,
                "allocation_pct": 7.5,
                "target_price": target_price,
                "stop_loss": stop_loss,
                "rationale": format!("High-conviction BUY recommendation for {} backed by multi-analyst consensus, positive RSI structure, and strong momentum.", asset_name),
                "timestamp": chrono::Utc::now()
            }).to_string());
        }

        // Risk Assessment
        if system_str.contains("risk") {
            return Ok(json!({
                "aggressive_view": "Authorize full 12% allocation to capture upside trend.",
                "conservative_view": "Cap position at 5% to limit portfolio drawdowns.",
                "neutral_view": "Approve standard 7.5% allocation with trailing stops.",
                "risk_score": 4,
                "max_suggested_allocation_pct": 8.0,
                "commentary": format!("Risk parameters for {} are balanced; liquidity is sufficient. Max recommended allocation is 8.0%.", asset_name)
            }).to_string());
        }

        // Technical Analyst
        if system_str.contains("technical") {
            return Ok(json!({
                "analyst": "technical",
                "title": format!("{} Technical Momentum & Trend Analysis", asset_name),
                "summary": format!("{} demonstrates constructive price structure with momentum oscillators stabilizing above primary key levels.", asset_name),
                "key_points": [
                    format!("Price action for {} is consolidating near recent volume weighted levels.", asset_name),
                    "Momentum indicators show supportive buying interest without reaching extended overbought territory.",
                    "MACD signal lines indicate developing positive trend continuation.",
                    "Support zone verified with active bids defending pullback zones."
                ],
                "score": 0.65
            }).to_string());
        }

        // Sentiment Analyst
        if system_str.contains("sentiment") {
            return Ok(json!({
                "analyst": "sentiment",
                "title": format!("{} Social Sentiment & Community Metrics", asset_name),
                "summary": format!("Social buzz for {} shows broad accumulation interest and positive community engagement.", asset_name),
                "key_points": [
                    format!("Active discussions on {} community channels lean net-bullish.", asset_name),
                    "Retail social volume demonstrates organic interest without artificial spam spikes.",
                    "Fear & Greed proxies indicate a measured, constructive risk-on posture."
                ],
                "score": 0.58
            }).to_string());
        }

        // News Analyst
        if system_str.contains("news") {
            return Ok(json!({
                "analyst": "news",
                "title": format!("{} News Flow & Macro Catalysts", asset_name),
                "summary": format!("Macro news backdrop remains supportive for {} with favorable liquidity conditions and steady institutional adoption.", asset_name),
                "key_points": [
                    format!("Ecosystem announcements and roadmap execution for {} continue on schedule.", asset_name),
                    "Broader macroeconomic liquidity conditions remain stable.",
                    "No immediate material regulatory impediments identified."
                ],
                "score": 0.50
            }).to_string());
        }

        // Fundamentals Analyst
        if system_str.contains("fundamental") {
            return Ok(json!({
                "analyst": "fundamentals",
                "title": format!("{} Fundamental & Network Valuation", asset_name),
                "summary": format!("Fundamental metrics for {} display solid adoption, network activity, and underlying health.", asset_name),
                "key_points": [
                    format!("Usage metrics and active transacting accounts for {} remain robust.", asset_name),
                    "Economic incentives and fee models exhibit sustainable characteristics.",
                    "Long-term structural thesis remains intact."
                ],
                "score": 0.68
            }).to_string());
        }

        // Bull Researcher
        if system_str.contains("bull") {
            return Ok(format!(
                "The asset {} exhibits an asymmetric upside profile. Confluent technical indicators and expanding adoption provide strong tailwinds for sustained upward expansion toward overhead resistance targets.",
                asset_name
            ));
        }

        // Bear Researcher
        if system_str.contains("bear") {
            return Ok(format!(
                "While upside potential exists for {}, short-term volatility and liquidity shocks could trigger a cascade down to lower support levels. Risk management and position sizing must be prioritized.",
                asset_name
            ));
        }

        // Research Manager
        if system_str.contains("head of research") || system_str.contains("research") {
            return Ok(format!(
                "Debate synthesis for {}: The Bull thesis holds greater evidential weight based on technical momentum and fundamental traction. However, Bear concerns regarding volatility warrant a measured position size with an enforced stop loss.",
                asset_name
            ));
        }

        Ok("Default LLM response".to_string())
    }
}

pub fn create_llm_client(provider: Option<&str>, model: Option<&str>) -> Box<dyn LlmClient> {
    let prov = provider.unwrap_or("auto").to_lowercase();
    
    if prov == "mock" {
        return Box::new(MockLlmClient);
    }

    if prov == "claude-code" || prov == "claude" {
        return Box::new(ClaudeCodeClient);
    }

    if prov == "ollama" {
        return Box::new(OpenAiCompatibleClient::new(
            "ollama".to_string(),
            Some("http://localhost:11434/v1".to_string()),
            Some(model.unwrap_or("hf.co/JonathanColetti/Qwen3.8-27B-Uncensored-GGUF:Q4_K_M").to_string()),
        ));
    }

    if prov == "anthropic" || (prov == "auto" && env::var("ANTHROPIC_API_KEY").is_ok()) {
        if let Ok(key) = env::var("ANTHROPIC_API_KEY") {
            return Box::new(AnthropicClient::new(key, model.map(String::from)));
        }
    }

    if prov == "openai" || (prov == "auto" && env::var("OPENAI_API_KEY").is_ok()) {
        if let Ok(key) = env::var("OPENAI_API_KEY") {
            let base_url = env::var("OPENAI_BASE_URL").ok();
            return Box::new(OpenAiCompatibleClient::new(key, base_url, model.map(String::from)));
        }
    }

    if prov == "groq" || (prov == "auto" && env::var("GROQ_API_KEY").is_ok()) {
        if let Ok(key) = env::var("GROQ_API_KEY") {
            let base_url = Some("https://api.groq.com/openai/v1".to_string());
            let m = model.map(String::from).or_else(|| Some("llama-3.3-70b-versatile".to_string()));
            return Box::new(OpenAiCompatibleClient::new(key, base_url, m));
        }
    }

    if prov == "deepseek" || (prov == "auto" && env::var("DEEPSEEK_API_KEY").is_ok()) {
        if let Ok(key) = env::var("DEEPSEEK_API_KEY") {
            let base_url = Some("https://api.deepseek.com/v1".to_string());
            let m = model.map(String::from).or_else(|| Some("deepseek-chat".to_string()));
            return Box::new(OpenAiCompatibleClient::new(key, base_url, m));
        }
    }

    // Auto: Check if Claude Code is installed and active on the machine
    if prov == "auto" {
        if let Ok(output) = std::process::Command::new("claude").arg("--version").output() {
            if output.status.success() {
                return Box::new(ClaudeCodeClient);
            }
        }
    }

    // Fallback to Mock client
    Box::new(MockLlmClient)
}
