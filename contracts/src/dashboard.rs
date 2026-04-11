use std::collections::HashMap;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Gauge, Paragraph, Row, Sparkline, Table,
    },
    Frame,
};
use chrono::Local;

use crate::executor::Executor;
use crate::risk::{RiskLevel, RiskVerdict};
use crate::market::Ticker;


fn level_color(level: &RiskLevel) -> Color {
    match level {
        RiskLevel::Clear    => Color::Green,
        RiskLevel::Caution  => Color::Yellow,
        RiskLevel::Halt     => Color::Red,
        RiskLevel::Cooldown => Color::Magenta,
    }
}

fn pnl_color(v: f64) -> Color {
    if v > 0.0 { Color::Green } else if v < 0.0 { Color::Red } else { Color::White }
}

fn level_badge(level: &RiskLevel) -> &'static str {
    match level {
        RiskLevel::Clear    => "● CLEAR",
        RiskLevel::Caution  => "▲ CAUTION",
        RiskLevel::Halt     => "✖ HALT",
        RiskLevel::Cooldown => "⏸ COOLDOWN",
    }
}

// Braille sparkline helper
// Ratatui Sparkline needs u64 data; we normalise f64 prices to 0-100 range.
fn to_spark(prices: &[f64]) -> Vec<u64> {
    if prices.is_empty() { return vec![]; }
    let min = prices.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(1e-9);
    prices.iter()
        .map(|p| (((p - min) / range) * 100.0) as u64)
        .collect()
}


