use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_HOST: &str = "https://api.z.ai";
const DEFAULT_PATH: &str = "/api/monitor/usage/quota/limit";

pub struct ZaiProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl ZaiProvider {
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
        std::env::var("Z_AI_API_KEY")
            .or_else(|_| std::env::var("ZAI_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn quota_url(&self) -> String {
        if let Ok(url) = std::env::var("Z_AI_QUOTA_URL")
            && !url.trim().is_empty()
        {
            return url.trim().to_string();
        }
        if !self.base_url.trim().is_empty() {
            return self.base_url.trim().to_string();
        }
        let host = std::env::var("Z_AI_API_HOST")
            .ok()
            .filter(|host| !host.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_HOST.to_string());
        let host = if host.starts_with("http://") || host.starts_with("https://") {
            host
        } else {
            format!("https://{host}")
        };
        format!("{}{}", host.trim_end_matches('/'), DEFAULT_PATH)
    }
}

#[async_trait::async_trait]
impl Provider for ZaiProvider {
    fn name(&self) -> &str {
        "zai"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("z.ai API key not configured. Set api_key or Z_AI_API_KEY")?;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .get(self.quota_url())
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to z.ai quota API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("z.ai quota API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse z.ai quota response")?;
        let data = json.get("data").unwrap_or(&json);
        let plan = string_field(
            data,
            &["planName", "plan", "plan_type", "planType", "packageName"],
        )
        .unwrap_or_else(|| "Quota".to_string());

        let mut windows = Vec::new();
        if let Some(limits) = data.get("limits").and_then(|value| value.as_array()) {
            for limit in limits {
                if let Some(window) = parse_limit_window(limit, &plan) {
                    windows.push(window);
                }
            }
        }

        if windows.is_empty()
            && let Some(window) = parse_limit_window(data, &plan)
        {
            windows.push(window);
        }

        if windows.is_empty() {
            windows.push(UsageWindow {
                label: plan,
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
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn parse_limit_window(value: &serde_json::Value, plan: &str) -> Option<UsageWindow> {
    let limit = number_field(
        value,
        &[
            "limit",
            "total",
            "totalQuota",
            "quota",
            "max",
            "totalAmount",
        ],
    );
    let used = number_field(value, &["used", "usedQuota", "usage", "consumed"]);
    let remaining = number_field(value, &["remaining", "remainingQuota", "available"]);
    let used = used.or_else(|| match (limit, remaining) {
        (Some(limit), Some(remaining)) => Some((limit - remaining).max(0.0)),
        _ => None,
    });
    let used_percent = number_field(value, &["usedPercent", "used_percent", "usagePercent"])
        .or_else(|| match (used, limit) {
            (Some(used), Some(limit)) if limit > 0.0 => {
                Some((used / limit * 100.0).clamp(0.0, 100.0))
            }
            _ => None,
        })
        .unwrap_or(0.0);
    let label = string_field(value, &["type", "limitType", "name", "label"])
        .map(|label| label.replace("_LIMIT", ""))
        .unwrap_or_else(|| plan.to_string());
    let resets_at =
        number_field(value, &["nextResetTime", "next_reset_time", "resetAt"]).and_then(|raw| {
            let raw = raw as i64;
            let seconds = if raw > 10_000_000_000 {
                raw / 1000
            } else {
                raw
            };
            Utc.timestamp_opt(seconds, 0).single()
        });

    if used.is_some() || limit.is_some() || remaining.is_some() || used_percent > 0.0 {
        Some(UsageWindow {
            label,
            used_percent,
            limit,
            used,
            unit: Some("quota".to_string()),
            resets_at,
        })
    } else {
        None
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
