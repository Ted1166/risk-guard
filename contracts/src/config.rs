#[derive(Debug, Clone)]
pub struct RiskParams {
    pub max_position_pct: f64,
    pub max_total_exposure_pct: f64,

    pub atr_period: usize,
    pub atr_max_multiple: f64,
    pub atr_caution_multiple: f64,

    pub max_drawdown_pct: f64,
    pub drawdown_warning_pct: f64,

    pub rsi_period: usize,
    pub rsi_overbought: f64,
    pub rsi_oversold: f64,

    pub cooldown_secs: u64,

    pub cancel_after_secs: u64,
}

impl Default for RiskParams {
    fn default() -> Self {
        Self {
            max_position_pct: 0.20,
            max_total_exposure_pct: 0.60,
            atr_period: 14,
            atr_max_multiple: 2.5,
            atr_caution_multiple: 1.5,
            max_drawdown_pct: 0.12,
            drawdown_warning_pct: 0.07,
            rsi_period: 14,
            rsi_overbought: 72.0,
            rsi_oversold: 28.0,
            cooldown_secs: 300,
            cancel_after_secs: 120,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub watch_pairs: Vec<String>,
    pub paper_initial_balance: f64,
    pub paper_currency: String,
    pub poll_interval_secs: u64,
    pub ohlc_interval_mins: u32,
    pub ohlc_history: usize,
    pub kraken_cli: String,
    pub ai_enabled: bool,
    pub claude_model: String,
    pub log_file: String,
    pub risk: RiskParams,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            watch_pairs: vec![
                "BTCUSD".to_string(),
                "ETHUSD".to_string(),
                "SOLUSD".to_string(),
            ],
            paper_initial_balance: 10_000.0,
            paper_currency: "USD".to_string(),
            poll_interval_secs: 30,
            ohlc_interval_mins: 60,
            ohlc_history: 50,
            kraken_cli: std::env::var("KRAKEN_CLI_PATH")
                .unwrap_or_else(|_| "kraken".to_string()),
            ai_enabled: std::env::var("AI_ENABLED")
                .map(|v| v != "0" && v.to_lowercase() != "false")
                .unwrap_or(true),
            claude_model: "claude-sonnet-4-20250514".to_string(),
            log_file: "logs/risk_guard.log".to_string(),
            risk: RiskParams::default(),
        }
    }
}