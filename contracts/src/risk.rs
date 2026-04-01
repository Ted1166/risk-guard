use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::warn;

use crate::config::RiskParams;
use crate::indicators::{
    atr, atr_series, rsi, volatility_regime, momentum, DrawdownTracker, VolatilityRegime,
};
use crate::market::{Candle, Ticker};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Clear    = 0,
    Caution  = 1,
    Cooldown = 2,
    Halt     = 3,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Clear    => write!(f, "CLEAR"),
            Self::Caution  => write!(f, "CAUTION"),
            Self::Cooldown => write!(f, "COOLDOWN"),
            Self::Halt     => write!(f, "HALT"),
        }
    }
}


#[derive(Debug, Clone)]
pub struct GuardResult {
    pub name: String,
    pub passed: bool,
    pub level: RiskLevel,
    pub reason: String,
    pub value: Option<f64>,
    pub threshold: Option<f64>,
}


#[derive(Debug, Clone)]
pub struct RiskVerdict {
    pub pair: String,
    pub level: RiskLevel,
    pub guards: Vec<GuardResult>,
    pub allowed_position_pct: f64,
    pub summary: String,
    pub rsi: Option<f64>,
    pub atr_multiple: Option<f64>,
    pub drawdown_pct: f64,
}

impl RiskVerdict {
    pub fn can_trade(&self) -> bool {
        matches!(self.level, RiskLevel::Clear | RiskLevel::Caution)
    }
}

struct PairState {
    atr_history: Vec<f64>,
    cooldown_until: Option<Instant>,
    cooldown_reason: String,
}

impl PairState {
    fn new() -> Self {
        Self {
            atr_history: Vec::new(),
            cooldown_until: None,
            cooldown_reason: String::new(),
        }
    }

    fn in_cooldown(&self) -> bool {
        self.cooldown_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false)
    }

    fn cooldown_remaining_secs(&self) -> u64 {
        self.cooldown_until
            .map(|t| {
                let now = Instant::now();
                if t > now { (t - now).as_secs() } else { 0 }
            })
            .unwrap_or(0)
    }

    fn trigger_cooldown(&mut self, reason: &str, secs: u64) {
        self.cooldown_until = Some(Instant::now() + Duration::from_secs(secs));
        self.cooldown_reason = reason.to_string();
    }
}


pub struct RiskEngine {
    pub params: RiskParams,
    pub dd_tracker: DrawdownTracker,
    pair_states: HashMap<String, PairState>,
}

impl RiskEngine {
    pub fn new(params: RiskParams) -> Self {
        Self {
            params,
            dd_tracker: DrawdownTracker::new(),
            pair_states: HashMap::new(),
        }
    }

    pub fn update_portfolio(&mut self, value: f64) -> f64 {
        self.dd_tracker.update(value)
    }

    pub fn trigger_cooldown(&mut self, pair: &str, reason: &str) {
        let state = self.pair_states.entry(pair.to_string()).or_insert_with(PairState::new);
        state.trigger_cooldown(reason, self.params.cooldown_secs);
        warn!("[{pair}] Cooldown triggered ({}s): {reason}", self.params.cooldown_secs);
    }

