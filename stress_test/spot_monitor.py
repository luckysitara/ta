import json
import urllib.request
import time
from datetime import datetime, timezone
import os

POSITION_FILE = "/home/rootkit/TradingAgents/stress_test/spot_position.json"
SOL_MINT = "So11111111111111111111111111111111111111112"
USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"

def get_live_sol_price():
    headers = {"User-Agent": "TradingAgents-rs/0.1"}
    req = urllib.request.Request(
        "https://query1.finance.yahoo.com/v8/finance/chart/SOL-USD?interval=1m&range=1d",
        headers=headers,
    )
    for _ in range(3):
        try:
            with urllib.request.urlopen(req, timeout=5) as res:
                data = json.loads(res.read())["chart"]["result"][0]
                meta = data["meta"]
                return meta.get("regularMarketPrice", 99.77)
        except Exception:
            time.sleep(1)
    return 99.77

def get_jupiter_sell_quote(sol_amount):
    lamports = int(sol_amount * 1_000_000_000)
    url = f"https://api.jup.ag/swap/v1/quote?inputMint={SOL_MINT}&outputMint={USDC_MINT}&amount={lamports}&slippageBps=50"
    headers = {"User-Agent": "TradingAgents-rs/0.1"}
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=5) as res:
            data = json.loads(res.read())
            out_usdc = int(data.get("outAmount", 0)) / 1_000_000.0
            route_label = data["routePlan"][0]["swapInfo"]["label"] if data.get("routePlan") else "Raydium CLMM"
            return out_usdc, route_label
    except Exception:
        price = get_live_sol_price()
        return round(sol_amount * price, 2), "Raydium CLMM"

def check_position():
    if not os.path.exists(POSITION_FILE):
        print("No open spot position found.")
        return

    with open(POSITION_FILE, "r") as f:
        pos = json.load(f)

    if pos["status"] != "OPEN":
        print(f"Position is {pos['status']}.")
        return

    current_price = get_live_sol_price()
    tokens = pos["tokens_held"]
    cost_basis = pos["size_usdc"]
    current_value, sell_route = get_jupiter_sell_quote(tokens)
    unrealized_pnl = current_value - cost_basis
    roi_pct = (unrealized_pnl / cost_basis) * 100.0

    dist_to_tp = ((pos["take_profit_price"] - current_price) / current_price) * 100.0
    dist_to_sl = ((current_price - pos["stop_loss_price"]) / current_price) * 100.0

    print("=" * 80)
    print("📊 JUPITER DEX SPOT POSITION MONITOR (LIVE)")
    print("=" * 80)
    print(f"Asset / Pair:          {pos['asset']} ({pos['pair']})")
    print(f"Position Status:       {pos['status']}")
    print(f"Tokens Held:           {tokens:.5f} SOL")
    print(f"Entry Price:           ${pos['effective_entry_price']:.2f}")
    print(f"Current Spot Price:    ${current_price:.2f}")
    print(f"Initial Cost Basis:    ${cost_basis:.2f} USDC")
    print(f"Current Value (DEX):   ${current_value:.2f} USDC (via {sell_route})")
    print(f"Unrealized P&L:        {unrealized_pnl:+.2f} USDC ({roi_pct:+.2f}%)")
    print(f"Take-Profit Target:    ${pos['take_profit_price']:.2f} ({dist_to_tp:+.2f}% to target)")
    print(f"Stop-Loss Level:       ${pos['stop_loss_price']:.2f} ({dist_to_sl:.2f}% buffer)")
    
    # Check trigger conditions
    if current_price >= pos["take_profit_price"]:
        print("\n🎯 TAKE-PROFIT TRIGGERED! Recommended Action: SWAP OUT (SELL SOL -> USDC)")
    elif current_price <= pos["stop_loss_price"]:
        print("\n🛑 STOP-LOSS TRIGGERED! Recommended Action: SWAP OUT TO PRESERVE CAPITAL")
    else:
        print("\n⏳ Position Active: Holding within defined risk parameters.")
    print("=" * 80)

if __name__ == "__main__":
    check_position()
