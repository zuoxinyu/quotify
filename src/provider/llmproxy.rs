use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:4000";

pub struct LlmProxyProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl LlmProxyProvider {
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
        std::env::var("LLM_PROXY_API_KEY")
            .or_else(|_| std::env::var("LLMPROXY_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("LLM_PROXY_BASE_URL")
                .or_else(|_| std::env::var("LLMPROXY_BASE_URL"))
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for LlmProxyProvider {
    fn name(&self) -> &str {
        "llmproxy"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("LLM Proxy API key not configured. Set api_key or LLM_PROXY_API_KEY")?;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .get(format!("{}/v1/quota-stats", self.base_url()))
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to LLM Proxy quota stats API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM Proxy quota API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse LLM Proxy quota response")?;
        let root = json.get("data").unwrap_or(&json);
        let used = number_field(root, &["used", "usage", "spent", "cost"]).unwrap_or(0.0);
        let limit = number_field(root, &["limit", "quota", "budget"]);
        let remaining = number_field(root, &["remaining", "balance"]);
        let used_percent = number_field(root, &["used_percent", "usedPercent", "percent"])
            .or_else(|| {
                limit
                    .filter(|limit| *limit > 0.0)
                    .map(|limit| used / limit * 100.0)
            })
            .unwrap_or(0.0)
            .clamp(0.0, 100.0);
        let resets_at = string_field(root, &["reset_at", "resetAt", "resetsAt"])
            .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let currency =
            string_field(root, &["currency", "unit"]).unwrap_or_else(|| "USD".to_string());

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Quota".to_string(),
                used_percent,
                limit,
                used: Some(used),
                unit: Some(currency.clone()),
                resets_at,
            }],
            credits: remaining.map(|balance| CreditsInfo {
                balance,
                currency,
                total_granted: limit,
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
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
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|v| v.as_str()).map(str::to_string))
}
