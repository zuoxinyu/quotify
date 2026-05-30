use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://server.codeium.com/api/v1";

pub struct WindsurfProvider {
    service_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl WindsurfProvider {
    pub fn new(service_key: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            service_key,
            base_url,
            client: http_client(proxy),
        }
    }

    fn resolve_service_key(&self) -> Option<String> {
        if !self.service_key.trim().is_empty() {
            return Some(self.service_key.trim().to_string());
        }
        std::env::var("WINDSURF_SERVICE_KEY")
            .or_else(|_| std::env::var("CODEIUM_SERVICE_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("WINDSURF_API_BASE")
                .or_else(|_| std::env::var("CODEIUM_API_BASE"))
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for WindsurfProvider {
    fn name(&self) -> &str {
        "windsurf"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let service_key = self.resolve_service_key().context(
            "Windsurf service key not configured. Set api_key, WINDSURF_SERVICE_KEY, or CODEIUM_SERVICE_KEY",
        )?;
        let resp = self
            .client
            .post(format!("{}/GetUsageConfig", self.base_url()))
            .json(&json!({
                "service_key": service_key,
                "team_level": true
            }))
            .send()
            .await
            .context("Failed to connect to Windsurf usage config API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Windsurf usage config API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Windsurf usage config response")?;
        let cap = number_field(&json, &["addOnCreditCap", "add_on_credit_cap"]);

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Add-on Cap".to_string(),
                used_percent: 0.0,
                limit: cap,
                used: None,
                unit: Some("credits".to_string()),
                resets_at: None,
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
