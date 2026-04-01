use std::collections::HashMap;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};
use chrono::Local;

use crate::executor::{Executor, Position};
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

fn pnl_color(value: f64) -> Color {
    if value > 0.0 { Color::Green }
    else if value < 0.0 { Color::Red }
    else { Color::White }
}

fn level_badge(level: &RiskLevel) -> &'static str {
    match level {
        RiskLevel::Clear    => "● CLEAR",
        RiskLevel::Caution  => "▲ CAUTION",
        RiskLevel::Halt     => "✖ HALT",
        RiskLevel::Cooldown => "⏸ COOLDOWN",
    }
}


fn render_header(f: &mut Frame, area: Rect, executor: &Executor, cycle: u64) {
    let ts = Local::now().format("%Y-%m-%d  %H:%M:%S");
    let pnl = executor.total_pnl();
    let pnl_pct = executor.total_pnl_pct();

    let lines = vec![
        Line::from(vec![
            Span::styled("⚡ RISK GUARD", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("   "),
            Span::styled(format!("Cycle #{cycle}"), Style::default().fg(Color::DarkGray)),
            Span::raw("   "),
            Span::styled(ts.to_string(), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled(
                format!("Portfolio  ${:.2}", executor.portfolio_value),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::raw("     "),
            Span::styled(
                format!("P&L  ${pnl:+.2}  ({pnl_pct:+.2}%)"),
                Style::default().fg(pnl_color(pnl)).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Blue));
    let p = Paragraph::new(Text::from(lines)).block(block);
    f.render_widget(p, area);
}


fn render_risk(f: &mut Frame, area: Rect, verdicts: &HashMap<String, RiskVerdict>) {
    let header_cells = ["Pair", "Status", "Summary"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows: Vec<Row> = verdicts
        .iter()
        .map(|(pair, v)| {
            Row::new(vec![
                Cell::from(pair.as_str()),
                Cell::from(level_badge(&v.level))
                    .style(Style::default().fg(level_color(&v.level)).add_modifier(Modifier::BOLD)),
                Cell::from(v.summary.as_str()).style(Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    let table = Table::new(rows, [Constraint::Length(10), Constraint::Length(14), Constraint::Min(30)])
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Risk Guard Status"));

    f.render_widget(table, area);
}


fn render_market(
    f: &mut Frame,
    area: Rect,
    pairs: &[String],
    tickers: &HashMap<String, Ticker>,
    verdicts: &HashMap<String, RiskVerdict>,
) {
    let header_cells = ["Pair", "Last $", "24h Hi", "24h Lo", "Spread%", "RSI", "Allow%"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows: Vec<Row> = pairs
        .iter()
        .map(|pair| {
            let ticker  = tickers.get(pair.as_str());
            let verdict = verdicts.get(pair.as_str());

            let (last, hi, lo, spread) = ticker
                .map(|t| (t.last, t.high_24h, t.low_24h, t.spread_pct()))
                .unwrap_or((0.0, 0.0, 0.0, 0.0));

            let rsi_str = verdict
                .and_then(|v| v.rsi)
                .map(|r| format!("{r:.1}"))
                .unwrap_or_else(|| "--".to_string());

            let allow_str = verdict
                .map(|v| format!("{:.0}%", v.allowed_position_pct * 100.0))
                .unwrap_or_else(|| "--".to_string());

            Row::new(vec![
                Cell::from(pair.as_str()),
                Cell::from(format!("${last:.4}")),
                Cell::from(format!("${hi:.2}")),
                Cell::from(format!("${lo:.2}")),
                Cell::from(format!("{spread:.3}")),
                Cell::from(rsi_str),
                Cell::from(allow_str),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(10), Constraint::Length(12), Constraint::Length(12),
        Constraint::Length(12), Constraint::Length(9), Constraint::Length(7),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Market Snapshot"));

    f.render_widget(table, area);
}


fn render_positions(
    f: &mut Frame,
    area: Rect,
    executor: &Executor,
    tickers: &HashMap<String, Ticker>,
) {
    let header_cells = ["Pair", "Vol", "Entry $", "Current $", "P&L $", "P&L %", "SL $", "TP $"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows: Vec<Row> = if executor.positions.is_empty() {
        vec![Row::new(vec![
            Cell::from("--"), Cell::from("--"), Cell::from("--"), Cell::from("--"),
            Cell::from("No open positions"), Cell::from("--"), Cell::from("--"), Cell::from("--"),
        ])]
    } else {
        executor.positions.values().map(|pos| {
            let curr = tickers.get(pos.pair.as_str())
                .map(|t| t.last)
                .unwrap_or(pos.entry_price);

            let mut pos_copy = pos.clone();
            pos_copy.current_price = curr;

            let pnl_usd = pos_copy.pnl_usd();
            let pnl_pct = pos_copy.pnl_pct();
            let pc = pnl_color(pnl_usd);

            Row::new(vec![
                Cell::from(pos.pair.as_str()),
                Cell::from(format!("{:.6}", pos.volume)),
                Cell::from(format!("${:.4}", pos.entry_price)),
                Cell::from(format!("${curr:.4}")),
                Cell::from(format!("${pnl_usd:+.2}")).style(Style::default().fg(pc)),
                Cell::from(format!("{pnl_pct:+.2}%")).style(Style::default().fg(pc)),
                Cell::from(pos.stop_loss_price.map(|p| format!("${p:.2}")).unwrap_or("--".to_string())),
                Cell::from(pos.take_profit_price.map(|p| format!("${p:.2}")).unwrap_or("--".to_string())),
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
                .title("Open Positions"),
        );

    f.render_widget(table, area);
}


fn render_trades(f: &mut Frame, area: Rect, executor: &Executor) {
    let header_cells = ["Time", "Pair", "Action", "Volume", "Price $", "Reason", "Result"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let recent = executor.recent_trades(8);
    let rows: Vec<Row> = if recent.is_empty() {
        vec![Row::new(vec![
            Cell::from("--"), Cell::from("--"), Cell::from("--"), Cell::from("--"),
            Cell::from("--"), Cell::from("Waiting for first trade..."), Cell::from("--"),
        ])]
    } else {
        recent.iter().map(|e| {
            let ts = e.timestamp.format("%H:%M:%S").to_string();
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
            let reason = if e.reason.len() > 40 { &e.reason[..40] } else { &e.reason };

            Row::new(vec![
                Cell::from(ts),
                Cell::from(e.pair.as_str()),
                Cell::from(e.action.to_uppercase())
                    .style(Style::default().fg(action_color).add_modifier(Modifier::BOLD)),
                Cell::from(if e.volume > 0.0 { format!("{:.6}", e.volume) } else { "--".to_string() }),
                Cell::from(format!("${:.4}", e.price)),
                Cell::from(reason.to_string()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(e.result.as_str()).style(Style::default().fg(result_color)),
            ])
        }).collect()
    };

    let widths = [
        Constraint::Length(10), Constraint::Length(10), Constraint::Length(8),
        Constraint::Length(12), Constraint::Length(12), Constraint::Min(20),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta))
                .title("Recent Trades"),
        );

    f.render_widget(table, area);
}


pub fn render(
    f: &mut Frame,
    executor: &Executor,
    pairs: &[String],
    tickers: &HashMap<String, Ticker>,
    verdicts: &HashMap<String, RiskVerdict>,
    cycle: u64,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Min(6),
        ])
        .split(f.size());

    render_header(f, chunks[0], executor, cycle);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[1]);

    render_risk(f, mid[0], verdicts);
    render_market(f, mid[1], pairs, tickers, verdicts);
    render_positions(f, chunks[2], executor, tickers);
    render_trades(f, chunks[3], executor);
}