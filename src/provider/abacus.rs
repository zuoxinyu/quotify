use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{COOKIE, HeaderMap, HeaderValue};

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://apps.abacus.ai";

pub struct AbacusProvider {
    cookie: String,
    base_url: String,
    client: reqwest::Client,
}

impl AbacusProvider {
    pub fn new(cookie: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            cookie,
            base_url,
            client: http_client(proxy),
        }
    }

    fn resolve_cookie(&self) -> Option<String> {
        if !self.cookie.trim().is_empty() {
            return Some(cookie_header(self.cookie.trim()));
        }
        std::env::var("ABACUS_COOKIE")
            .or_else(|_| std::env::var("ABACUS_COOKIE_HEADER"))
            .or_else(|_| std::env::var("ABACUS_AI_COOKIE"))
            .ok()
            .filter(|cookie| !cookie.trim().is_empty())
            .map(|cookie| cookie_header(cookie.trim()))
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("ABACUS_BASE_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for AbacusProvider {
    fn name(&self) -> &str {
        "abacus"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let cookie = self.resolve_cookie().context(
            "Abacus AI cookie not configured. Set api_key, ABACUS_COOKIE, or ABACUS_COOKIE_HEADER",
        )?;
        let base_url = self.base_url();
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(&cookie)?);

        let points_resp = self
            .client
            .get(format!("{base_url}/api/_getOrganizationComputePoints"))
            .headers(headers.clone())
            .send()
            .await
            .context("Failed to connect to Abacus compute points API")?;
        if !points_resp.status().is_success() {
            let status = points_resp.status();
            let body = points_resp.text().await.unwrap_or_default();
            anyhow::bail!("Abacus compute points API error: {status} - {body}");
        }
        let points_json: serde_json::Value = points_resp
            .json()
            .await
            .context("Failed to parse Abacus compute points response")?;

        let billing_json = match self
            .client
            .post(format!("{base_url}/api/_getBillingInfo"))
            .headers(headers)
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp.json::<serde_json::Value>().await.ok(),
            _ => None,
        };

        let total = find_number(
            &points_json,
            &[
                "totalComputePoints",
                "total_compute_points",
                "monthlyComputePoints",
                "monthly_compute_points",
            ],
        )
        .unwrap_or(0.0);
        let remaining = find_number(
            &points_json,
            &[
                "computePointsLeft",
                "compute_points_left",
                "remainingComputePoints",
                "remaining_compute_points",
            ],
        )
        .unwrap_or(0.0);
        let used = find_number(&points_json, &["computePointsUsed", "compute_points_used"])
            .unwrap_or_else(|| (total - remaining).max(0.0));
        let limit = if total > 0.0 {
            Some(total)
        } else if used > 0.0 || remaining > 0.0 {
            Some(used + remaining)
        } else {
            None
        };
        let used_percent = limit
            .filter(|limit| *limit > 0.0)
            .map(|limit| (used / limit * 100.0).clamp(0.0, 100.0))
            .unwrap_or(0.0);
        let resets_at = billing_json
            .as_ref()
            .and_then(|json| find_string(json, &["nextBillingDate", "next_billing_date"]))
            .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let tier = billing_json
            .as_ref()
            .and_then(|json| find_string(json, &["currentTier", "current_tier", "tier", "plan"]));

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: tier.unwrap_or_else(|| "Monthly".to_string()),
                used_percent,
                limit,
                used: Some(used),
                unit: Some("credits".to_string()),
                resets_at,
            }],
            credits: Some(CreditsInfo {
                balance: remaining.max(0.0),
                currency: "credits".to_string(),
                total_granted: limit,
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn cookie_header(raw: &str) -> String {
    raw.strip_prefix("Cookie:")
        .or_else(|| raw.strip_prefix("cookie:"))
        .unwrap_or(raw)
        .trim()
        .to_string()
}

fn find_number(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    match value {
        serde_json::Value::Object(map) => keys
            .iter()
            .find_map(|key| map.get(*key).and_then(json_number))
            .or_else(|| map.values().find_map(|value| find_number(value, keys))),
        serde_json::Value::Array(values) => {
            values.iter().find_map(|value| find_number(value, keys))
        }
        _ => None,
    }
}

fn find_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => keys
            .iter()
            .find_map(|key| {
                map.get(*key)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .or_else(|| map.values().find_map(|value| find_string(value, keys))),
        serde_json::Value::Array(values) => {
            values.iter().find_map(|value| find_string(value, keys))
        }
        _ => None,
    }
}

fn json_number(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}
