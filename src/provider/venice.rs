use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://api.venice.ai/api/v1";

pub struct VeniceProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl VeniceProvider {
    pub fn new(api_key: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            api_key,
            base_url,
            client: http_client(proxy),
        }
    }

    fn resolve_api_key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.trim().to_string());
        }
        std::env::var("VENICE_API_KEY")
            .or_else(|_| std::env::var("VENICE_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        configured_url(&self.base_url, "VENICE_API_URL", DEFAULT_BASE_URL)
    }
}

#[async_trait::async_trait]
impl Provider for VeniceProvider {
    fn name(&self) -> &str {
        "venice"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("Venice API key not configured. Set api_key or VENICE_API_KEY")?;
        let base_url = self.base_url();

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .get(format!("{base_url}/billing/balance"))
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to Venice balance API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Venice balance API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Venice balance response")?;
        let currency = string_field(&json, &["consumptionCurrency", "consumption_currency"])
            .unwrap_or_else(|| "DIEM".to_string());
        let balances = json.get("balances").unwrap_or(&json);
        let balance = match currency.to_ascii_uppercase().as_str() {
            "USD" => number_field(balances, &["usd", "USD"]).unwrap_or(0.0),
            "DIEM" => number_field(balances, &["diem", "DIEM"]).unwrap_or(0.0),
            _ => number_field(balances, &["diem", "DIEM", "usd", "USD"]).unwrap_or(0.0),
        };
        let allocation = number_field(&json, &["diemEpochAllocation", "diem_epoch_allocation"]);
        let used_percent = if currency.eq_ignore_ascii_case("DIEM") {
            allocation
                .filter(|limit| *limit > 0.0)
                .map(|limit| ((limit - balance).max(0.0) / limit * 100.0).clamp(0.0, 100.0))
                .unwrap_or(0.0)
        } else {
            0.0
        };

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Balance".to_string(),
                used_percent,
                limit: allocation,
                used: allocation.map(|limit| (limit - balance).max(0.0)),
                unit: Some(currency.clone()),
                resets_at: None,
            }],
            credits: Some(CreditsInfo {
                balance,
                currency,
                total_granted: allocation,
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn configured_url(configured: &str, env_name: &str, default: &str) -> String {
    if !configured.trim().is_empty() {
        configured.trim().trim_end_matches('/').to_string()
    } else {
        std::env::var(env_name)
            .ok()
            .filter(|url| !url.trim().is_empty())
            .unwrap_or_else(|| default.to_string())
            .trim_end_matches('/')
            .to_string()
    }
}

fn number_field(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
    })
}

fn string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    })
}
