use std::collections::HashMap;
use tracing::{info, warn};
use chrono::{DateTime, Local};

use crate::brain::{Action, TradeDecision};
use crate::config::AppConfig;
use crate::market::{self, Ticker};
use crate::risk::RiskEngine;

// ── Position ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Position {
    pub pair: String,
    pub side: String,
    pub volume: f64,
    pub entry_price: f64,
    pub entry_time: DateTime<Local>,
    pub stop_loss_price: Option<f64>,
    pub take_profit_price: Option<f64>,
    pub current_price: f64,
}

impl Position {
    pub fn pnl_usd(&self) -> f64 {
        (self.current_price - self.entry_price) * self.volume
    }
    pub fn pnl_pct(&self) -> f64 {
        if self.entry_price == 0.0 { return 0.0; }
        ((self.current_price - self.entry_price) / self.entry_price) * 100.0
    }
}


#[derive(Debug, Clone)]
pub struct ExecLog {
    pub timestamp: DateTime<Local>,
    pub pair: String,
    pub action: String,
    pub volume: f64,
    pub price: f64,
    pub reason: String,
    pub result: String,
    pub error: String,
}


pub struct Executor {
    pub positions: HashMap<String, Position>,
    pub exec_log: Vec<ExecLog>,
    pub portfolio_value: f64,
    pub initial_balance: f64,
    initialized: bool,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
            exec_log: Vec::new(),
            portfolio_value: 0.0,
            initial_balance: 0.0,
            initialized: false,
        }
    }

    pub fn initialize(&mut self, cfg: &AppConfig) -> anyhow::Result<()> {
        market::paper_init(&cfg.kraken_cli, cfg.paper_initial_balance, &cfg.paper_currency)?;
        self.initial_balance = cfg.paper_initial_balance;
        self.portfolio_value = cfg.paper_initial_balance;
        self.initialized = true;
        info!("Paper account initialized: ${:.2}", cfg.paper_initial_balance);
        Ok(())
    }

    pub fn refresh_portfolio(&mut self, cfg: &AppConfig) -> f64 {
        match market::paper_status(&cfg.kraken_cli) {
            Ok(s) if s.portfolio_value > 0.0 => {
                self.portfolio_value = s.portfolio_value;
            }
            Ok(_) => {}
            Err(e) => warn!("Could not refresh portfolio: {e}"),
        }
        self.portfolio_value
    }

    pub fn total_pnl(&self) -> f64 {
        self.portfolio_value - self.initial_balance
    }

    pub fn total_pnl_pct(&self) -> f64 {
        if self.initial_balance == 0.0 { return 0.0; }
        (self.total_pnl() / self.initial_balance) * 100.0
    }


    pub fn check_exits(
        &mut self,
        cfg: &AppConfig,
        risk: &mut RiskEngine,
        pair: &str,
        current_price: f64,
    ) -> Option<ExecLog> {
        let pos = self.positions.get_mut(pair)?;
        pos.current_price = current_price;

        if let Some(sl) = pos.stop_loss_price {
            if current_price <= sl {
                warn!("[{pair}] STOP LOSS @ ${current_price:.4} (SL: ${sl:.4})");
                risk.trigger_cooldown(pair, "stop-loss triggered");
                return Some(self.close_position(cfg, pair, current_price, "Stop-loss triggered"));
            }
        }

        if let Some(tp) = pos.take_profit_price {
            if current_price >= tp {
                info!("[{pair}] TAKE PROFIT @ ${current_price:.4} (TP: ${tp:.4})");
                return Some(self.close_position(cfg, pair, current_price, "Take-profit triggered"));
            }
        }

        None
    }


    pub fn execute(
        &mut self,
        cfg: &AppConfig,
        decision: &TradeDecision,
        ticker: &Ticker,
    ) -> ExecLog {
        let pair = &decision.pair;
        let current_price = ticker.last;

        match &decision.action {
            Action::Hold => {
                let entry = self.make_log(pair, "hold", 0.0, current_price, &decision.reasoning, "skipped", "");
                self.push_log(entry.clone());
                return entry;
            }

            Action::Buy => {
                if self.positions.contains_key(pair.as_str()) {
                    let entry = self.make_log(pair, "buy", 0.0, current_price,
                        "Already have an open position", "skipped", "");
                    self.push_log(entry.clone());
                    return entry;
                }

                let allowed_usd = self.portfolio_value * decision.size_pct * cfg.risk.max_position_pct;
                if allowed_usd < 10.0 {
                    let entry = self.make_log(pair, "buy", 0.0, current_price,
                        &format!("Position size too small: ${allowed_usd:.2}"), "skipped", "");
                    self.push_log(entry.clone());
                    return entry;
                }

                let volume = (allowed_usd / current_price * 1_000_000.0).round() / 1_000_000.0;

                match market::paper_buy(&cfg.kraken_cli, pair, volume) {
                    Ok(fill) => {
                        let fill_price = if fill.price > 0.0 { fill.price } else { current_price };
                        let sl = decision.stop_loss_pct
                            .map(|pct| fill_price * (1.0 - pct / 100.0));
                        let tp = decision.take_profit_pct
                            .map(|pct| fill_price * (1.0 + pct / 100.0));

                        self.positions.insert(pair.clone(), Position {
                            pair: pair.clone(),
                            side: "long".to_string(),
                            volume,
                            entry_price: fill_price,
                            entry_time: Local::now(),
                            stop_loss_price: sl,
                            take_profit_price: tp,
                            current_price: fill_price,
                        });

                        info!(
                            "[{pair}] BUY {volume:.6} @ ${fill_price:.4} \
                             SL:{} TP:{} — {}",
                            sl.map(|p| format!("${p:.2}")).unwrap_or("none".to_string()),
                            tp.map(|p| format!("${p:.2}")).unwrap_or("none".to_string()),
                            decision.reasoning,
                        );

                        let entry = self.make_log(pair, "buy", volume, fill_price,
                            &decision.reasoning, "ok", "");
                        self.push_log(entry.clone());
                        entry
                    }
                    Err(e) => {
                        let entry = self.make_log(pair, "buy", volume, current_price,
                            &decision.reasoning, "error", &e.to_string());
                        self.push_log(entry.clone());
                        entry
                    }
                }
            }

            Action::Sell => {
                if !self.positions.contains_key(pair.as_str()) {
                    let entry = self.make_log(pair, "sell", 0.0, current_price,
                        "No open position to sell", "skipped", "");
                    self.push_log(entry.clone());
                    return entry;
                }
                self.close_position(cfg, pair, current_price, &decision.reasoning)
            }
        }
    }


    fn close_position(
        &mut self,
        cfg: &AppConfig,
        pair: &str,
        current_price: f64,
        reason: &str,
    ) -> ExecLog {
        let volume = match self.positions.get(pair) {
            Some(p) => p.volume,
            None => {
                let entry = self.make_log(pair, "sell", 0.0, current_price,
                    reason, "skipped", "no position found");
                self.push_log(entry.clone());
                return entry;
            }
        };

        match market::paper_sell(&cfg.kraken_cli, pair, volume) {
            Ok(fill) => {
                let fill_price = if fill.price > 0.0 { fill.price } else { current_price };
                if let Some(pos) = self.positions.remove(pair) {
                    let pnl = (fill_price - pos.entry_price) * volume;
                    info!(
                        "[{pair}] SELL {volume:.6} @ ${fill_price:.4} \
                         P&L: ${pnl:+.2} — {reason}"
                    );
                }
                let entry = self.make_log(pair, "sell", volume, fill_price, reason, "ok", "");
                self.push_log(entry.clone());
                entry
            }
            Err(e) => {
                let entry = self.make_log(pair, "sell", volume, current_price,
                    reason, "error", &e.to_string());
                self.push_log(entry.clone());
                entry
            }
        }
    }


    fn make_log(
        &self,
        pair: &str,
        action: &str,
        volume: f64,
        price: f64,
        reason: &str,
        result: &str,
        error: &str,
    ) -> ExecLog {
        ExecLog {
            timestamp: Local::now(),
            pair: pair.to_string(),
            action: action.to_string(),
            volume,
            price,
            reason: reason.to_string(),
            result: result.to_string(),
            error: error.to_string(),
        }
    }

    fn push_log(&mut self, entry: ExecLog) {
        self.exec_log.push(entry);
        if self.exec_log.len() > 500 {
            self.exec_log.drain(0..100);
        }
    }

    pub fn recent_trades(&self, n: usize) -> Vec<&ExecLog> {
        self.exec_log
            .iter()
            .filter(|e| e.action != "hold")
            .rev()
            .take(n)
            .collect()
    }
}