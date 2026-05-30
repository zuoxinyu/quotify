use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://api.synthetic.new/v2";

pub struct SyntheticProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl SyntheticProvider {
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
        std::env::var("SYNTHETIC_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("SYNTHETIC_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for SyntheticProvider {
    fn name(&self) -> &str {
        "synthetic"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("Synthetic API key not configured. Set api_key or SYNTHETIC_API_KEY")?;
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .get(format!("{}/quotas", self.base_url()))
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to Synthetic quota API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Synthetic quota API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Synthetic quota response")?;
        let subscription = json.get("subscription").unwrap_or(&json);
        let limit = number_field(subscription, &["limit", "requestLimit"]).unwrap_or(0.0);
        let used = number_field(subscription, &["requests", "used", "usage"]).unwrap_or(0.0);
        let resets_at = string_field(subscription, &["renewsAt", "renews_at", "resetAt"])
            .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let used_percent = if limit > 0.0 {
            (used / limit * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Subscription".to_string(),
                used_percent,
                limit: (limit > 0.0).then_some(limit),
                used: Some(used),
                unit: Some("requests".to_string()),
                resets_at,
            }],
            credits: None,
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
