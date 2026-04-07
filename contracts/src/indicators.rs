use crate::market::Candle;


fn true_range(candle: &Candle, prev_close: f64) -> f64 {
    let hl = candle.high - candle.low;
    let hpc = (candle.high - prev_close).abs();
    let lpc = (candle.low  - prev_close).abs();
    hl.max(hpc).max(lpc)
}

pub fn atr(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period + 1 {
        return None;
    }

    let trs: Vec<f64> = candles
        .windows(2)
        .map(|w| true_range(&w[1], w[0].close))
        .collect();

    let mut atr_val: f64 = trs[..period].iter().sum::<f64>() / period as f64;

    for &tr in &trs[period..] {
        atr_val = (atr_val * (period as f64 - 1.0) + tr) / period as f64;
    }

    Some(atr_val)
}

pub fn atr_series(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.len() < period + 1 {
        return vec![];
    }

    let trs: Vec<f64> = candles
        .windows(2)
        .map(|w| true_range(&w[1], w[0].close))
        .collect();

    let mut series = Vec::with_capacity(trs.len() - period + 1);
    let mut atr_val: f64 = trs[..period].iter().sum::<f64>() / period as f64;
    series.push(atr_val);

    for &tr in &trs[period..] {
        atr_val = (atr_val * (period as f64 - 1.0) + tr) / period as f64;
        series.push(atr_val);
    }

    series
}


pub fn rsi(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period + 1 {
        return None;
    }

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let deltas: Vec<f64> = closes.windows(2).map(|w| w[1] - w[0]).collect();

    let gains: Vec<f64> = deltas.iter().map(|d| d.max(0.0)).collect();
    let losses: Vec<f64> = deltas.iter().map(|d| (-d).max(0.0)).collect();

    let mut avg_gain: f64 = gains[..period].iter().sum::<f64>() / period as f64;
    let mut avg_loss: f64 = losses[..period].iter().sum::<f64>() / period as f64;

    for i in period..gains.len() {
        avg_gain = (avg_gain * (period as f64 - 1.0) + gains[i]) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + losses[i]) / period as f64;
    }

    if avg_loss == 0.0 {
        return Some(100.0);
    }

    let rs = avg_gain / avg_loss;
    Some(100.0 - 100.0 / (1.0 + rs))
}


#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct DrawdownTracker {
    pub peak: f64,
    pub current: f64,
    pub max_drawdown: f64,
}

impl DrawdownTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, value: f64) -> f64 {
        if value > self.peak {
            self.peak = value;
        }
        self.current = value;

        let dd = if self.peak > 0.0 {
            (self.peak - self.current) / self.peak
        } else {
            0.0
        };

        if dd > self.max_drawdown {
            self.max_drawdown = dd;
        }

        dd
    }

    #[allow(dead_code)]
    pub fn current_drawdown(&self) -> f64 {
        if self.peak > 0.0 {
            (self.peak - self.current) / self.peak
        } else {
            0.0
        }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}


#[derive(Debug, Clone, PartialEq)]
pub enum VolatilityRegime {
    Low,
    Normal,
    Elevated,
    Extreme,
    Unknown,
}

impl std::fmt::Display for VolatilityRegime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low      => write!(f, "low"),
            Self::Normal   => write!(f, "normal"),
            Self::Elevated => write!(f, "elevated"),
            Self::Extreme  => write!(f, "extreme"),
            Self::Unknown  => write!(f, "unknown"),
        }
    }
}

pub fn volatility_regime(
    current_atr: f64,
    atr_history: &[f64],
    lookback: usize,
) -> (VolatilityRegime, f64) {
    if atr_history.is_empty() || current_atr == 0.0 {
        return (VolatilityRegime::Unknown, 1.0);
    }

    let window = if atr_history.len() >= lookback {
        &atr_history[atr_history.len() - lookback..]
    } else {
        atr_history
    };

    let avg: f64 = window.iter().sum::<f64>() / window.len() as f64;
    if avg == 0.0 {
        return (VolatilityRegime::Unknown, 1.0);
    }

    let multiple = current_atr / avg;

    let regime = if multiple < 0.75 {
        VolatilityRegime::Low
    } else if multiple < 1.5 {
        VolatilityRegime::Normal
    } else if multiple < 2.5 {
        VolatilityRegime::Elevated
    } else {
        VolatilityRegime::Extreme
    };

    (regime, (multiple * 100.0).round() / 100.0)
}


pub fn momentum(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period + 1 {
        return None;
    }
    let start = candles[candles.len() - period - 1].close;
    let end   = candles[candles.len() - 1].close;
    if start == 0.0 { return None; }
    Some(((end - start) / start) * 100.0)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_candles(n: usize) -> Vec<Candle> {
        let mut candles = Vec::with_capacity(n);
        let mut price = 60_000.0f64;
        for i in 0..n {
            let noise = ((i as f64 * 1.618).sin() * 500.0) + 100.0;
            let open  = price;
            let high  = open + noise.abs();
            let low   = open - noise.abs() * 0.5;
            let close = open + noise * 0.3;
            candles.push(Candle { time: i as i64, open, high, low, close, volume: 100.0 });
            price = close;
        }
        candles
    }

    #[test]
    fn test_atr_requires_enough_candles() {
        let candles = synthetic_candles(10);
        assert!(atr(&candles, 14).is_none());
        let candles = synthetic_candles(20);
        assert!(atr(&candles, 14).is_some());
    }

    #[test]
    fn test_rsi_range() {
        let candles = synthetic_candles(30);
        let r = rsi(&candles, 14).unwrap();
        assert!(r >= 0.0 && r <= 100.0, "RSI out of range: {r}");
    }

    #[test]
    fn test_drawdown_tracker() {
        let mut dt = DrawdownTracker::new();
        dt.update(10_000.0);
        dt.update(11_000.0);
        let dd = dt.update(9_900.0);
        // Peak is 11_000, current is 9_900 → dd = (11000-9900)/11000 ≈ 0.1
        assert!((dd - 0.1).abs() < 0.001, "Expected ~10% dd, got {dd:.4}");
        assert_eq!(dt.max_drawdown, dd);
    }

    #[test]
    fn test_volatility_regime() {
        let history = vec![100.0, 105.0, 98.0, 102.0, 100.0];
        let (regime, _multiple) = volatility_regime(100.0, &history, 5);
        assert_eq!(regime, VolatilityRegime::Normal);

        let (regime2, _) = volatility_regime(300.0, &history, 5);
        assert_eq!(regime2, VolatilityRegime::Extreme);
    }
}