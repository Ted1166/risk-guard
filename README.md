# ⚡ Risk Guard

> **AI-powered autonomous trading risk agent — built in Rust on Kraken CLI**

---

## What It Does

Risk Guard is an autonomous trading agent that puts **risk management first**.

Most trading bots chase alpha. Risk Guard's primary job is to **not lose money** — every trade decision passes through a 5-layer guard system before a single order is placed. Claude AI then decides whether the risk-approved opportunity is worth taking.

```
Market Data (Kraken CLI)
        │
        ▼
┌───────────────────┐
│   5-Guard Engine  │  Drawdown · Volatility · RSI · Spread · Cooldown
└───────┬───────────┘
        │ CLEAR or CAUTION only
        ▼
┌───────────────────┐
│   Claude AI Brain │  Reads market context → Buy / Sell / Hold
└───────┬───────────┘
        │
        ▼
┌───────────────────┐
│  Paper Executor   │  kraken paper buy/sell · SL/TP monitoring
└───────┬───────────┘
        │
        ▼
┌───────────────────┐
│ Ratatui Dashboard │  Live sparklines · Guard breakdown · Drawdown gauge
└───────────────────┘
```

---

## Features

| Feature | Detail |
|---|---|
| **5 risk guards** | Drawdown hard stop, ATR volatility regime, RSI overbought/oversold, bid-ask spread, post-stop cooldown |
| **AI trade decisions** | Claude reads market context and outputs structured JSON — buy/sell/hold + position size + SL/TP |
| **Rule-based fallback** | If Claude is unavailable, RSI + momentum rules take over automatically |
| **Dead man's switch** | `kraken order cancel-after` re-armed every cycle — agent dying can't leave open orders |
| **Paper trading** | Full `kraken paper` sandbox — live prices, zero real money, identical interface to live |
| **Braille sparklines** | 60-point price history rendered per pair in the terminal |
| **Drawdown gauge** | Visual fill bar — yellow at 7%, red at 12% hard halt |
| **Per-guard breakdown** | Every guard shows pass/fail, numeric value, and reason string |
| **Social log** | Markdown report written every 10 cycles + on shutdown — paste-ready for Discord/lablab |
| **Zero Python** | Pure Rust — async Tokio, Ratatui TUI, reqwest for Claude API |

---

## Architecture

```
contracts/
└── src/
    ├── main.rs          — async Tokio loop, dead man's switch, shutdown
    ├── config.rs        — RiskParams + AppConfig (all thresholds here)
    ├── market.rs        — Kraken CLI subprocess wrapper (typed structs)
    ├── indicators.rs    — ATR, RSI, DrawdownTracker, momentum (pure Rust)
    ├── risk.rs          — 5-guard engine → RiskVerdict + allowed_position_pct
    ├── brain.rs         — Claude API → TradeDecision, rule-based fallback
    ├── executor.rs      — Position tracking, SL/TP monitoring, paper execution
    ├── dashboard.rs     — Ratatui: sparklines, guards, gauge, positions, trades
    └── social.rs        — One-liner + markdown report generator
```

---

## Prerequisites

