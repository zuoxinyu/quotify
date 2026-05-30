use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde_json::json;

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str =
    "https://www.kimi.com/apiv2/kimi.gateway.billing.v1.BillingService/GetUsages";

pub struct KimiProvider {
    auth_token: String,
    base_url: String,
    client: reqwest::Client,
}

impl KimiProvider {
    pub fn new(auth_token: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            auth_token,
            base_url,
            client: http_client(proxy),
        }
    }

    fn resolve_token(&self) -> Option<String> {
        if !self.auth_token.trim().is_empty() {
            return Some(self.auth_token.trim().to_string());
        }
        std::env::var("KIMI_AUTH_TOKEN")
            .ok()
            .filter(|token| !token.trim().is_empty())
    }

    fn endpoint(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().to_string()
        } else {
            std::env::var("KIMI_USAGE_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for KimiProvider {
    fn name(&self) -> &str {
        "kimi"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let token = self
            .resolve_token()
            .context("Kimi auth token not configured. Set api_key or KIMI_AUTH_TOKEN")?;
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {token}").parse()?);

        let resp = self
            .client
            .post(self.endpoint())
            .headers(headers)
            .json(&json!({}))
            .send()
            .await
            .context("Failed to connect to Kimi usage API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Kimi usage API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Kimi usage response")?;
        let usages = json
            .get("usages")
            .and_then(|v| v.as_array())
            .context("Kimi usage response did not contain usages")?;

        let coding = usages
            .iter()
            .find(|usage| {
                usage
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .is_some_and(|scope| scope == "FEATURE_CODING")
            })
            .or_else(|| usages.first())
            .context("Kimi usage response did not include coding usage")?;

        let mut windows = Vec::new();
        if let Some(detail) = coding.get("detail") {
            push_window(&mut windows, "Weekly", detail);
        }
        if let Some(limits) = coding.get("limits").and_then(|v| v.as_array()) {
            for limit in limits {
                let label = limit
                    .get("window")
                    .and_then(|window| window.get("duration"))
                    .and_then(|duration| duration.as_i64())
                    .map(|minutes| {
                        if minutes == 300 {
                            "Session (5h)".to_string()
                        } else {
                            format!("{}m Window", minutes)
                        }
                    })
                    .unwrap_or_else(|| "Rate Limit".to_string());
                if let Some(detail) = limit.get("detail") {
                    push_window(&mut windows, &label, detail);
                }
            }
        }

        if windows.is_empty() {
            anyhow::bail!("Kimi usage response did not contain parseable quota details");
        }

        Ok(UsageData {
            provider: self.name().to_string(),
            windows,
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn push_window(windows: &mut Vec<UsageWindow>, label: &str, detail: &serde_json::Value) {
    let limit = number_field(detail, "limit");
    let used = number_field(detail, "used").unwrap_or(0.0);
    let remaining = number_field(detail, "remaining");
    let limit = limit.or_else(|| remaining.map(|remaining| used + remaining));
    let used_percent = limit
        .filter(|limit| *limit > 0.0)
        .map(|limit| (used / limit * 100.0).clamp(0.0, 100.0))
        .unwrap_or(0.0);
    let resets_at = detail
        .get("resetTime")
        .and_then(|v| v.as_str())
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| dt.with_timezone(&Utc));

    windows.push(UsageWindow {
        label: label.to_string(),
        used_percent,
        limit,
        used: Some(used),
        unit: Some("requests".to_string()),
        resets_at,
    });
}

fn number_field(value: &serde_json::Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
}