    pub fn evaluate(
        &mut self,
        pair: &str,
        candles: &[Candle],
        ticker: &Ticker,
        portfolio_value: f64,
    ) -> RiskVerdict {
        let state = self.pair_states.entry(pair.to_string()).or_insert_with(PairState::new);
        let mut guards: Vec<GuardResult> = Vec::new();
        let p = &self.params;

        if state.in_cooldown() {
            let rem = state.cooldown_remaining_secs();
            guards.push(GuardResult {
                name: "cooldown".to_string(),
                passed: false,
                level: RiskLevel::Cooldown,
                reason: format!("Cooling down for {rem}s after: {}", state.cooldown_reason),
                value: Some(rem as f64),
                threshold: None,
            });
            return build_verdict(pair, guards, 0.0, None, None, 0.0);
        }

        let current_dd = self.dd_tracker.update(portfolio_value);
        if current_dd >= p.max_drawdown_pct {
            guards.push(GuardResult {
                name: "drawdown".to_string(),
                passed: false,
                level: RiskLevel::Halt,
                reason: format!(
                    "Drawdown {:.1}% ≥ hard limit {:.1}%",
                    current_dd * 100.0,
                    p.max_drawdown_pct * 100.0
                ),
                value: Some(current_dd),
                threshold: Some(p.max_drawdown_pct),
            });
            let state = self.pair_states.get_mut(pair).unwrap();
            state.trigger_cooldown(
                &format!("drawdown halt {:.1}%", current_dd * 100.0),
                p.cooldown_secs,
            );
            return build_verdict(pair, guards, 0.0, None, None, current_dd);
        } else if current_dd >= p.drawdown_warning_pct {
            guards.push(GuardResult {
                name: "drawdown".to_string(),
                passed: true,
                level: RiskLevel::Caution,
                reason: format!("Drawdown {:.1}% in warning zone", current_dd * 100.0),
                value: Some(current_dd),
                threshold: Some(p.drawdown_warning_pct),
            });
        } else {
            guards.push(GuardResult {
                name: "drawdown".to_string(),
                passed: true,
                level: RiskLevel::Clear,
                reason: format!("Drawdown {:.1}% within limits", current_dd * 100.0),
                value: Some(current_dd),
                threshold: None,
            });
        }

        let mut atr_multiple: Option<f64> = None;
        let state = self.pair_states.get_mut(pair).unwrap();

        if let Some(current_atr) = atr(candles, p.atr_period) {
            let series = atr_series(candles, p.atr_period);
            if !series.is_empty() {
                state.atr_history = series;
            }

            let (regime, multiple) = volatility_regime(current_atr, &state.atr_history, 20);
            atr_multiple = Some(multiple);

            if multiple >= p.atr_max_multiple || regime == VolatilityRegime::Extreme {
                guards.push(GuardResult {
                    name: "volatility".to_string(),
                    passed: false,
                    level: RiskLevel::Halt,
                    reason: format!(
                        "ATR {multiple:.2}× avg — extreme volatility ({regime})"
                    ),
                    value: Some(multiple),
                    threshold: Some(p.atr_max_multiple),
                });
            } else if multiple >= p.atr_caution_multiple || regime == VolatilityRegime::Elevated {
                guards.push(GuardResult {
                    name: "volatility".to_string(),
                    passed: true,
                    level: RiskLevel::Caution,
                    reason: format!("Elevated volatility — ATR {multiple:.2}× avg"),
                    value: Some(multiple),
                    threshold: Some(p.atr_caution_multiple),
                });
            } else {
                guards.push(GuardResult {
                    name: "volatility".to_string(),
                    passed: true,
                    level: RiskLevel::Clear,
                    reason: format!("Volatility {regime} — ATR {multiple:.2}× avg"),
                    value: Some(multiple),
                    threshold: None,
                });
            }
        } else {
            guards.push(GuardResult {
                name: "volatility".to_string(),
                passed: true,
                level: RiskLevel::Caution,
                reason: "Insufficient candle history for ATR — caution".to_string(),
                value: None,
                threshold: None,
            });
        }

        let rsi_val = rsi(candles, p.rsi_period);
        if let Some(r) = rsi_val {
            if r > p.rsi_overbought {
                guards.push(GuardResult {
                    name: "rsi".to_string(),
                    passed: true,
                    level: RiskLevel::Caution,
                    reason: format!("RSI {r:.1} — overbought, avoid long entries"),
                    value: Some(r),
                    threshold: Some(p.rsi_overbought),
                });
            } else if r < p.rsi_oversold {
                guards.push(GuardResult {
                    name: "rsi".to_string(),
                    passed: true,
                    level: RiskLevel::Caution,
                    reason: format!("RSI {r:.1} — oversold, avoid short entries"),
                    value: Some(r),
                    threshold: Some(p.rsi_oversold),
                });
            } else {
                guards.push(GuardResult {
                    name: "rsi".to_string(),
                    passed: true,
                    level: RiskLevel::Clear,
                    reason: format!("RSI {r:.1} — neutral"),
                    value: Some(r),
                    threshold: None,
                });
            }
        }

        let spread = ticker.spread_pct();
        if spread > 0.5 {
            guards.push(GuardResult {
                name: "spread".to_string(),
                passed: true,
                level: RiskLevel::Caution,
                reason: format!("Wide spread {spread:.3}% — thin liquidity"),
                value: Some(spread),
                threshold: Some(0.5),
            });
        } else {
            guards.push(GuardResult {
                name: "spread".to_string(),
                passed: true,
                level: RiskLevel::Clear,
                reason: format!("Spread {spread:.3}% — liquid"),
                value: Some(spread),
                threshold: None,
            });
        }

        build_verdict(pair, guards, p.max_position_pct, rsi_val, atr_multiple, current_dd)
    }
}


fn build_verdict(
    pair: &str,
    guards: Vec<GuardResult>,
    max_position_pct: f64,
    rsi_val: Option<f64>,
    atr_multiple: Option<f64>,
    drawdown_pct: f64,
) -> RiskVerdict {
    let worst_level = guards
        .iter()
        .map(|g| &g.level)
        .max()
        .cloned()
        .unwrap_or(RiskLevel::Clear);

    let allowed = match worst_level {
        RiskLevel::Halt | RiskLevel::Cooldown => 0.0,
        RiskLevel::Caution => {
            let caution_count = guards
                .iter()
                .filter(|g| g.level == RiskLevel::Caution)
                .count();
            max_position_pct * 0.67f64.powi(caution_count as i32)
        }
        RiskLevel::Clear => max_position_pct,
    };

    let halted: Vec<_> = guards.iter().filter(|g| !g.passed).collect();
    let cautioned: Vec<_> = guards
        .iter()
        .filter(|g| g.passed && g.level == RiskLevel::Caution)
        .collect();

    let summary = match worst_level {
        RiskLevel::Halt => format!(
            "HALT — {}",
            halted.first().map(|g| g.reason.as_str()).unwrap_or("risk limit breached")
        ),
        RiskLevel::Cooldown => format!(
            "COOLDOWN — {}",
            halted.first().map(|g| g.reason.as_str()).unwrap_or("cooling down")
        ),
        RiskLevel::Caution => format!(
            "CAUTION ({} flags) — max position {:.0}%",
            cautioned.len(),
            allowed * 100.0
        ),
        RiskLevel::Clear => format!("CLEAR — all {} guards green", guards.len()),
    };

    RiskVerdict {
        pair: pair.to_string(),
        level: worst_level,
        guards,
        allowed_position_pct: (allowed * 10000.0).round() / 10000.0,
        summary,
        rsi: rsi_val,
        atr_multiple,
        drawdown_pct,
    }
}