### 1. Rust (1.78+)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. Kraken CLI

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/krakenfx/kraken-cli/releases/latest/download/kraken-cli-installer.sh | sh
```

Verify:

```bash
kraken status && kraken ticker BTCUSD
```

### 3. Claude API key (optional — for AI brain)

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

If not set, the agent runs in **rule-based mode** (RSI + momentum logic).

---

## Running

### Build

```bash
cd contracts
cargo build --release
```

Binary lands at `target/release/risk_guard`.

### Run (paper trading — no real money)

```bash
cd contracts
cargo run --release
```

Or directly:

```bash
./target/release/risk_guard
```

### Run in rule-based mode (no Claude API needed)

```bash
AI_ENABLED=false cargo run --release
```

### Custom Kraken CLI path

```bash
KRAKEN_CLI_PATH=/usr/local/bin/kraken cargo run --release
```

---

## Controls

| Key | Action |
|---|---|
| `q` | Quit gracefully |
| `ESC` | Quit gracefully |
| `Ctrl-C` | Shutdown + write final report |

---

## Dashboard Layout

```
┌─ ⚡ RISK GUARD  #42  2026-04-07 14:32:11 ────────────┬─ Drawdown ──────┐
│  Portfolio  $10,234.56    P&L  +$234.56  (+2.35%)    │ ████░░░░░  2.3% │
│  q quit   Ctrl-C shutdown                             │  (HALT @ 12%)   │
├─ Risk Guards ─────────────────┬─ Sparklines ──────────────────────────────┤
│  BTCUSD  ● CLEAR              │ BTCUSD $61,234  ↑  ▁▂▃▄▅▆▇█▇▆           │
│    drawdown  ✓  0.02          │ ETHUSD $3,421   →  ▄▄▅▄▅▄▃▄▅▄           │
│    volatility ✓  1.12×        │ SOLUSD $142     ↓  ▇▆▅▄▃▂▁▂▁▂           │
│    rsi  ✓  RSI 54.3 [▓▓▓▓▓░░]├─ Market Snapshot ─────────────────────────┤
│    spread ✓  0.012%           │ Pair    Last $    24h Hi   RSI   Allow%  │
│                               │ BTCUSD  $61234    $62100   54.3↑  20%   │
├─ Open Positions ──────────────────────────────────────────────────────────┤
│  BTCUSD  0.003210  $60,120  $61,234  +$3.59  +1.83%  SL:$58,900  TP:—  │
├─ Recent Trades ───────────────────────────────────────────────────────────┤
│  14:28:02  BTCUSD  BUY   0.003210  $60,120  RSI 48 + momentum +1.4%  ok │
```

---

## Risk Guard Logic

### Guard cascade (evaluated in order)

```
1. Cooldown     — skip if within N seconds of a stop-loss trigger
2. Drawdown     — HALT if portfolio down ≥12% from peak (CAUTION at 7%)
3. Volatility   — HALT if ATR ≥2.5× rolling average (CAUTION at 1.5×)
4. RSI          — CAUTION if RSI >72 (overbought) or <28 (oversold)
5. Spread       — CAUTION if bid-ask spread >0.5% (thin liquidity)
```

### Position sizing

```
CLEAR   → up to 20% of portfolio
CAUTION → 20% × 0.67^(number of caution flags)
HALT    → 0% — no new entries
```

### Dead man's switch

Every cycle calls `kraken order cancel-after 120` before any other work. If the process dies mid-cycle, all open orders cancel within 120 seconds automatically.

---

## Configuration

All parameters live in `src/config.rs` — no config file needed.

```rust
pub struct RiskParams {
    pub max_position_pct:      f64,   // 0.20  — 20% max per trade
    pub max_drawdown_pct:      f64,   // 0.12  — 12% hard halt
    pub drawdown_warning_pct:  f64,   // 0.07  — 7% caution
    pub atr_period:            usize, // 14    — ATR lookback
    pub atr_max_multiple:      f64,   // 2.5   — extreme volatility halt
    pub atr_caution_multiple:  f64,   // 1.5   — elevated volatility caution
    pub rsi_period:            usize, // 14    — RSI lookback
    pub rsi_overbought:        f64,   // 72.0
    pub rsi_oversold:          f64,   // 28.0
    pub cooldown_secs:         u64,   // 300   — 5 min after stop-loss
    pub cancel_after_secs:     u64,   // 120   — dead man's switch
}
```

---

## Output

### Log file

`logs/risk_guard.log` — rolling daily, full cycle detail.

### Social reports

`logs/report_<timestamp>.md` — written every 10 cycles and on shutdown.

Example one-liner output:

```
🟢 RiskGuard | Cycle 42 | Portfolio $10,234.56 | P&L +$234.56 (+2.35%) | Trades 3/5 | Halts 1 | 14:32 UTC
```

---

## Tech Stack

| Layer | Technology |
|---|---|
| Language | Rust 2021 edition |
| Async runtime | Tokio |
| Exchange connectivity | Kraken CLI (subprocess, JSON output) |
| AI brain | Claude claude-sonnet-4 via Anthropic API |
| Terminal UI | Ratatui + Crossterm |
| HTTP client | reqwest (rustls) |
| Logging | tracing + tracing-appender |
| Time | chrono |

---

## Hackathon Tracks

- ✅ **Kraken CLI Challenge** — autonomous paper trading agent via `kraken paper`
- ✅ **Social Engagement** — markdown reports + one-liners generated every 10 cycles

---

## Disclaimer

This is experimental software built for a hackathon. Paper trading only — no real funds are at risk during the competition. The dead man's switch and risk guards are production-pattern implementations but this has not been audited for live trading. Read [Kraken CLI's DISCLAIMER.md](https://github.com/krakenfx/kraken-cli/blob/main/DISCLAIMER.md) before using with real funds.

---

*Built with Rust + Kraken CLI + Claude AI · AI Trading Agents Hackathon 2026 · #AITrading #KrakenCLI #lablab*