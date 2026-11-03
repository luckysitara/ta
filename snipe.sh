#!/usr/bin/env bash
# ==============================================================================
# TradingAgents-rs: Solana Yellowstone gRPC Sniper Quick Launcher
# Supports: Standard (1), Option 1 (2), Option 2 (3), High-Conviction (4)
# ==============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

ARGS=()
HAS_MODE=false
INTERACTIVE=false

if [ -t 0 ]; then
    INTERACTIVE=true
fi

# Pre-process arguments for convenience flags and check if mode is provided
while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode|-m)
            HAS_MODE=true
            ARGS+=("$1" "$2")
            shift 2
            ;;
        --mode=*)
            HAS_MODE=true
            ARGS+=("$1")
            shift
            ;;
        --opt1|--option1|--reserves)
            HAS_MODE=true
            ARGS+=("--mode" "2")
            shift
            ;;
        --opt2|--option2|--strict)
            HAS_MODE=true
            ARGS+=("--mode" "3")
            shift
            ;;
        --high-conviction|--combined)
            HAS_MODE=true
            ARGS+=("--mode" "4")
            shift
            ;;
        --momentum|--breakout)
            HAS_MODE=true
            ARGS+=("--mode" "5")
            shift
            ;;
        --live-reserves|--require-socials|--min-dev-buy|--min-curve-pct|-h|--help)
            HAS_MODE=true
            ARGS+=("$1")
            shift
            ;;
        *)
            ARGS+=("$1")
            shift
            ;;
    esac
done

# If no specific mode flag was passed and running interactively, prompt user
if [ "$HAS_MODE" = false ] && [ "$INTERACTIVE" = true ]; then
    echo "================================================================================"
    echo "   ⚡ TRADINGAGENTS-RS: CHOOSE YOUR SNIPER STRATEGY MODE"
    echo "================================================================================"
    echo "  1) Standard Mode (Defaults: Fastest mempool heuristics, freeze/mint revoke)"
    echo "  2) Option 1 Only (Live On-Chain Bonding Curve Reserves + Dev Skin-in-Game)"
    echo "  3) Option 2 Only (Strict Anti-Rug & Verified Socials Filter)"
    echo "  4) High-Conviction Mode (Option 1 + Option 2 Combined)"
    echo "  5) Momentum Breakout Mode (≥10% Curve Completion / 8.5+ SOL Reserves)"
    echo "--------------------------------------------------------------------------------"
    read -p "Select Strategy Mode [1-5, Default: 1]: " user_choice
    echo ""

    case "$user_choice" in
        2)
            ARGS=("--mode" "2" "${ARGS[@]}")
            ;;
        3)
            ARGS=("--mode" "3" "${ARGS[@]}")
            ;;
        4)
            ARGS=("--mode" "4" "${ARGS[@]}")
            ;;
        5)
            ARGS=("--mode" "5" "${ARGS[@]}")
            ;;
        *)
            ARGS=("--mode" "1" "${ARGS[@]}")
            ;;
    esac
fi

cargo run --manifest-path tradingagents-rs/Cargo.toml -- snipe "${ARGS[@]}"
