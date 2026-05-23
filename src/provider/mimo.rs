use anyhow::{Context, Result};
use chrono::Utc;

use super::{CreditsInfo, Provider, UsageData, UsageWindow};
use crate::cookies;

pub struct MimoProvider {
    api_key: String,
    client: reqwest::Client,
}

impl MimoProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[expect(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct MimoBalanceResponse {
    data: Option<MimoBalanceData>,
    balance: Option<f64>,
    total_balance: Option<f64>,
    error: Option<MimoError>,
}

#[expect(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct MimoBalanceData {
    balance: Option<f64>,
    total_balance: Option<f64>,
    granted: Option<f64>,
    currency: Option<String>,
}

#[expect(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct MimoError {
    message: Option<String>,
    code: Option<i32>,
}

#[async_trait::async_trait]
impl Provider for MimoProvider {
    fn name(&self) -> &str {
        "mimo"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let cookie_header = cookies::find_cookie_header(&["xiaomimimo.com", ".xiaomimimo.com"])
            .context("No Xiaomi MiMo browser session found. Please log in at platform.xiaomimimo.com")?;

        let url = std::env::var("MIMO_API_URL")
            .unwrap_or_else(|_| "https://platform.xiaomimimo.com/api/v1/balance".to_string());

        let resp = self
            .client
            .get(&url)
            .header("Cookie", cookie_header)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .send()
            .await
            .context("Failed to connect to MiMo API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("MiMo API error: {status} - {body}");
        }

        let balance: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse MiMo balance response")?;

        tracing::debug!("MiMo API response: {balance:#?}");

        let mut windows = Vec::new();
        let mut credits = None;

        let balance_val = balance
            .get("data")
            .and_then(|d| d.get("balance"))
            .or_else(|| balance.get("balance"))
            .or_else(|| balance.get("data").and_then(|d| d.get("total_balance")))
            .or_else(|| balance.get("total_balance"))
            .and_then(|v| v.as_f64());

        let currency = balance
            .get("data")
            .and_then(|d| d.get("currency"))
            .or_else(|| balance.get("currency"))
            .and_then(|v| v.as_str())
            .unwrap_or("CNY");

        if let Some(bal) = balance_val {
            credits = Some(CreditsInfo {
                balance: bal,
                currency: currency.to_string(),
                total_granted: balance
                    .get("data")
                    .and_then(|d| d.get("granted"))
                    .or_else(|| balance.get("granted"))
                    .and_then(|v| v.as_f64()),
                topped_up: None,
            });

            windows.push(UsageWindow {
                label: format!("Balance ({currency})"),
                used_percent: 0.0,
                limit: None,
                used: None,
                unit: Some(currency.to_string()),
                resets_at: None,
            });
        }

        if windows.is_empty() {
            windows.push(UsageWindow {
                label: "No data".to_string(),
                used_percent: 0.0,
                limit: None,
                used: None,
                unit: None,
                resets_at: None,
            });
        }

        Ok(UsageData {
            provider: self.name().to_string(),
            windows,
            credits,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

