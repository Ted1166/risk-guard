mod config;
mod market;
mod indicators;
mod risk;
mod brain;
mod executor;
mod dashboard;
mod social;

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
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use config::AppConfig;
use risk::RiskEngine;
use executor::Executor;

const REPORT_EVERY: u64 = 10;

#[tokio::main]
async fn main() -> Result<()> {
    // Logging
    std::fs::create_dir_all("logs")?;
    let file_appender = tracing_appender::rolling::daily("logs", "risk_guard.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("risk_guard=info".parse()?),
        )
        .with_writer(non_blocking)
        .init();

    let cfg = AppConfig::default();

    info!("═══════════════════════════════════════════════");
    info!("  RISK GUARD — Starting up");
    info!("  Pairs:         {:?}", cfg.watch_pairs);
    info!("  Balance:       ${:.2}", cfg.paper_initial_balance);
    info!("  Poll interval: {}s", cfg.poll_interval_secs);
    info!("  AI brain:      {}", if cfg.ai_enabled { "enabled" } else { "rule-based" });
    info!("  Dead man's:    re-arm every {}s", cfg.risk.cancel_after_secs);
    info!("═══════════════════════════════════════════════");

    // Verify Kraken CLI is reachable
    match market::system_status(&cfg.kraken_cli) {
        Ok(s) => info!("Kraken system status: {}", s.status),
        Err(e) => {
            eprintln!("❌  Cannot reach Kraken CLI: {e}");
            eprintln!("    Install: curl -LsSf https://github.com/krakenfx/kraken-cli/releases/\
                       latest/download/kraken-cli-installer.sh | sh");
            std::process::exit(1);
        }
    }

    // Subsystems
    let mut risk_engine = RiskEngine::new(cfg.risk.clone());
    let mut executor    = Executor::new();

    executor.initialize(&cfg)?;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    // Ctrl-C shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let tx2 = shutdown_tx.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            info!("Ctrl-C received — shutting down cleanly");
            let _ = tx2.send(true);
        }
    });

    // Terminal setup
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend  = CrosstermBackend::new(out);
    let mut term = Terminal::new(backend)?;

    let pairs   = cfg.watch_pairs.clone();
    let mut tickers:       HashMap<String, market::Ticker>    = HashMap::new();
    let mut verdicts:      HashMap<String, risk::RiskVerdict> = HashMap::new();
    let mut price_history: HashMap<String, Vec<f64>>          = HashMap::new();
    let mut _current_drawdown: f64 = 0.0;
    let mut cycle: u64 = 0;

    for pair in &pairs {
        price_history.insert(pair.clone(), Vec::with_capacity(60));
    }

    // Main agent loop
    'main: loop {
        if *shutdown_rx.borrow() { break; }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(k) = event::read()? {
                if k.code == KeyCode::Char('q')
                    || k.code == KeyCode::Esc
                    || (k.code == KeyCode::Char('c')
                        && k.modifiers.contains(KeyModifiers::CONTROL))
                {
                    break;
                }
            }
        }

        cycle += 1;
        info!("──────────────── Cycle {cycle} ────────────────");

        // 1. Dead man's switch — re-arm FIRST, before any work
        if let Err(e) = market::arm_dead_mans_switch(
            &cfg.kraken_cli,
            cfg.risk.cancel_after_secs,
        ) {
            warn!("Dead man's switch arm failed: {e}");
        } else {
            info!("Dead man's switch armed ({}s)", cfg.risk.cancel_after_secs);
        }

        // 2. Refresh portfolio value
        let portfolio_value = executor.refresh_portfolio(&cfg);
        risk_engine.update_portfolio(portfolio_value);

        // 3. Per-pair: fetch → risk → exits → decide → execute
        for pair in &pairs {
            if *shutdown_rx.borrow() { break 'main; }

            // Fetch ticker
            let ticker = match market::get_ticker(&cfg.kraken_cli, pair) {
                Ok(t) => t,
                Err(e) => {
                    warn!("[{pair}] Ticker fetch failed: {e}");
                    continue;
                }
            };

            // Fetch OHLC candles
            let candles = match market::get_ohlc(
                &cfg.kraken_cli,
                pair,
                cfg.ohlc_interval_mins,
                cfg.ohlc_history,
            ) {
                Ok(c) => c,
                Err(e) => {
                    warn!("[{pair}] OHLC fetch failed: {e}");
                    continue;
                }
            };

            // Risk evaluation
            let verdict =
                risk_engine.evaluate(pair, &candles, &ticker, portfolio_value);
            info!("[{pair}] Risk: {} — {}", verdict.level, verdict.summary);

            // SL/TP monitoring — check before deciding on new entries
            let current_price = ticker.last;
            if let Some(exit) =
                executor.check_exits(&cfg, &mut risk_engine, pair, current_price)
            {
                if exit.result == "ok" {
                    info!("[{pair}] Auto-exit: {} @ ${:.4}", exit.reason, exit.price);
                } else if exit.result == "error" {
                    error!("[{pair}] Exit failed: {}", exit.error);
                }
            }

            // Trade decision + execution
            if verdict.can_trade() {
                let open_pos_pct = executor
                    .positions
                    .get(pair.as_str())
                    .map(|p| (p.volume * current_price) / portfolio_value)
                    .unwrap_or(0.0);

                let decision = brain::get_trade_decision(
                    &http,
                    &cfg,
                    pair,
                    &ticker,
                    &candles,
                    &verdict,
                    portfolio_value,
                    open_pos_pct,
                )
                .await;

                info!(
                    "[{pair}] Decision: {} | confidence: {} | {}",
                    decision.action, decision.confidence, decision.reasoning
                );

                let log = executor.execute(&cfg, &decision, &ticker);

                match log.result.as_str() {
                    "ok" => info!(
                        "[{pair}] ✓ {} {:.6} @ ${:.4}",
                        log.action, log.volume, log.price
                    ),
                    "error" => error!(
                        "[{pair}] ✗ {} failed: {}",
                        log.action, log.error
                    ),
                    _ => {} // skipped
                }
            } else {
                info!("[{pair}] Skipping trade — {}", verdict.summary);
            }

            // Update sparkline history (keep last 60 points)
            let hist = price_history.entry(pair.clone()).or_default();
            hist.push(ticker.last);
            if hist.len() > 60 { hist.remove(0); }

            tickers.insert(pair.clone(), ticker);
            verdicts.insert(pair.clone(), verdict);
        }

        // 4. Refresh portfolio again after executions
        let portfolio_value = executor.refresh_portfolio(&cfg);
        _current_drawdown = risk_engine.dd_tracker.current_drawdown();

        // 5. Social log — one-liner every cycle, full report every N
        let summary = social::build_summary(cycle, &executor, &verdicts);
        info!("{}", social::one_liner(&summary));

        if cycle % REPORT_EVERY == 0 {
            match social::save_report(&summary, &executor, &verdicts) {
                Ok(path) => info!("📄 Report saved: {path}"),
                Err(e)   => warn!("Report save failed: {e}"),
            }
        }

        // 6. Render dashboard
        term.draw(|f| {
            dashboard::render(f, &executor, &pairs, &tickers, &verdicts, &price_history, cycle, _current_drawdown);
        })?;

        info!(
            "Cycle {cycle} complete | Portfolio: ${:.2} | P&L: ${:+.2} ({:+.2}%) | \
             Next in {}s",
            portfolio_value,
            executor.total_pnl(),
            executor.total_pnl_pct(),
            cfg.poll_interval_secs,
        );

        // 7. Interruptible sleep
        for _ in 0..cfg.poll_interval_secs {
            if *shutdown_rx.borrow() {
                break 'main;
            }
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(k) = event::read()? {
                    if k.code == KeyCode::Char('q') || k.code == KeyCode::Esc {
                        break 'main;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    // Shutdown 
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;

    let summary = social::build_summary(cycle, &executor, &verdicts);
    match social::save_report(&summary, &executor, &verdicts) {
        Ok(path) => println!("📄 Final report: {path}"),
        Err(e)   => eprintln!("Report save failed: {e}"),
    }

    println!("\n{}", social::one_liner(&summary));
    println!(
        "\nRisk Guard stopped after {cycle} cycles.\n\
         Final P&L: ${:+.2} ({:+.2}%)",
        executor.total_pnl(),
        executor.total_pnl_pct(),
    );

    Ok(())
}