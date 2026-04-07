use std::fmt::Write as FmtWrite;
use chrono::Local;
use crate::executor::Executor;
use crate::risk::RiskVerdict;
use std::collections::HashMap;

#[derive(Debug)]
pub struct SessionSummary {
    pub cycle: u64,
    pub portfolio_value: f64,
    pub pnl_usd: f64,
    pub pnl_pct: f64,
    pub trades_total: usize,
    pub trades_ok: usize,
    #[allow(dead_code)]
    pub pairs_monitored: usize,
    pub risk_halts: usize,
}

pub fn one_liner(summary: &SessionSummary) -> String {
    let emoji = if summary.pnl_usd >= 0.0 { "🟢" } else { "🔴" };
    format!(
        "{emoji} RiskGuard | Cycle {} | Portfolio ${:.2} | P&L ${:+.2} ({:+.2}%) | \
         Trades {}/{} | Halts {} | {}",
        summary.cycle,
        summary.portfolio_value,
        summary.pnl_usd,
        summary.pnl_pct,
        summary.trades_ok,
        summary.trades_total,
        summary.risk_halts,
        Local::now().format("%H:%M UTC"),
    )
}

#[allow(dead_code)]
pub fn social_post(summary: &SessionSummary, verdicts: &HashMap<String, RiskVerdict>) -> String {
    let emoji   = if summary.pnl_usd >= 0.0 { "🟢" } else { "🔴" };
    let dd_line = verdicts
        .values()
        .map(|v| format!("{}: {}", v.pair, v.level))
        .collect::<Vec<_>>()
        .join(" | ");

    format!(
        "{emoji} #RiskGuard update\n\
         Portfolio: ${:.2}  P&L: {:+.2}%\n\
         Trades: {}/{}  Risk halts: {}\n\
         Guards: {}\n\
         #AITrading #KrakenCLI #lablab",
        summary.portfolio_value,
        summary.pnl_pct,
        summary.trades_ok,
        summary.trades_total,
        summary.risk_halts,
        dd_line,
    )
}

pub fn markdown_report(
    summary: &SessionSummary,
    executor: &Executor,
    verdicts: &HashMap<String, RiskVerdict>,
) -> String {
    let mut out = String::new();
    let ts = Local::now().format("%Y-%m-%d %H:%M UTC");

    let _ = writeln!(out, "## ⚡ Risk Guard — Session Report");
    let _ = writeln!(out, "> {ts} | Cycle #{}", summary.cycle);
    let _ = writeln!(out);

    let pnl_icon = if summary.pnl_usd >= 0.0 { "🟢" } else { "🔴" };
    let _ = writeln!(out, "### Portfolio");
    let _ = writeln!(out, "| Metric | Value |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| Balance | ${:.2} |", summary.portfolio_value);
    let _ = writeln!(out, "| P&L | {pnl_icon} ${:+.2} ({:+.2}%) |", summary.pnl_usd, summary.pnl_pct);
    let _ = writeln!(out, "| Trades executed | {}/{} |", summary.trades_ok, summary.trades_total);
    let _ = writeln!(out, "| Risk halts triggered | {} |", summary.risk_halts);
    let _ = writeln!(out);

    let _ = writeln!(out, "### Risk Guard Status");
    let _ = writeln!(out, "| Pair | Level | Summary |");
    let _ = writeln!(out, "|---|---|---|");
    for (pair, v) in verdicts {
        let _ = writeln!(out, "| {} | **{}** | {} |", pair, v.level, v.summary);
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "### Open Positions");
    if executor.positions.is_empty() {
        let _ = writeln!(out, "_No open positions._");
    } else {
        let _ = writeln!(out, "| Pair | Volume | Entry $ | Current $ | P&L $ |");
        let _ = writeln!(out, "|---|---|---|---|---|");
        for (_, pos) in &executor.positions {
            let _ = writeln!(
                out, "| {} | {:.6} | ${:.4} | ${:.4} | ${:+.2} |",
                pos.pair, pos.volume, pos.entry_price,
                pos.current_price, pos.pnl_usd(),
            );
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "### Recent Trades");
    let recent = executor.recent_trades(5);
    if recent.is_empty() {
        let _ = writeln!(out, "_No trades yet._");
    } else {
        let _ = writeln!(out, "| Time | Pair | Action | Volume | Price $ | Result |");
        let _ = writeln!(out, "|---|---|---|---|---|---|");
        for e in recent {
            let ts = e.timestamp.format("%H:%M:%S");
            let _ = writeln!(
                out, "| {} | {} | **{}** | {:.6} | ${:.4} | {} |",
                ts, e.pair, e.action.to_uppercase(),
                e.volume, e.price, e.result,
            );
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "_Built with Rust + Kraken CLI + Claude AI | #AITrading #lablab_");

    out
}

pub fn save_report(
    summary: &SessionSummary,
    executor: &Executor,
    verdicts: &HashMap<String, RiskVerdict>,
) -> anyhow::Result<String> {
    std::fs::create_dir_all("logs")?;
    let filename = format!(
        "logs/report_{}.md",
        Local::now().format("%Y%m%d_%H%M%S")
    );
    let content = markdown_report(summary, executor, verdicts);
    std::fs::write(&filename, &content)?;
    Ok(filename)
}

pub fn build_summary(
    cycle: u64,
    executor: &Executor,
    verdicts: &HashMap<String, RiskVerdict>,
) -> SessionSummary {
    let all_trades: Vec<_> = executor
        .exec_log
        .iter()
        .filter(|e| e.action != "hold")
        .collect();

    let risk_halts = verdicts
        .values()
        .filter(|v| matches!(v.level, crate::risk::RiskLevel::Halt | crate::risk::RiskLevel::Cooldown))
        .count();

    SessionSummary {
        cycle,
        portfolio_value: executor.portfolio_value,
        pnl_usd: executor.total_pnl(),
        pnl_pct: executor.total_pnl_pct(),
        trades_total: all_trades.len(),
        trades_ok: all_trades.iter().filter(|e| e.result == "ok").count(),
        pairs_monitored: verdicts.len(),
        risk_halts,
    }
}