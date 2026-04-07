use anyhow::{anyhow, bail, Context};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Command;
use tracing::debug;


fn run_kraken(cli: &str, args: &[&str]) -> anyhow::Result<Value> {
    debug!("kraken {}", args.join(" "));

    let output = Command::new(cli)
        .args(args)
        .args(["-o", "json"])
        .output()
        .with_context(|| {
            format!(
                "Failed to run '{cli}'. Install: \
                 curl -LsSf https://github.com/krakenfx/kraken-cli/releases/latest/\
                 download/kraken-cli-installer.sh | sh"
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw = stdout.trim();

    if raw.is_empty() {
        bail!("Empty response from kraken CLI (exit {})", output.status.code().unwrap_or(-1));
    }

    let val: Value = serde_json::from_str(raw)
        .with_context(|| format!("Non-JSON output: {}", &raw[..raw.len().min(200)]))?;

    if let Some(err_cat) = val.get("error").and_then(|v| v.as_str()) {
        let msg = val
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        bail!("[{err_cat}] {msg}");
    }

    Ok(val)
}


#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Ticker {
    pub pair: String,
    pub ask: f64,
    pub bid: f64,
    pub last: f64,
    pub volume_24h: f64,
    pub high_24h: f64,
    pub low_24h: f64,
    pub vwap_24h: f64,
}

impl Ticker {
    pub fn spread_pct(&self) -> f64 {
        let mid = (self.ask + self.bid) / 2.0;
        if mid == 0.0 { return 0.0; }
        ((self.ask - self.bid) / mid) * 100.0
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Candle {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PaperStatus {
    pub portfolio_value: f64,
    pub unrealized_pnl: f64,
    pub trade_count: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PaperFill {
    pub pair: String,
    pub side: String,
    pub volume: f64,
    pub price: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct SystemStatus {
    pub status: String,
    pub timestamp: Option<String>,
}


pub fn system_status(cli: &str) -> anyhow::Result<SystemStatus> {
    let val = run_kraken(cli, &["status"])?;
    let status: SystemStatus = serde_json::from_value(val)
        .context("Failed to parse system status")?;
    Ok(status)
}

pub fn get_ticker(cli: &str, pair: &str) -> anyhow::Result<Ticker> {
    let val = run_kraken(cli, &["ticker", pair])?;

    let obj = val
        .as_object()
        .ok_or_else(|| anyhow!("Expected object for ticker {pair}"))?;

    let (key, data) = obj
        .iter()
        .next()
        .ok_or_else(|| anyhow!("Empty ticker response for {pair}"))?;

    fn pick(v: &Value, field: &str, idx: usize) -> f64 {
        v.get(field)
            .and_then(|a| a.get(idx))
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0)
    }
    fn pick_arr(v: &Value, field: &str, idx: usize) -> f64 {
        v.get(field)
            .and_then(|a| a.get(idx))
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0)
    }

    Ok(Ticker {
        pair: key.clone(),
        ask:      pick(data, "a", 0),
        bid:      pick(data, "b", 0),
        last:     pick(data, "c", 0),
        volume_24h: pick_arr(data, "v", 1),
        high_24h: pick_arr(data, "h", 1),
        low_24h:  pick_arr(data, "l", 1),
        vwap_24h: pick_arr(data, "p", 1),
    })
}

pub fn get_ohlc(
    cli: &str,
    pair: &str,
    interval_mins: u32,
    count: usize,
) -> anyhow::Result<Vec<Candle>> {
    let interval_str = interval_mins.to_string();
    let val = run_kraken(cli, &["ohlc", pair, "--interval", &interval_str])?;

    let obj = val.as_object().ok_or_else(|| anyhow!("Expected object for OHLC"))?;

    let candle_arr = obj
        .values()
        .find(|v| v.is_array())
        .ok_or_else(|| anyhow!("No candle array in OHLC response for {pair}"))?;

    let raw_candles = candle_arr
        .as_array()
        .ok_or_else(|| anyhow!("OHLC candle list is not an array"))?;

    let mut candles: Vec<Candle> = raw_candles
        .iter()
        .filter_map(|c| {
            let arr = c.as_array()?;
            Some(Candle {
                time:   arr.get(0)?.as_i64()?,
                open:   arr.get(1)?.as_str()?.parse().ok()?,
                high:   arr.get(2)?.as_str()?.parse().ok()?,
                low:    arr.get(3)?.as_str()?.parse().ok()?,
                close:  arr.get(4)?.as_str()?.parse().ok()?,
                volume: arr.get(6)?.as_str()?.parse().ok()?,
            })
        })
        .collect();

    if candles.len() > count {
        let skip = candles.len() - count;
        candles = candles.into_iter().skip(skip).collect();
    }

    Ok(candles)
}


pub fn paper_init(cli: &str, balance: f64, currency: &str) -> anyhow::Result<()> {
    let bal_str = format!("{:.2}", balance);
    run_kraken(cli, &[
        "paper", "init",
        "--balance", &bal_str,
        "--currency", currency,
    ])?;
    Ok(())
}

pub fn paper_status(cli: &str) -> anyhow::Result<PaperStatus> {
    let val = run_kraken(cli, &["paper", "status"])?;

    let portfolio_value = val
        .get("portfolio_value")
        .or_else(|| val.get("total_value"))
        .or_else(|| val.get("equity"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let unrealized_pnl = val
        .get("unrealized_pnl")
        .or_else(|| val.get("pnl"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let trade_count = val
        .get("trade_count")
        .or_else(|| val.get("trades"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    Ok(PaperStatus { portfolio_value, unrealized_pnl, trade_count })
}

#[allow(dead_code)]
pub fn paper_balance(cli: &str) -> anyhow::Result<HashMap<String, f64>> {
    let val = run_kraken(cli, &["paper", "balance"])?;
    let mut result = HashMap::new();
    if let Some(obj) = val.as_object() {
        for (k, v) in obj {
            if let Some(f) = v.as_f64() {
                result.insert(k.clone(), f);
            } else if let Some(s) = v.as_str() {
                if let Ok(f) = s.parse::<f64>() {
                    result.insert(k.clone(), f);
                }
            }
        }
    }
    Ok(result)
}

pub fn paper_buy(
    cli: &str,
    pair: &str,
    volume: f64,
) -> anyhow::Result<PaperFill> {
    let vol_str = format!("{:.8}", volume);
    let val = run_kraken(cli, &["paper", "buy", pair, &vol_str])?;
    let price = val
        .get("price")
        .and_then(|v| v.as_f64())
        .or_else(|| val.get("fill_price").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
    Ok(PaperFill { pair: pair.to_string(), side: "buy".to_string(), volume, price })
}

pub fn paper_sell(
    cli: &str,
    pair: &str,
    volume: f64,
) -> anyhow::Result<PaperFill> {
    let vol_str = format!("{:.8}", volume);
    let val = run_kraken(cli, &["paper", "sell", pair, &vol_str])?;
    let price = val
        .get("price")
        .and_then(|v| v.as_f64())
        .or_else(|| val.get("fill_price").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
    Ok(PaperFill { pair: pair.to_string(), side: "sell".to_string(), volume, price })
}

#[allow(dead_code)]
pub fn paper_reset(cli: &str) -> anyhow::Result<()> {
    run_kraken(cli, &["paper", "reset"])?;
    Ok(())
}

#[allow(dead_code)]
pub fn paper_history(cli: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let val = run_kraken(cli, &["paper", "history"])?;
    Ok(match val {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(ref obj) => obj
            .get("trades")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => vec![],
    })
}

pub fn arm_dead_mans_switch(cli: &str, seconds: u64) -> anyhow::Result<()> {
    let secs = seconds.to_string();
    match run_kraken(cli, &["order", "cancel-after", &secs]) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("auth") || e.to_string().contains("Invalid key") => Ok(()),
        Err(e) => Err(e),
    }
}