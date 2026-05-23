pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod deepseek;
pub mod gemini;
pub mod mimo;
pub mod opencode;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageWindow {
    pub label: String,
    pub used_percent: f64,
    pub limit: Option<f64>,
    pub used: Option<f64>,
    pub unit: Option<String>,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditsInfo {
    pub balance: f64,
    pub currency: String,
    pub total_granted: Option<f64>,
    pub topped_up: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageData {
    pub provider: String,
    pub windows: Vec<UsageWindow>,
    pub credits: Option<CreditsInfo>,
    pub fetched_at: DateTime<Utc>,
    pub error: Option<String>,
}

impl UsageData {
    #[expect(dead_code)]
    pub fn max_used_percent(&self) -> f64 {
        self.windows
            .iter()
            .map(|w| w.used_percent)
            .fold(0.0f64, f64::max)
    }

    #[expect(dead_code)]
    pub fn has_data(&self) -> bool {
        !self.windows.is_empty()
            && self
                .windows
                .iter()
                .any(|w| w.label != "No data" && w.label != "Error")
    }
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn fetch_usage(&self) -> Result<UsageData>;
}
