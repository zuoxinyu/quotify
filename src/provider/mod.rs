pub mod abacus;
pub mod alibabatoken;
pub mod amp;
pub mod antigravity;
pub mod augment;
pub mod azureopenai;
pub mod bedrock;
pub mod claude;
pub mod codebuff;
pub mod codex;
pub mod copilot;
pub mod crof;
pub mod cursor;
pub mod deepgram;
pub mod deepseek;
pub mod doubao;
pub mod droid;
pub mod elevenlabs;
pub mod gemini;
pub mod grok;
pub mod groqcloud;
pub mod jetbrains;
pub mod kilo;
pub mod kimi;
pub mod kiro;
pub mod llmproxy;
pub mod mimo;
pub mod minimax;
pub mod mistral;
pub mod moonshot;
pub mod ollama;
pub mod openai;
pub mod opencode;
pub mod openrouter;
pub mod stepfun;
pub mod synthetic;
pub mod t3chat;
pub mod venice;
pub mod vertexai;
pub mod warp;
pub mod windsurf;
pub mod zai;

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
pub struct CodexResetCredit {
    pub status: String,
    pub granted_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexResetCredits {
    pub available_count: i32,
    pub credits: Vec<CodexResetCredit>,
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
    #[allow(dead_code)]
    pub fn max_used_percent(&self) -> f64 {
        self.windows
            .iter()
            .map(|w| w.used_percent)
            .fold(0.0f64, f64::max)
    }

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

pub fn http_client(proxy: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().no_proxy();

    if let Some(proxy) = proxy.map(str::trim).filter(|proxy| !proxy.is_empty()) {
        match reqwest::Proxy::all(proxy) {
            Ok(proxy) => {
                builder = builder.proxy(proxy);
            }
            Err(err) => {
                tracing::warn!("Ignoring invalid network proxy '{proxy}': {err}");
            }
        }
    }

    builder.build().unwrap_or_else(|err| {
        tracing::warn!("Failed to build HTTP client, using default client: {err}");
        reqwest::Client::new()
    })
}
