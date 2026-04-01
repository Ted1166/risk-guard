mod config;
mod market;
mod indicators;
mod risk;
mod brain;
mod executor;
mod dashboard;

use std::collections::HashMap;
use std::io::stdout;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    event::{self, Event, KeyCode, KeyModifiers},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::watch;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use config::AppConfig;
use risk::RiskEngine;
use executor::Executor;

#[tokio::main]
async fn main() -> Result<()> {
    // Logging
    std::fs::create_dir_all("logs")?;
    let file_appender = tracing_appender::rolling::daily("logs", "risk_guard.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("risk_guard=info".parse()?))
        .with_writer(non_blocking)
        .init();

    let cfg = AppConfig::default();

    info!("═══════════════════════════════════════");
    info!("  RISK GUARD — Starting");
    info!("  Pairs:   {:?}", cfg.watch_pairs);
    info!("  Balance: ${:.2}", cfg.paper_initial_balance);
    info!("  AI:      {}", if cfg.ai_enabled { "enabled" } else { "rule-based" });
    info!("═══════════════════════════════════════");

    match market::system_status(&cfg.kraken_cli) {
        Ok(s) => info!("Kraken status: {}", s.status),
        Err(e) => {
            eprintln!("Cannot reach Kraken CLI: {e}");
            eprintln!("Install: curl -LsSf https://github.com/krakenfx/kraken-cli/releases/latest/download/kraken-cli-installer.sh | sh");
            std::process::exit(1);
        }
    }

    let mut risk_engine = RiskEngine::new(cfg.risk.clone());
    let mut executor    = Executor::new();
    executor.initialize(&cfg)?;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let tx2 = shutdown_tx.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = tx2.send(true);
        }
    });

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend  = CrosstermBackend::new(out);
    let mut term = Terminal::new(backend)?;

    let pairs   = cfg.watch_pairs.clone();
    let mut tickers:  HashMap<String, market::Ticker>    = HashMap::new();
    let mut verdicts: HashMap<String, risk::RiskVerdict> = HashMap::new();
    let mut cycle: u64 = 0;

    'main: loop {
        if *shutdown_rx.borrow() { break; }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(k) = event::read()? {
                if k.code == KeyCode::Char('q')
                    || k.code == KeyCode::Esc
                    || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
                {
                    break;
                }
            }
        }

        cycle += 1;
        info!("─── Cycle {cycle} ───");

        let portfolio_value = executor.refresh_portfolio(&cfg);
        risk_engine.update_portfolio(portfolio_value);

        for pair in &pairs {
            let ticker = match market::get_ticker(&cfg.kraken_cli, pair) {
                Ok(t) => t,
                Err(e) => { warn!("[{pair}] Ticker error: {e}"); continue; }
            };
            let candles = match market::get_ohlc(
                &cfg.kraken_cli, pair, cfg.ohlc_interval_mins, cfg.ohlc_history
            ) {
                Ok(c) => c,
                Err(e) => { warn!("[{pair}] OHLC error: {e}"); continue; }
            };

            let verdict = risk_engine.evaluate(pair, &candles, &ticker, portfolio_value);
            info!("[{pair}] Risk: {} — {}", verdict.level, verdict.summary);

            if let Some(exit) = executor.check_exits(&cfg, &mut risk_engine, pair, ticker.last) {
                if exit.result == "ok" {
                    info!("[{pair}] Exit: {}", exit.reason);
                }
            }

            if verdict.can_trade() {
                let open_pos_pct = executor.positions.get(pair.as_str())
                    .map(|p| (p.volume * ticker.last) / portfolio_value)
                    .unwrap_or(0.0);

                let decision = brain::get_trade_decision(
                    &http, &cfg, pair, &ticker, &candles,
                    &verdict, portfolio_value, open_pos_pct,
                ).await;

                info!("[{pair}] Decision: {} ({}) — {}", decision.action, decision.confidence, decision.reasoning);
                let log = executor.execute(&cfg, &decision, &ticker);
                if log.result == "ok" {
                    info!("[{pair}] Executed {} {:.6} @ ${:.4}", log.action, log.volume, log.price);
                }
            } else {
                info!("[{pair}] Skipping — {}", verdict.summary);
            }

            tickers.insert(pair.clone(), ticker);
            verdicts.insert(pair.clone(), verdict);
        }

        term.draw(|f| {
            dashboard::render(f, &executor, &pairs, &tickers, &verdicts, cycle);
        })?;

        info!("Cycle {cycle} done. Portfolio: ${:.2} (P&L: ${:+.2}). Sleeping {}s…",
              portfolio_value, executor.total_pnl(), cfg.poll_interval_secs);

        for _ in 0..cfg.poll_interval_secs {
            if *shutdown_rx.borrow() { break 'main; }
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(k) = event::read()? {
                    if k.code == KeyCode::Char('q') || k.code == KeyCode::Esc {
                        break 'main;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;

    println!("\nRisk Guard stopped. Final P&L: ${:+.2} ({:+.2}%)",
             executor.total_pnl(), executor.total_pnl_pct());
    Ok(())
}