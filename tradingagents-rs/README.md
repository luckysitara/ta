# 🦀 TradingAgents-rs: Multi-Agent Financial Trading Framework in Native Rust

A high-performance, native Rust reproduction of the **TradingAgents** multi-agent LLM financial trading framework, optimized for crypto assets, equity markets, and on-chain **Solana DEX & Prediction Market execution**.

---

## ⚡ Architecture Overview

TradingAgents-rs mirrors the full dynamics of a quantitative trading firm using an asynchronous state DAG powered by **Tokio**:

```
           [ Live Market Feeds, Sentiment & Jupiter Prediction Markets ]
                                         │
        ┌──────────────┬─────────────────┴─────────────┬──────────────┐
        ▼              ▼                               ▼              ▼
[Technical Analyst] [Sentiment Analyst]          [News Analyst] [Fundamentals/On-Chain]
        └──────────────┬───────────────────────────────┴──────────────┘
                       │ (Parallel join via tokio::join!)
                       ▼
       [ Research Debate: Bullish vs Bearish Researcher ]
                       │
                       ▼
            [ Head of Research Synthesis ]
                       │
                       ▼
            [ Lead Portfolio Trader ]
                       │ (Trade Proposal)
                       ▼
    [ Multi-Perspective Risk Management Committee ]
    (Aggressive, Conservative & Neutral Risk Officers)
                       │
                       ▼
             [ Final Consensus Report ]
                       │
         ┌─────────────┴────────────────────────┐
         ▼                                      ▼
  [ ⚡ Jupiter DEX Spot Swap ]    [ 🎯 Jupiter Prediction Market ]
  (Raydium / Orca Whirlpools)      (YES/NO Binary Contracts on Solana)
```

---

## 🚀 Key Features

* **True Async Concurrency**: Parallel analyst execution via `tokio::join!` with zero GIL latency.
* **🎯 Jupiter Prediction Market API (`api.jup.ag/prediction/v1`)**:
  * Live discovery of prediction events and binary YES/NO markets on Solana.
  * Price-as-probability ingestion ($0.65 = 65% probability) for macro and sentiment agents.
  * Automatic strategy formulation: sizes contract purchases in USDC, computes potential $1.00 payout and ROI.
  * Prepares `POST /orders` unsigned Solana transactions for instant wallet signing.
* **⚡ Native Solana Spot DEX Routing**:
  * Queries real-time DEX liquidity routes and price impact via **Jupiter Swap API v6**.
  * Outputs exact token swap routing across Raydium CLMM, Orca Whirlpools, and Meteora.
* **Multi-Provider LLM Engine**:
  * **Claude Code CLI** (`--provider claude-code`): Uses your active local session with zero API key required.
  * **Ollama** (`--provider ollama`): Local offline LLM execution.
  * **OpenAI** (`gpt-4o`), **Anthropic** (`claude-3-5-sonnet`), **Groq** (`llama-3.3-70b-versatile`).
  * **Mock LLM**: Built-in offline dry-run testing mode.

---

## 🛠️ Usage

### 1. Build and Run
```bash
cd tradingagents-rs
cargo build --release
```

### 2. Run Analysis on Solana (SOL)
```bash
# Analyzes SOL spot and discovers Jupiter Prediction Markets:
cargo run -- --ticker SOL

# Run using your local Claude Code session:
cargo run -- --ticker SOL --provider claude-code

# Run with custom cash sizing:
cargo run -- --ticker SOL --cash 25000
```

### 3. CLI Options
```
Options:
  -t, --ticker <TICKER>          Ticker symbol to analyze (default: SOL)
  -p, --provider <PROVIDER>      auto, claude-code, ollama, openai, groq, mock [default: auto]
  -m, --model <MODEL>            Model identifier
  -a, --analysts <ANALYSTS>      Comma-separated list of analysts [default: technical,sentiment,news,fundamentals]
      --solana-dex               Display Solana DEX spot swap route via Jupiter API [default: true]
      --prediction-markets       Discover & formulate Jupiter Prediction Market contracts [default: true]
      --cash <CASH>              Initial portfolio cash in USD [default: 10000.0]
  -h, --help                     Print help
```
