# AGENT TRADING RULES & MANDATORY EXECUTION FILTERS

> **CRITICAL DIRECTIVE**: ANY AI Agent, automated process, or CLI runner operating in this repository **MUST STRICTLY ENFORCE** the commands, parameters, and risk filters documented below and in `ACTIVE_STRATEGIES_AND_COMMANDS.md`.
> **NEVER** run commands without these filters. Unfiltered trading causes immediate loss of funds to low-liquidity traps, sandwich attacks, and dev rugpulls.

---

## 1. Golden Rules of Execution

1. **MAXIMUM ACTIVE CONCURRENT TRADES**: Capped strictly at **10 concurrent positions**. Never open an 11th trade until an existing position has been fully closed.
2. **POSITION SIZING**:
   - Buy Amount per position: **`0.04 SOL`**
   - Strategy Budget Cap: **`0.40 SOL`** per engine
3. **STRICT EXIT & RISK MANAGEMENT**:
   - **Stop Loss**: Mandatory hard stop loss at **`-8.0%`** (Mean Reversion) or **`-5.0%`** (Breakout / Sniper).
   - **Take Profit**: **`+15.0%`** target or RSI exit threshold **`> 55.0`** (Mean Reversion), or trailing take-profit **`+3.5%`** with **`2.5%`** callback (Breakout).
   - **No Unshielded pump.fun tokens**: Reject any token with dev holding > 0% or liquidity < $40,000 USD.
4. **MEV & EXECUTION SAFETY**:
   - Maximum Jito tip: **`0.00003 SOL`** per transaction.
   - Pre-flight simulation check required before broadcasting any transaction.
   - Closed token accounts must have their ATA closed to reclaim SOL rent back to the wallet.

---

## 2. Mandatory Strategy Launch Commands

### Engine 1: Mean Reversion RSI Engine (Established Tokens Only)
```bash
target/release/tradingagents-rs mean-rev \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --budget 0.40 \
  --buy-amount 0.04 \
  --max-positions 10 \
  --jito-tip 0.00003 \
  --rsi-period 14 \
  --rsi-oversold 25.0 \
  --rsi-exit 55.0 \
  --stop-loss-pct=-8.0 \
  --take-profit-pct 15.0 \
  --max-hold-mins 0 \
  --poll-interval-secs 20
```
*Filters*:
- Requires 28 price history samples before entry.
- Zero-volatility clamped to neutral 50.0.
- Tracks verified tokens universe only.

### Engine 2: Secondary Market Breakout & Volume Surge Scalper
```bash
target/release/tradingagents-rs breakout \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --strategy multi \
  --budget 0.40 \
  --buy-amount 0.04 \
  --max-positions 10 \
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
*Filters*:
- Minimum Liquidity: **$40,000 USD**
- Minimum 5-minute volume: **$10,000 USD** with >= 60% buy ratio.
- Trailing take-profit with breakeven protection.

### Engine 3: Yellowstone gRPC Real-Time Sniper
```bash
target/release/tradingagents-rs snipe \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --budget 0.40 \
  --buy-amount 0.04 \
  --max-positions 10 \
  --jito-tip 0.00003 \
  --min-curve-pct 25.0 \
  --max-curve-pct 75.0 \
  --min-unique-buyers 8 \
  --min-buys-recent 6 \
  --max-sells-recent 1 \
  --velocity-window-secs 15 \
  --max-dev-buy 1.5 \
  --live-reserves \
  --require-dev-sold-out true \
  --enable-raydium true \
  --stop-loss=-5.0 \
  --tp1 1.05 \
  --tp2 1.15 \
  --break-even-mult 1.025
```
*Filters*:
- Bonding curve sweet spot: **25.0% - 75.0%**
- Dev sold out: **`require-dev-sold-out true`** (dev must hold 0%)
- Minimum 8 unique buyers with velocity window checks (min 6 buys, max 1 sell in 15s).

### Engine 4: Cross-DEX Atomic Arbitrage (Risk-Free Atomic Cycle)
```bash
target/release/tradingagents-rs arbitrage \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --trade-amount 0.02 \
  --budget 0.08 \
  --min-profit-bps 25 \
  --jito-tip 0.00003 \
  --poll-interval-secs 5
```

### Engine 5: Jupiter Route Arbitrage w/ Kamino Flash Loan
```bash
target/release/tradingagents-rs route-arb \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --use-flash-loan \
  --flash-loan-amount 5.0 \
  --min-spread-bps 25 \
  --jito-tip 0.00003 \
  --poll-interval-secs 5
```

---

## 3. Discretionary Liquidation / Position Close Command
When instructed to close positions or liquidate back into SOL:
```bash
target/release/tradingagents-rs liquidate --mint <MINT_ADDRESS>
```
Or for all open tokens:
```bash
target/release/tradingagents-rs liquidate
```
*Always reclaims SPL account rent back to the wallet SOL balance.*