fn render_header(f: &mut Frame, area: Rect, executor: &Executor, cycle: u64, dd: f64) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(area);

    let ts      = Local::now().format("%Y-%m-%d  %H:%M:%S");
    let pnl     = executor.total_pnl();
    let pnl_pct = executor.total_pnl_pct();

    let lines = vec![
        Line::from(vec![
            Span::styled("⚡ RISK GUARD", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(format!("#{cycle}"), Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(ts.to_string(), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled(
                format!("Portfolio  ${:.2}", executor.portfolio_value),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                format!("P&L  ${pnl:+.2}  ({pnl_pct:+.2}%)"),
                Style::default().fg(pnl_color(pnl)).add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                format!("Positions: {}", executor.positions.len()),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::styled(" quit   ", Style::default().fg(Color::DarkGray)),
            Span::styled("Ctrl-C", Style::default().fg(Color::Yellow)),
            Span::styled(" shutdown", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    f.render_widget(Paragraph::new(lines).block(block), chunks[0]);

    let dd_pct  = (dd * 100.0).min(100.0);
    let dd_color = if dd_pct >= 12.0 {
        Color::Red
    } else if dd_pct >= 7.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Drawdown")
                .border_style(Style::default().fg(dd_color)),
        )
        .gauge_style(Style::default().fg(dd_color))
        .percent(dd_pct as u16)
        .label(format!("{dd_pct:.1}%  (HALT @ 12%)"));
    f.render_widget(gauge, chunks[1]);
}


fn render_guards(f: &mut Frame, area: Rect, verdicts: &HashMap<String, RiskVerdict>) {
    let mut rows: Vec<Row> = Vec::new();

    for (pair, v) in verdicts {
        rows.push(
            Row::new(vec![
                Cell::from(pair.as_str())
                    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Cell::from(level_badge(&v.level))
                    .style(Style::default()
                        .fg(level_color(&v.level))
                        .add_modifier(Modifier::BOLD)),
                Cell::from(""),
                Cell::from(""),
            ])
        );

        for g in &v.guards {
            let (icon, color) = if !g.passed {
                ("✗", Color::Red)
            } else {
                match g.level {
                    RiskLevel::Caution => ("▲", Color::Yellow),
                    _                  => ("✓", Color::Green),
                }
            };

            let val_str = g.value
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".to_string());

            let _thr_str = g.threshold
                .map(|t| format!("{t:.2}"))
                .unwrap_or_else(|| "—".to_string());

            rows.push(Row::new(vec![
                Cell::from(format!("  {}", g.name))
                    .style(Style::default().fg(Color::White)),
                Cell::from(icon).style(Style::default().fg(color)),
                Cell::from(val_str).style(Style::default().fg(Color::DarkGray)),
                Cell::from(
                    if g.reason.len() > 45 { format!("{}…", &g.reason[..44]) }
                    else { g.reason.clone() }
                ).style(Style::default().fg(Color::DarkGray)),
            ]));
        }

        let rsi_str = v.rsi
            .map(|r| {
                let bar = rsi_bar(r);
                format!("RSI {r:.1}  {bar}")
            })
            .unwrap_or_else(|| "RSI —".to_string());

        let atr_str = v.atr_multiple
            .map(|m| format!("ATR {m:.2}×"))
            .unwrap_or_else(|| "ATR —".to_string());

        rows.push(Row::new(vec![
            Cell::from(""),
            Cell::from(""),
            Cell::from(rsi_str).style(Style::default().fg(Color::Blue)),
            Cell::from(atr_str).style(Style::default().fg(Color::Blue)),
        ]));

        rows.push(Row::new(vec![
            Cell::from("─────────────────────────────────────────────────────")
                .style(Style::default().fg(Color::DarkGray)),
            Cell::from(""), Cell::from(""), Cell::from(""),
        ]));
    }

    if rows.is_empty() {
        rows.push(Row::new(vec![
            Cell::from("Waiting for first evaluation…").style(Style::default().fg(Color::DarkGray)),
            Cell::from(""), Cell::from(""), Cell::from(""),
        ]));
    }

    let widths = [
        Constraint::Length(16),
        Constraint::Length(12),
        Constraint::Length(14),
        Constraint::Min(20),
    ];

    let table = Table::new(rows, widths)
        .header(
            Row::new(["Guard", "Status", "Value", "Reason"])
                .style(Style::default().add_modifier(Modifier::BOLD))
                .bottom_margin(1),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(" Risk Guards "),
        );
    f.render_widget(table, area);
}

fn rsi_bar(rsi: f64) -> String {
    let filled = ((rsi / 100.0) * 10.0) as usize;
    let empty  = 10_usize.saturating_sub(filled);
    format!("[{}{}]", "▓".repeat(filled), "░".repeat(empty))
}


fn render_sparklines(
    f: &mut Frame,
    area: Rect,
    pairs: &[String],
    price_history: &HashMap<String, Vec<f64>>,
    tickers: &HashMap<String, Ticker>,
) {
    let n = pairs.len().max(1);
    let heights: Vec<Constraint> = (0..n).map(|_| Constraint::Min(3)).collect();

    let slots = Layout::default()
        .direction(Direction::Vertical)
        .constraints(heights)
        .split(area);

    for (i, pair) in pairs.iter().enumerate() {
        let history = price_history.get(pair).cloned().unwrap_or_default();
        let last    = tickers.get(pair).map(|t| t.last).unwrap_or(0.0);
        let spark_data = to_spark(&history);

        let trend = if history.len() >= 3 {
            let recent = &history[history.len()-3..];
            if recent[2] > recent[0] { "↑" } else if recent[2] < recent[0] { "↓" } else { "→" }
        } else { "→" };

        let trend_color = match trend {
            "↑" => Color::Green,
            "↓" => Color::Red,
            _   => Color::White,
        };

        let title = Line::from(vec![
            Span::styled(pair.as_str(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(format!("${last:.4}"), Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled(trend, Style::default().fg(trend_color).add_modifier(Modifier::BOLD)),
        ]);

        let spark_color = trend_color;

        let spark = Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(title),
            )
            .data(&spark_data)
            .style(Style::default().fg(spark_color))
            .bar_set(symbols::bar::NINE_LEVELS);

        if i < slots.len() {
            f.render_widget(spark, slots[i]);
        }
    }
}


fn render_market(
    f: &mut Frame,
    area: Rect,
    pairs: &[String],
    tickers: &HashMap<String, Ticker>,
    verdicts: &HashMap<String, RiskVerdict>,
) {
    let header = Row::new(["Pair", "Last $", "24h Hi", "24h Lo", "Spread%", "RSI", "Allow%"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let rows: Vec<Row> = pairs.iter().map(|pair| {
        let t = tickers.get(pair.as_str());
        let v = verdicts.get(pair.as_str());

        let (last, hi, lo, spread) = t
            .map(|t| (t.last, t.high_24h, t.low_24h, t.spread_pct()))
            .unwrap_or((0.0, 0.0, 0.0, 0.0));

        let rsi_str = v.and_then(|v| v.rsi)
            .map(|r| {
                let color_hint = if r > 70.0 { "↑" } else if r < 30.0 { "↓" } else { "" };
                format!("{r:.1}{color_hint}")
            })
            .unwrap_or_else(|| "--".to_string());

        let allow_str = v
            .map(|v| format!("{:.0}%", v.allowed_position_pct * 100.0))
            .unwrap_or_else(|| "--".to_string());

        let level = v.map(|v| &v.level);
        let row_style = match level {
            Some(RiskLevel::Halt) | Some(RiskLevel::Cooldown) =>
                Style::default().fg(Color::DarkGray),
            _ => Style::default(),
        };

        Row::new(vec![
            Cell::from(pair.as_str()),
            Cell::from(format!("${last:.4}")),
            Cell::from(format!("${hi:.2}")),
            Cell::from(format!("${lo:.2}")),
            Cell::from(format!("{spread:.3}")),
            Cell::from(rsi_str),
            Cell::from(allow_str),
        ]).style(row_style)
    }).collect();

    let widths = [
        Constraint::Length(10), Constraint::Length(12), Constraint::Length(12),
        Constraint::Length(12), Constraint::Length(9),  Constraint::Length(8),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Market Snapshot "),
        );
    f.render_widget(table, area);
}


fn render_positions(
    f: &mut Frame,
    area: Rect,
    executor: &Executor,
    tickers: &HashMap<String, Ticker>,
) {
    let header = Row::new(["Pair", "Vol", "Entry $", "Current $", "P&L $", "P&L%", "SL $", "TP $"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let rows: Vec<Row> = if executor.positions.is_empty() {
        vec![Row::new(vec![
            Cell::from("—"), Cell::from("—"), Cell::from("—"), Cell::from("—"),
            Cell::from("No open positions"), Cell::from("—"), Cell::from("—"), Cell::from("—"),
        ]).style(Style::default().fg(Color::DarkGray))]
    } else {
        executor.positions.values().map(|pos| {
            let curr = tickers.get(pos.pair.as_str()).map(|t| t.last).unwrap_or(pos.entry_price);
            let mut p = pos.clone();
            p.current_price = curr;
            let pnl_usd = p.pnl_usd();
            let pnl_pct = p.pnl_pct();
            let pc = pnl_color(pnl_usd);

            Row::new(vec![
                Cell::from(pos.pair.as_str()),
                Cell::from(format!("{:.6}", pos.volume)),
                Cell::from(format!("${:.4}", pos.entry_price)),
                Cell::from(format!("${curr:.4}")),
                Cell::from(format!("${pnl_usd:+.2}")).style(Style::default().fg(pc)),
                Cell::from(format!("{pnl_pct:+.2}%")).style(Style::default().fg(pc)),
                Cell::from(pos.stop_loss_price.map(|p| format!("${p:.2}")).unwrap_or_else(|| "--".to_string())),
                Cell::from(pos.take_profit_price.map(|p| format!("${p:.2}")).unwrap_or_else(|| "--".to_string())),
            ])
        }).collect()
    };

    let widths = [
        Constraint::Length(10), Constraint::Length(12), Constraint::Length(12),
        Constraint::Length(12), Constraint::Length(12), Constraint::Length(10),
        Constraint::Length(10), Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .title(" Open Positions "),
        );
    f.render_widget(table, area);
}


fn render_trades(f: &mut Frame, area: Rect, executor: &Executor) {
    let header = Row::new(["Time", "Pair", "Action", "Volume", "Price $", "Reason", "Result"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let recent  = executor.recent_trades(6);
    let rows: Vec<Row> = if recent.is_empty() {
        vec![Row::new(vec![
            Cell::from("—"), Cell::from("—"), Cell::from("—"), Cell::from("—"),
            Cell::from("—"), Cell::from("Waiting for first trade…"), Cell::from("—"),
        ]).style(Style::default().fg(Color::DarkGray))]
    } else {
        recent.iter().map(|e| {
            let ts           = e.timestamp.format("%H:%M:%S").to_string();
            let action_color = match e.action.as_str() {
                "buy"  => Color::Green,
                "sell" => Color::Red,
                _      => Color::DarkGray,
            };
            let result_color = match e.result.as_str() {
                "ok"    => Color::Green,
                "error" => Color::Red,
                _       => Color::DarkGray,
            };
            let reason = if e.reason.len() > 38 {
                format!("{}…", &e.reason[..37])
            } else {
                e.reason.clone()
            };

            Row::new(vec![
                Cell::from(ts),
                Cell::from(e.pair.as_str()),
                Cell::from(e.action.to_uppercase())
                    .style(Style::default().fg(action_color).add_modifier(Modifier::BOLD)),
                Cell::from(if e.volume > 0.0 { format!("{:.6}", e.volume) } else { "--".to_string() }),
                Cell::from(format!("${:.4}", e.price)),
                Cell::from(reason).style(Style::default().fg(Color::DarkGray)),
                Cell::from(e.result.as_str())
                    .style(Style::default().fg(result_color).add_modifier(Modifier::BOLD)),
            ])
        }).collect()
    };

    let widths = [
        Constraint::Length(10), Constraint::Length(9), Constraint::Length(8),
        Constraint::Length(12), Constraint::Length(12), Constraint::Min(20),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta))
                .title(" Recent Trades "),
        );
    f.render_widget(table, area);
}


pub fn render(
    f: &mut Frame,
    executor: &Executor,
    pairs: &[String],
    tickers: &HashMap<String, Ticker>,
    verdicts: &HashMap<String, RiskVerdict>,
    price_history: &HashMap<String, Vec<f64>>,
    cycle: u64,
    current_drawdown: f64,
    ) {
    let size = f.size();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(20),
            Constraint::Length(6),
            Constraint::Length(10),
        ])
        .split(size);

    render_header(f, rows[0], executor, cycle, current_drawdown);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(rows[1]);

    render_guards(f, body[0], verdicts);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((pairs.len() as u16) * 4 + 1),
            Constraint::Min(7),
        ])
        .split(body[1]);

    render_sparklines(f, right[0], pairs, price_history, tickers);
    render_market(f, right[1], pairs, tickers, verdicts);

    render_positions(f, rows[2], executor, tickers);
    render_trades(f, rows[3], executor);
}