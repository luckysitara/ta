# Active Solana Mainnet Trading Strategies & Command Execution Reference

This document records the exact running commands, configurations, parameters, and daemon setups currently active on Solana mainnet for wallet `BEmX1nfeZT5i4VpSEeZmhiYxpZ9z4Y1LQLjAtPR9c3re`.

---

## 1. Patched Mean Reversion RSI Engine
- **Task ID**: `task-37390`
- **Binary**: `target/release/tradingagents-rs mean-rev`
- **Key Enhancements Active**:
  - Minimum 28 price samples required before signal calculation (prevents false startup entries).
  - Volatility clamp: clamped to neutral `50.0` when zero price movement occurs.
  - Unlimited hold time (`--max-hold-mins 0`) to prevent unprofitable time-based stopouts.
  - True All-In Net PnL accounting standards.
- **Exact Command**:
```bash
target/release/tradingagents-rs mean-rev \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --budget 0.20 \
  --buy-amount 0.04 \
  --max-positions 5 \
  --jito-tip 0.00003 \
  --rsi-period 14 \
  --rsi-oversold 25.0 \
  --rsi-exit 55.0 \
  --stop-loss-pct=-8.0 \
  --take-profit-pct 15.0 \
  --max-hold-mins 0 \
  --poll-interval-secs 20
```

---

## 2. Cross-DEX Atomic Arbitrage Engine (Raydium, Meteora, Orca, PumpSwap)
- **Task ID**: `task-37654`
- **Binary**: `target/release/tradingagents-rs arbitrage`
- **Key Enhancements Active**:
  - Single-transaction atomic cyclic arbitrage execution via Jito MEV bundles.
  - Zero bag-holding risk: if roundtrip output does not yield net profit after fees and tips, transaction reverts completely.
  - Multi-pool candidate discovery across major Solana AMMs.
- **Exact Command**:
```bash
./target/release/tradingagents-rs arbitrage \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --trade-amount 0.02 \
  --budget 0.08 \
  --min-profit-bps 25 \
  --jito-tip 0.00003 \
  --poll-interval-secs 5
```

---

## 3. Jupiter Route Arbitrage (Kamino Flash Loan Enabled)
- **Task ID**: `task-37626`
- **Binary**: `target/release/tradingagents-rs route-arb`
- **Key Enhancements Active**:
  - **Kamino Flash Loan ENABLED** (`5.00 SOL` borrow size) for zero-collateral cross-DEX execution.
  - Dynamic spread detection across Whirlpool, Raydium CLMM, and Meteora DLMM.
  - Real-time Jito bundle broadcast.
- **Exact Command**:
```bash
./target/release/tradingagents-rs route-arb \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --use-flash-loan \
  --flash-loan-amount 5.0 \
  --min-spread-bps 25 \
  --jito-tip 0.00003 \
  --poll-interval-secs 5
```

---

## 4. Yellowstone gRPC Real-Time Memecoin Sniper
- **Task ID**: `task-37388`
- **Binary**: `target/release/tradingagents-rs snipe`
- **Key Enhancements Active**:
  - Direct geyser Yellowstone gRPC feed over 1.48M+ mainnet transactions.
  - Bonding curve momentum sweet spot filter: `25.0% – 75.0%`.
  - Anti-rug defense: Creator dump detection and automated pool blacklisting.
  - Buy velocity filter: minimum 8 unique buyers, at least 6 buys in 15s window, max 1 sell.
  - Requires dev sold-out or dev holding <= 0%.
- **Exact Command**:
```bash
./target/release/tradingagents-rs snipe \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --budget 0.20 \
  --buy-amount 0.04 \
  --max-positions 5 \
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

---

## 5. Secondary Market DEX Breakout & Volume Surge Scalper
- **Task ID**: `task-37387`
- **Binary**: `target/release/tradingagents-rs breakout`
- **Key Enhancements Active**:
  - Minimum liquidity floor: `$40,000 USD`.
  - 5-minute volume surge detection ($10k+ volume, >= 60% buy ratio).
  - Trailing take-profit activation: `+3.5%` with `2.5%` callback.
  - Break-even multiplier floor: `>= 1.025x`.
- **Exact Command**:
```bash
target/release/tradingagents-rs breakout \
  --onchain \
  --keypair-path /home/rootkit/.config/solana/id.json \
  --strategy multi \
  --budget 0.20 \
  --buy-amount 0.04 \
  --max-positions 5 \
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

## 6. Real-Time Cadence Monitor & Audit Engine
- **Task ID**: `task-36596`
- **Interval**: Every 2 minutes (`*/2 * * * *`)
- **Output Target**: `dual_onchain_and_paper_trading_audit.md`
- **Accounting Methodology**: True All-In Net PnL (including roundtrip DEX pool fees, price impact, and dual Jito entry/exit MEV tips).
