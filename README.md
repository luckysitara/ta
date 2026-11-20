# 🦀 TradingAgents-rs: Multi-Agent Financial Trading & Solana High-Frequency Execution Engine

A high-performance, native Rust quantitative trading engine and multi-agent LLM financial research framework, optimized for crypto assets, equity markets, and live on-chain **Solana DEX & Memecoin Execution** with Jito MEV protection.

---

## ⚡ Architecture Overview

TradingAgents-rs pairs quantitative execution engines with an asynchronous multi-agent DAG powered by **Tokio**:

```
           [ Live Solana gRPC Stream / Yellowstone DEX Feeds / Jup Quote API ]
                                          │
         ┌──────────────┬─────────────────┴─────────────┬──────────────┐
         ▼              ▼                               ▼              ▼
 [Breakout Scalper] [Mean Reversion RSI]        [Route Arbitrage] [Strict Sniper]
         └──────────────┬───────────────────────────────┴──────────────┘
                        │
                        ▼
       [ Guaranteed Net Profit Lockbox & Trailing TP Clamp ]
                        │
                        ▼
       [ Jito MEV Bundle Execution / Zero-Slippage Routing ]
                        │
                        ▼
       [ Auto Rent Reclamation (SPL & Token-2022 Account Close) ]
```

---

## 🚀 Live On-Chain Trading Strategies (Tested & Working)

Below are the exact production-tested commands for running the live strategy daemons with strict risk management, guaranteed net profit lockboxes, and anti-rug filters.

### Prerequisites & Environment Setup
Ensure your Solana keypair is available and your RPC / gRPC credentials are configured:

```bash
# Build the optimized release binary
cargo build --release

# Set keypair and RPC environment variables (or specify via CLI flags)
export KEYPAIR_PATH="/home/tiktoor/ta/id.json"
export SOLANA_RPC_URL="https://graceful-oasis-a398.mainnet.rpcpool.com/95794a05-4057-49cb-87e4-42d3d9a46774"
export SOLANA_GRPC_ENDPOINT="https://graceful-oasis-a398.mainnet.rpcpool.com:443"
export SOLANA_GRPC_TOKEN="95794a05-4057-49cb-87e4-42d3d9a46774"
```

---

### 1. 📈 DEX Breakout Scalper (Guaranteed Profit Lockbox)
Scans Raydium, Meteora DLMM, and Orca Whirlpools for volume surges and multi-timeframe buy dominance.
* **Guaranteed Net Profit Lockbox**: Activates trailing take-profit at `+3.5%`, trails by `2.5%` from peak watermark, and firmly clamps the trailing exit floor to `>= +2.5%` gross to guarantee net positive profit over LP swap fees (1% buy + 1% sell) and Jito tips. A trade in profit never turns into a loss.
* **Safety Filters**: Enforces minimum `$40,000 USD` liquidity floor, requires 24h volume >= `$250,000 USD` and 5m volume >= `$10,000 USD`, 5m buy ratio >= `60%`, excludes `pumpswap`/`pumpfun` thin bonding curves, and blacklists predatory tokens like `OGTA`.

```bash
target/release/tradingagents-rs breakout \
  --onchain \
  --keypair-path /home/tiktoor/ta/id.json \
  --strategy multi \
  --budget 0.12 \
  --buy-amount 0.04 \
  --max-positions 3 \
  --jito-tip 0.00003 \
  --min-liquidity-usd 40000 \
  --min-volume-24h-usd 250000 \
  --min-volume-5m-usd 10000 \
  --min-buys-5m 25 \
  --min-buy-ratio-5m 0.60 \
  --min-buy-ratio-1h 0.55 \
  --min-graduation-liquidity-usd 40000 \
  --min-graduation-buy-ratio-5m 0.65 \
  --stop-loss-pct=-5.0 \
  --trailing-take-profit-activation-pct 3.5 \
  --trailing-take-profit-callback-pct 2.5 \
  --trailing-stop-pct=-8.0 \
  --break-even-mult 1.025 \
  --poll-interval-secs 5
```

---

