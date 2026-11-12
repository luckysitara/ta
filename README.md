# 🦀 Solana Trading Agents: High-Performance Rust Sniper & Breakout Engine

A high-performance, native Rust trading engine and autonomous sniper built specifically for **Solana DEXs, Yellowstone gRPC mempool feeds, and Jito MEV bundles**.

---

## ⚡ Core Engine Architecture

```
                 [ Solana Blockchain / Yellowstone gRPC Stream ]
                                        │
           ┌────────────────────────────┴───────────────────────────┐
           ▼                                                        ▼
  [ Yellowstone Mempool Sniper ]                          [ DEX Breakout Scanner ]
  - Sub-slot pool creation detection                      - Real-time multi-pool scanning
  - Bonding curve reserves tracking                       - Liquidity & volume velocity filters
  - Option 1: Live price heuristics                       - Anti-FOMO & parabolic extension caps
  - Option 2: Anti-rug & dev holding checks               - Strict Sweet Spot enforcement
           │                                                        │
           └────────────────────────────┬───────────────────────────┘
                                        │
                                        ▼
                       [ Autonomous Strategy Classifier ]
        ┌───────────────────────────────┴───────────────────────────────┐
        ▼                                                               ▼
[ Strategy 1: Dip-Rebound ]                               [ Strategy 3: Graduation Burst ]
(5m oversold pullback in deep pools)                      (Pump.fun to Raydium migrations)
        │                                                               │
        └───────────────────────────────┬───────────────────────────────┘
                                        │
                                        ▼
                     [ Multi-Tier Trailing Profit Lockbox ]
                      - Activation Threshold: +7.0%
                      - Dynamic Trailing Callback: 1.5%
                      - Break-Even Floor Ratchet: 1.025x
                      - Tight Capital Stop Loss: -5.0%
                                        │
                                        ▼
                      [ Jito MEV On-Chain Bundle Executor ]
                      - Direct validator tip routing (0.0001 SOL)
                      - Atomic bundle inclusion / zero frontrunning
                      - Immediate ATA rent reclamation (+0.002 SOL)
```

---

## 🚀 Key Features

* **⚡ Ultra-Low Latency Yellowstone gRPC**: Streams raw validator blocks, transactions, and pool creations with zero RPC polling lag.
* **🛡️ Strict Anti-Rug & Liquidity Filters**:
  * Enforces minimum liquidity floors ($\ge \$25\text{k}$) and 5m volume thresholds ($\ge \$2\text{k}$).
  * Dev holding filters reject pools where creators retain excessive supply or bundle launches.
* **🎯 High-Conviction Strategies**:
  * **Strategy 1 (5m Oversold Dip-Rebound)**: Detects healthy pullbacks ($-2\%$ to $-25\%$) within high-depth pools ($\ge \$250\text{k}$ liquidity) and captures aggressive reversals.
  * **Strategy 3 (DEX Graduation Burst)**: Targets fresh Pump.fun-to-Raydium graduations ($\le 30$ mins old) experiencing initial institutional liquidity injections.
  * *Strategy 2 (Momentum Breakout)* has been permanently deprecated to prevent buying microcap exhaustion tops.
* **🔒 Dynamic Trailing Profit Lockbox**: Automatically ratchets guaranteed stop floors beneath peak watermarks as unrealized gains surge, locking in profits while letting winners run.
* **⚡ Jito MEV Bundle Delivery**: Delivers buy and sell instructions directly to Jito block engines with custom priority tips, shielding transactions from toxic sandwich attacks and slippage.
* **💰 Zero Stranded Rent**: Automatically closes empty Associated Token Accounts (ATAs) upon position exit, immediately reclaiming the `~0.002039 SOL` account rent back to liquid capital.

---

## 🛠️ Quick Start

### Prerequisites
* Rust 1.78+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
* Solana CLI tools (`solana-keygen`, `spl-token`)
* Solana Keypair located at `~/.config/solana/id.json`

### 1. Build the Release Binary
```bash
cd tradingagents-rs
cargo build --release
```

### 2. Run the Yellowstone Mempool Sniper
```bash
# Quick launcher with interactive strategy selection
./snipe.sh

# Or directly with Cargo:
cargo run --manifest-path tradingagents-rs/Cargo.toml -- snipe \
  --grpc-endpoint "https://your-yellowstone-grpc.com:443" \
  --grpc-token "YOUR_TOKEN" \
  --budget 0.1 \
  --buy-amount 0.015 \
  --paper
```

### 3. Run the Autonomous Multi-Strategy Breakout Engine
```bash
./tradingagents-rs/target/release/tradingagents-rs breakout \
  --strategy dip,grad \
  --budget 0.080 \
  --buy-amount 0.015 \
  --max-positions 1 \
  --jito-tip 0.0001 \
  --stop-loss-pct=-5.0 \
  --trailing-take-profit-activation-pct 7.0 \
  --trailing-take-profit-callback-pct 1.5 \
  --trailing-stop-pct=-10.0 \
  --break-even-mult 1.025 \
  --max-price-change-24h 250.0 \
  --max-price-change-1h 50.0 \
  --min-liquidity-usd 25000 \
  --min-volume-24h-usd 100000 \
  --min-volume-5m-usd 2000 \
  --poll-interval-secs 10 \
  --onchain
```

---

## 🧪 Testing Suite

Run the full native test suite covering Jito tips, rug filters, trailing take-profit ratchets, and strategy qualifications:

```bash
cd tradingagents-rs
cargo test
```

All 43 unit tests run in $< 2$ seconds and verify:
- In-slot emergency exit and dev dump rejection
- Bonding curve reserves and graduation detection
- Trailing stop calculation and profit lockbox ratchets
- Strict exclusion of deprecated breakout strategies
- Associated token account closing and rent reclamation

---

## 📜 Audit & Performance History

Full session checkpoints, transaction receipts, and live solvency records are documented in [dual_onchain_and_paper_trading_audit.md](file:///dual_onchain_and_paper_trading_audit.md).

---

## 📄 License

Apache-2.0. See `LICENSE` for details.
