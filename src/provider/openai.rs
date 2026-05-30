use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://api.openai.com";

pub struct OpenAiProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
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
        std::env::var("OPENAI_ADMIN_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("OPENAI_API_BASE")
                .or_else(|_| std::env::var("OPENAI_BASE_URL"))
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self.resolve_api_key().context(
            "OpenAI API key not configured. Set api_key, OPENAI_ADMIN_KEY, or OPENAI_API_KEY",
        )?;
        let base_url = self.base_url();
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        match self.fetch_costs(&base_url, &headers).await {
            Ok(data) => Ok(data),
            Err(cost_err) => match self.fetch_credit_grants(&base_url, &headers).await {
                Ok(data) => Ok(data),
                Err(credit_err) => anyhow::bail!(
                    "OpenAI costs API failed: {cost_err}; legacy credit grants failed: {credit_err}"
                ),
            },
        }
    }
}

impl OpenAiProvider {
    async fn fetch_costs(&self, base_url: &str, headers: &HeaderMap) -> Result<UsageData> {
        let end = Utc::now();
        let start = end - Duration::days(7);
        let resp = self
            .client
            .get(format!("{base_url}/v1/organization/costs"))
            .headers(headers.clone())
            .query(&[
                ("start_time", start.timestamp().to_string()),
                ("end_time", end.timestamp().to_string()),
                ("limit", "7".to_string()),
            ])
            .send()
            .await
            .context("Failed to connect to OpenAI costs API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI costs API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse OpenAI costs response")?;
        let total_cost = sum_cost_values(&json);

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Cost 7d".to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(total_cost),
                unit: Some("USD".to_string()),
                resets_at: None,
            }],
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }

    async fn fetch_credit_grants(&self, base_url: &str, headers: &HeaderMap) -> Result<UsageData> {
        let resp = self
            .client
            .get(format!("{base_url}/dashboard/billing/credit_grants"))
            .headers(headers.clone())
            .send()
            .await
            .context("Failed to connect to OpenAI credit grants API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI credit grants API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse OpenAI credit grants response")?;
        let root = json.get("data").unwrap_or(&json);
        let granted = number_field(root, &["total_granted", "totalGranted"]).unwrap_or(0.0);
        let used = number_field(root, &["total_used", "totalUsed"]).unwrap_or(0.0);
        let available = number_field(root, &["total_available", "totalAvailable"])
            .unwrap_or_else(|| (granted - used).max(0.0));
        let used_percent = if granted > 0.0 {
            (used / granted * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Credits".to_string(),
                used_percent,
                limit: (granted > 0.0).then_some(granted),
                used: Some(used),
                unit: Some("USD".to_string()),
                resets_at: None,
            }],
            credits: Some(CreditsInfo {
                balance: available,
                currency: "USD".to_string(),
                total_granted: (granted > 0.0).then_some(granted),
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn sum_cost_values(value: &serde_json::Value) -> f64 {
    match value {
        serde_json::Value::Object(map) => {
            let own = if map
                .get("amount")
                .is_some_and(|amount| amount.get("value").is_some())
            {
                map.get("amount")
                    .and_then(|amount| amount.get("value"))
                    .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            own + map.values().map(sum_cost_values).sum::<f64>()
        }
        serde_json::Value::Array(values) => values.iter().map(sum_cost_values).sum(),
        _ => 0.0,
    }
}

fn number_field(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
    })
}