### 2. 📉 Mean Reversion RSI Engine
Monitors established, highly-liquid Solana memecoins (`BONK`, `WIF`, `POPCAT`, `MEW`, `FARTCOIN`) for RSI-14 oversold washouts (`< 25`).
* **Strict Net Positive PnL Enforced**: Requires `net_pnl_sol > 0.0` after all fees before triggering profit exits.
* **Risk Control**: Hard stop-loss at `-8.0%`, 120-minute maximum hold timeout to release idle capital.

```bash
target/release/tradingagents-rs mean-rev \
  --onchain \
  --keypair-path /home/tiktoor/ta/id.json \
  --budget 0.12 \
  --buy-amount 0.04 \
  --max-positions 1 \
  --jito-tip 0.00003 \
  --rsi-period 14 \
  --rsi-oversold 25.0 \
  --rsi-exit 55.0 \
  --stop-loss-pct=-8.0 \
  --take-profit-pct 15.0 \
  --max-hold-mins 120 \
  --poll-interval-secs 20
```

---

### 3. ⚡ Jupiter Route Arbitrage Engine
Exploits instantaneous cross-DEX routing mispricings across Jupiter routes with atomic or fast leg execution.
* **Spread Threshold**: Captures price discrepancies exceeding `25 bps (0.25%)` across DEX routes (e.g. Raydium, Meteora, Orca, Deriverse, Scorch).

```bash
target/release/tradingagents-rs route-arb \
  --onchain \
  --keypair-path /home/tiktoor/ta/id.json \
  --trade-amount 0.04 \
  --budget 0.12 \
  --min-spread-bps 25 \
  --jito-tip 0.00003 \
  --poll-interval-secs 5
```

---

### 4. 🎯 Pump.fun Strict Sniper (Mode 3: Curve Momentum)
Subscribes to Yellowstone gRPC stream for real-time slot and instruction parsing with zero polling latency.
* **Anti-Rug & Anti-Dev Dump Filters**: Requires verified socials (`--require-socials`), filters out insider dev bundles (`--max-dev-buy 1.5`), enforces dev skin-in-the-game (`--min-dev-buy 0.20`), checks unique buyer threshold (`>= 5` wallets), and requires a positive buy/sell ratio within a 15-second velocity window (`>= 6 buys`, `<= 2 sells`).
* **Bonding Curve Sweet Spot**: Enters only between `8.0%` and `30.0%` curve progress to avoid early dump traps and late graduation dumps.
* **Circuit Breaker**: If 3 consecutive trades exit at a loss, immediately halt the sniper.

```bash
target/release/tradingagents-rs snipe \
  --onchain \
  --keypair-path /home/tiktoor/ta/id.json \
  --mode 3 \
  --budget 0.12 \
  --buy-amount 0.04 \
  --max-positions 2 \
  --jito-tip 0.00003 \
  --min-liquidity 2.0 \
  --require-socials \
  --max-dev-buy 1.5 \
  --min-dev-buy 0.2 \
  --min-unique-buyers 5 \
  --min-curve-pct 8.0 \
  --max-curve-pct 30.0 \
  --min-buys-recent 6 \
  --max-sells-recent 2 \
  --velocity-window-secs 15 \
  --stop-loss=-8.0 \
  --break-even-mult 1.05 \
  --tp1 1.20 \
  --tp2 1.50
```

---

### 5. 💧 Meteora & Orca Liquidity Sniper
Snipes newly initialized pools on non-Pump.fun AMMs (Meteora DLMM & Orca Whirlpools) with strict liquidity floor and age limits.

```bash
target/release/tradingagents-rs liq-snipe \
  --onchain \
  --keypair-path /home/tiktoor/ta/id.json \
  --budget 0.12 \
  --buy-amount 0.04 \
  --max-positions 1 \
  --jito-tip 0.00003 \
  --target-dexes meteora,orca \
  --min-liquidity-usd 5000 \
  --stop-loss-pct=-8.0 \
  --take-profit-pct 25.0 \
  --trailing-stop-pct=-10.0 \
  --break-even-mult 1.05 \
  --poll-interval-secs 10
```

---

### 6. 🧹 Emergency Liquidation & Rent Reclaimer
Emergency tool to liquidate open token positions back into SOL via Jupiter DEX / Pump.fun bonding curves and automatically close empty SPL & Token-2022 ATAs to reclaim rent (`+0.002039 SOL` per account):

