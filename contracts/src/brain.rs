use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::config::AppConfig;
use crate::indicators::{momentum, rsi};
use crate::market::{Candle, Ticker};
use crate::risk::{RiskLevel, RiskVerdict};


#[derive(Debug, Clone)]
pub enum Action {
    Buy,
    Sell,
    Hold,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buy  => write!(f, "BUY"),
            Self::Sell => write!(f, "SELL"),
            Self::Hold => write!(f, "HOLD"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradeDecision {
    pub action: Action,
    pub pair: String,
    pub size_pct: f64,
    pub confidence: String,
    pub stop_loss_pct: Option<f64>,
    pub take_profit_pct: Option<f64>,
    pub reasoning: String,
}


#[derive(Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ClaudeMessage>,
}

#[derive(Serialize)]
struct ClaudeMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
}

#[derive(Deserialize)]
struct ClaudeContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct DecisionJson {
    action: String,
    size_pct: f64,
    confidence: String,
    stop_loss_pct: Option<f64>,
    take_profit_pct: Option<f64>,
    reasoning: String,
}


fn build_prompt(
    pair: &str,
    ticker: &Ticker,
    candles: &[Candle],
    verdict: &RiskVerdict,
    portfolio_value: f64,
    open_position_pct: f64,
) -> String {
    let rsi_str = rsi(candles, 14)
        .map(|r| format!("{r:.1}"))
        .unwrap_or_else(|| "N/A".to_string());

    let mom_str = momentum(candles, 10)
        .map(|m| format!("{m:+.2}%"))
        .unwrap_or_else(|| "N/A".to_string());

    let recent = &candles[candles.len().saturating_sub(5)..];
    let candle_lines: Vec<String> = recent
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                "  [{}] O:{:.2} H:{:.2} L:{:.2} C:{:.2} V:{:.2}",
                i + 1, c.open, c.high, c.low, c.close, c.volume
            )
        })
        .collect();

    let guard_lines: Vec<String> = verdict
        .guards
        .iter()
        .map(|g| {
            let mark = if g.passed { "✓" } else { "✗" };
            format!("  {mark} {}: [{}] {}", g.name, g.level, g.reason)
        })
        .collect();

    format!(
        r#"=== RISK GUARD — TRADE DECISION REQUEST ===
Pair: {pair}
Portfolio: ${portfolio_value:.2}
Open position: {:.1}% of portfolio
Risk allowance: {:.1}% max position

--- Market Snapshot ---
Last:    ${:.4}
Bid/Ask: ${:.4} / ${:.4}
24h Hi:  ${:.2}  |  24h Lo: ${:.2}
RSI(14): {rsi_str}
Mom(10): {mom_str}

--- Recent Candles (hourly, newest last) ---
{}

--- Risk Status: {} ---
{}

You are a disciplined quant risk manager. PRIMARY goal: capital preservation.
Respond ONLY with a JSON object — no markdown, no preamble:
{{
  "action": "buy" | "sell" | "hold",
  "size_pct": 0.0-1.0,
  "confidence": "high" | "medium" | "low",
  "stop_loss_pct": number | null,
  "take_profit_pct": number | null,
  "reasoning": "one sentence"
}}
Rules:
- size_pct is fraction of the allowed position ({:.1}% max × size_pct = actual size)
- stop_loss_pct: % below entry to cut loss (e.g. 2.0 = 2% stop)
- take_profit_pct: % above entry to take profit
- CAUTION status → prefer hold or size_pct ≤ 0.5
- Never exceed the risk allowance. Prefer hold over uncertain entries."#,
        open_position_pct * 100.0,
        verdict.allowed_position_pct * 100.0,
        ticker.last,
        ticker.bid,
        ticker.ask,
        ticker.high_24h,
        ticker.low_24h,
        candle_lines.join("\n"),
        verdict.level,
        guard_lines.join("\n"),
        verdict.allowed_position_pct * 100.0,
    )
}


