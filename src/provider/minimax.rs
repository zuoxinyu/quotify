use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str = "https://api.minimax.io/v1/coding_plan/remains";

pub struct MiniMaxProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl MiniMaxProvider {
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
        std::env::var("MINIMAX_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn endpoint(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().to_string()
        } else {
            std::env::var("MINIMAX_REMAINS_URL")
                .or_else(|_| std::env::var("MINIMAX_CODING_PLAN_URL"))
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for MiniMaxProvider {
    fn name(&self) -> &str {
        "minimax"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("MiniMax API key not configured. Set api_key or MINIMAX_API_KEY")?;
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .get(self.endpoint())
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to MiniMax coding plan API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("MiniMax coding plan API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse MiniMax coding plan response")?;
        let root = json
            .get("data")
            .or_else(|| json.get("model_remains"))
            .unwrap_or(&json);
        let limit = find_number(
            root,
            &["total", "total_amount", "totalAmount", "quota", "limit"],
        );
        let remaining = find_number(
            root,
            &[
                "remaining",
                "remains",
                "available",
                "available_usage",
                "availableUsage",
            ],
        );
        let used = find_number(root, &["used", "usage", "used_amount", "usedAmount"])
            .or_else(|| {
                limit
                    .zip(remaining)
                    .map(|(limit, remaining)| limit - remaining)
            })
            .unwrap_or(0.0);
        let limit = limit.or_else(|| remaining.map(|remaining| used + remaining));
        let reset = find_number(root, &["remains_time", "end_time", "endTime"])
            .and_then(timestamp_to_utc)
            .or_else(|| find_reset(root, &["resetAt", "resetsAt", "endTime"]));
        let used_percent = limit
            .filter(|limit| *limit > 0.0)
            .map(|limit| (used / limit * 100.0).clamp(0.0, 100.0))
            .unwrap_or(0.0);

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Coding Plan".to_string(),
                used_percent,
                limit,
                used: Some(used),
                unit: Some("tokens".to_string()),
                resets_at: reset,
            }],
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn timestamp_to_utc(raw: f64) -> Option<DateTime<Utc>> {
    let ts = raw as i64;
    if ts > 10_000_000_000 {
        Utc.timestamp_millis_opt(ts).single()
    } else if ts > 0 {
        Utc.timestamp_opt(ts, 0).single()
    } else {
        None
    }
}

fn find_number(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                if keys.iter().any(|needle| key.eq_ignore_ascii_case(needle))
                    && let Some(number) = val.as_f64().or_else(|| val.as_str()?.parse().ok())
                {
                    return Some(number);
                }
                if let Some(number) = find_number(val, keys) {
                    return Some(number);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(|v| find_number(v, keys)),
        _ => None,
    }
}

fn find_reset(value: &serde_json::Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    find_string(value, keys)
        .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn find_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                if keys.iter().any(|needle| key.eq_ignore_ascii_case(needle))
                    && let Some(raw) = val.as_str()
                {
                    return Some(raw.to_string());
                }
                if let Some(raw) = find_string(val, keys) {
                    return Some(raw);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(|v| find_string(v, keys)),
        _ => None,
    }
}
