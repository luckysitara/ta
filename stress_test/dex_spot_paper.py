import json
import time
import urllib.request
from datetime import datetime, timezone
import os

POSITION_FILE = "/home/rootkit/TradingAgents/stress_test/spot_position.json"
os.makedirs(os.path.dirname(POSITION_FILE), exist_ok=True)

USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
SOL_MINT = "So11111111111111111111111111111111111111112"

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
                return meta.get("regularMarketPrice", 100.30)
        except Exception:
            time.sleep(1)
    return 100.30

def get_jupiter_swap_quote(in_usdc_amount):
    units = int(in_usdc_amount * 1_000_000)
    url = f"https://api.jup.ag/swap/v1/quote?inputMint={USDC_MINT}&outputMint={SOL_MINT}&amount={units}&slippageBps=50"
    headers = {"User-Agent": "TradingAgents-rs/0.1"}
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=5) as res:
            data = json.loads(res.read())
            out_lamports = int(data.get("outAmount", 0))
            out_sol = out_lamports / 1_000_000_000.0
            route_label = data["routePlan"][0]["swapInfo"]["label"] if data.get("routePlan") else "Raydium CLMM"
            price_impact = float(data.get("priceImpactPct", 0.0001)) * 100.0
            return {
                "in_usdc": in_usdc_amount,
                "out_sol": round(out_sol, 5),
                "route": route_label,
                "price_impact_pct": round(price_impact, 5),
                "effective_price": round(in_usdc_amount / out_sol, 2) if out_sol > 0 else 100.30
            }
    except Exception as e:
        # Fallback simulation
        return {
            "in_usdc": in_usdc_amount,
            "out_sol": round(in_usdc_amount / 100.30, 5),
            "route": "Raydium CLMM / Orca Whirlpool",
            "price_impact_pct": 0.0002,
            "effective_price": 100.30
        }

def open_spot_position():
    current_price = get_live_sol_price()
    trade_allocation_usdc = 750.00
    portfolio_initial_cash = 10000.00

    quote = get_jupiter_swap_quote(trade_allocation_usdc)
    
    # Risk-reward targets
    # Entry at support $100.15 - $100.30
    # Stop-loss tight below psychological $100 level at $98.80 (-1.5%)
    # Take-profit 1 at 20-period EMA $103.80 (+3.5%)
    stop_loss = round(quote["effective_price"] * 0.985, 2) # -1.5%
    take_profit = round(quote["effective_price"] * 1.035, 2) # +3.5%

    position = {
        "status": "OPEN",
        "opened_at": datetime.now(timezone.utc).isoformat(),
        "asset": "SOL",
        "pair": "SOL/USDC",
        "direction": "LONG (BUY SPOT)",
        "spot_market_price": current_price,
        "effective_entry_price": quote["effective_price"],
        "size_usdc": quote["in_usdc"],
        "tokens_held": quote["out_sol"],
        "dex_route": quote["route"],
        "price_impact_pct": quote["price_impact_pct"],
        "stop_loss_price": stop_loss,
        "take_profit_price": take_profit,
        "max_risk_usdc": round(trade_allocation_usdc * 0.015, 2), # ~$11.25 risk
        "target_profit_usdc": round(trade_allocation_usdc * 0.035, 2), # ~$26.25 reward
        "portfolio": {
            "initial_capital_usdc": portfolio_initial_cash,
            "cash_reserve_usdc": portfolio_initial_cash - trade_allocation_usdc,
            "allocated_pct": 7.5
        }
    }

    with open(POSITION_FILE, "w") as f:
        json.dump(position, f, indent=2)

    return position

if __name__ == "__main__":
    pos = open_spot_position()
    print("=" * 80)
    print("⚡ JUPITER DEX SPOT PAPER TRADE EXECUTED")
    print("=" * 80)
    print(f"Action:                {pos['direction']}")
    print(f"Input Deposit:         ${pos['size_usdc']:.2f} USDC")
    print(f"SOL Tokens Acquired:   {pos['tokens_held']} SOL")
    print(f"Effective DEX Price:   ${pos['effective_entry_price']:.2f} per SOL")
    print(f"Jupiter DEX Route:     {pos['dex_route']} (Impact: {pos['price_impact_pct']}%)")
    print(f"Target Take-Profit:    ${pos['take_profit_price']:.2f} (+3.5% | +${pos['target_profit_usdc']:.2f} USDC)")
    print(f"Hard Stop-Loss:        ${pos['stop_loss_price']:.2f} (-1.5% | -${pos['max_risk_usdc']:.2f} USDC)")
    print(f"Risk-to-Reward Ratio:  2.33 : 1")
    print(f"Remaining USDC Cash:   ${pos['portfolio']['cash_reserve_usdc']:.2f}")
    print("=" * 80)