async fn call_claude(client: &reqwest::Client, cfg: &AppConfig, prompt: String) -> Result<String> {
    let req = ClaudeRequest {
        model: cfg.claude_model.clone(),
        max_tokens: 512,
        messages: vec![ClaudeMessage {
            role: "user".to_string(),
            content: prompt,
        }],
    };

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("anthropic-version", "2023-06-01")
        .json(&req)
        .send()
        .await
        .context("Claude API request failed")?
        .json::<ClaudeResponse>()
        .await
        .context("Failed to parse Claude response")?;

    resp.content
        .into_iter()
        .find(|c| c.content_type == "text")
        .and_then(|c| c.text)
        .ok_or_else(|| anyhow::anyhow!("No text in Claude response"))
}


pub async fn get_trade_decision(
    client: &reqwest::Client,
    cfg: &AppConfig,
    pair: &str,
    ticker: &Ticker,
    candles: &[Candle],
    verdict: &RiskVerdict,
    portfolio_value: f64,
    open_position_pct: f64,
) -> TradeDecision {
    if !cfg.ai_enabled {
        return rule_based_decision(pair, candles, verdict);
    }

    let prompt = build_prompt(pair, ticker, candles, verdict, portfolio_value, open_position_pct);

    match call_claude(client, cfg, prompt).await {
        Ok(raw) => {
            let clean = raw
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();

            match serde_json::from_str::<DecisionJson>(clean) {
                Ok(d) => {
                    let action = match d.action.to_lowercase().as_str() {
                        "buy"  => Action::Buy,
                        "sell" => Action::Sell,
                        _      => Action::Hold,
                    };
                    info!(
                        "[{pair}] AI → {} ({}) — {}",
                        d.action, d.confidence, d.reasoning
                    );
                    TradeDecision {
                        action,
                        pair: pair.to_string(),
                        size_pct: d.size_pct.clamp(0.0, 1.0),
                        confidence: d.confidence,
                        stop_loss_pct: d.stop_loss_pct,
                        take_profit_pct: d.take_profit_pct,
                        reasoning: d.reasoning,
                    }
                }
                Err(e) => {
                    error!("[{pair}] Failed to parse AI response: {e}. Raw: {clean}");
                    rule_based_decision(pair, candles, verdict)
                }
            }
        }
        Err(e) => {
            error!("[{pair}] Claude API error: {e} — falling back to rules");
            rule_based_decision(pair, candles, verdict)
        }
    }
}


fn rule_based_decision(pair: &str, candles: &[Candle], verdict: &RiskVerdict) -> TradeDecision {
    if !verdict.can_trade() {
        return TradeDecision {
            action: Action::Hold,
            pair: pair.to_string(),
            size_pct: 0.0,
            confidence: "high".to_string(),
            stop_loss_pct: None,
            take_profit_pct: None,
            reasoning: format!("Risk guard: {}", verdict.summary),
        };
    }

    let rsi_val = rsi(candles, 14);
    let mom_val = momentum(candles, 10);

    match (rsi_val, mom_val) {
        (Some(r), Some(m)) if r < 50.0 && m > 1.0 => TradeDecision {
            action: Action::Buy,
            pair: pair.to_string(),
            size_pct: 0.5,
            confidence: "medium".to_string(),
            stop_loss_pct: Some(2.0),
            take_profit_pct: Some(4.0),
            reasoning: format!("RSI {r:.1} + momentum {m:+.1}% — mild long signal"),
        },
        (Some(r), Some(m)) if r > 65.0 && m < -1.0 => TradeDecision {
            action: Action::Sell,
            pair: pair.to_string(),
            size_pct: 0.5,
            confidence: "medium".to_string(),
            stop_loss_pct: None,
            take_profit_pct: None,
            reasoning: format!("RSI {r:.1} overbought + negative momentum {m:+.1}%"),
        },
        _ => TradeDecision {
            action: Action::Hold,
            pair: pair.to_string(),
            size_pct: 0.0,
            confidence: "low".to_string(),
            stop_loss_pct: None,
            take_profit_pct: None,
            reasoning: "No clear signal — hold".to_string(),
        },
    }
}