```bash
# Liquidate all open token accounts and reclaim rent:
target/release/tradingagents-rs liquidate

# Liquidate a specific token mint:
target/release/tradingagents-rs liquidate <TOKEN_MINT_ADDRESS>
```

---

## 🛠️ Additional Subcommands & Modules

| Subcommand | Description |
|---|---|
| `analyze` | Multi-agent research debate (technical, sentiment, fundamentals) with Jupiter prediction market analysis |
| `arbitrage` | Atomic Solana cross-DEX cyclic arbitrage engine |
| `copy-trade` | Follow smart profitable memecoin wallets with convergence detection |
| `graduation` | Frontrun tokens at 85%–99% bonding curve before Raydium migration |
| `vol-spike` | 3–5x volume surge detector on established tokens |
| `analyze-wallets` | Historical wallet PnL and trade breakdown via Helius API |

---

## ⚙️ Configuration Reference & CLI Options

### Breakout Engine Flags (`breakout`)
- `--strategy`: Strategy mode (`multi`, `volume`, `dip`, `graduation`). Default: `multi`.
- `--min-liquidity-usd`: Minimum DEX liquidity in USD. Default: `40000`.
- `--min-volume-24h-usd`: Minimum 24h trading volume. Default: `250000`.
- `--min-volume-5m-usd`: Minimum 5-minute trading volume. Default: `10000`.
- `--min-buys-5m`: Minimum buy transactions in 5 minutes. Default: `25`.
- `--min-buy-ratio-5m`: Minimum 5-minute buy ratio (buys / total txs). Default: `0.60`.
- `--min-buy-ratio-1h`: Minimum 1-hour buy ratio. Default: `0.55`.
- `--stop-loss-pct`: Tight stop loss percentage (e.g. `-5.0`).
- `--trailing-take-profit-activation-pct`: Profit watermark required to activate trailing stop (e.g. `3.5` for +3.5%).
- `--trailing-take-profit-callback-pct`: Trailing distance from peak (e.g. `2.5` for 2.5% drop from high).
- `--break-even-mult`: Guaranteed gross price clamp multiplier (e.g. `1.025` for +2.5% minimum gross floor).

### Sniper Engine Flags (`snipe`)
- `--mode`: Snipe mode (`1` = dev frontrun, `2` = volume surge, `3` = curve momentum breakout).
- `--require-socials`: Require verified Twitter/Telegram in token launch metadata.
- `--min-dev-buy`: Minimum dev initial buy in SOL (skin-in-the-game filter).
- `--max-dev-buy`: Maximum dev initial buy in SOL (rejects insider pre-bundles).
- `--min-unique-buyers`: Minimum distinct wallet addresses purchasing before entry.
- `--min-curve-pct` / `--max-curve-pct`: Bonding curve progress entry corridor (e.g. `8.0` to `30.0`).
- `--velocity-window-secs`: Observation window in seconds for buy/sell velocity check.
- `--min-buys-recent` / `--max-sells-recent`: Required transaction counts inside the velocity window.
- `--tp1` / `--tp2`: First and second take-profit price multipliers.
- `--stop-loss`: Hard stop-loss percentage from entry (e.g. `--stop-loss=-8.0`).

### Mean Reversion Flags (`mean-rev`)
- `--rsi-period`: Lookback period for RSI computation. Default: `14`.
- `--rsi-oversold`: RSI entry threshold. Default: `25.0`.
- `--rsi-exit`: RSI exit threshold. Default: `55.0`.
- `--max-hold-mins`: Maximum holding period in minutes before flat/market exit. Default: `120`.

### Route Arbitrage Flags (`route-arb`)
- `--min-spread-bps`: Minimum spread in basis points required to execute (e.g. `25` = 0.25%).
- `--tokens`: Comma-separated list of target token symbols/mints to monitor.

---

## 🔒 Safety & Risk Disclaimers
* All on-chain trading involves risk of capital loss.
* Use `--budget` and `--buy-amount` to size positions appropriately.
* Always keep sufficient SOL in your wallet for rent, tx fees, and Jito MEV tips.
