#!/usr/bin/env bash
WALLET="BEmX1nfeZT5i4VpSEeZmhiYxpZ9z4Y1LQLjAtPR9c3re"
KEYPAIR="/home/tiktoor/ta/id.json"
APP_DIR="/home/tiktoor/ta"
LOG="/home/tiktoor/ta/auto_manager.log"

echo "[$(date -u)] Auto-Manager Daemon started" >> "$LOG"

while true; do
    # 1. Check for empty Token-2022 ATAs and close them immediately to reclaim 0.001509 SOL rent
    ACCOUNTS=$(spl-token accounts --owner "$WALLET" --program-2022 --verbose 2>/dev/null | grep -E "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb" || true)
    if [ -n "$ACCOUNTS" ]; then
        echo "$ACCOUNTS" | while read -r line; do
            # Check balance column
            BAL=$(echo "$line" | awk '{print $NF}')
            ACC=$(echo "$line" | awk '{print $3}')
            MINT=$(echo "$line" | awk '{print $2}')
            
            # If balance is 0, reclaim rent immediately
            if [ "$BAL" = "0" ] && [ -n "$ACC" ]; then
                echo "[$(date -u)] Reclaiming rent from empty ATA $ACC (Mint: $MINT)..." >> "$LOG"
                spl-token close --address "$ACC" --program-2022 --recipient "$WALLET" --fee-payer "$KEYPAIR" >> "$LOG" 2>&1
            fi
        done
    fi

    # 2. Safety check: make sure daemons are alive
    if ! pgrep -f "breakout" > /dev/null; then
        echo "[$(date -u)] WARNING: breakout daemon died! Restarting..." >> "$LOG"
        ./tradingagents-rs/target/release/tradingagents-rs breakout \
          --strategy multi \
          --budget 0.055 \
          --buy-amount 0.020 \
          --max-positions 2 \
          --jito-tip 0.0001 \
          --stop-loss-pct=-10.0 \
          --trailing-take-profit-activation-pct 18.0 \
          --trailing-take-profit-callback-pct 2.5 \
          --trailing-stop-pct=-12.0 \
          --break-even-mult 1.05 \
          --min-liquidity-usd 35000 \
          --min-volume-24h-usd 100000 \
          --min-volume-5m-usd 5000 \
          --max-price-change-24h 250.0 \
          --max-price-change-1h 50.0 \
          --poll-interval-secs 5 \
          --onchain >> "$APP_DIR/breakout_auto.log" 2>&1 &
    fi

    sleep 15
done
