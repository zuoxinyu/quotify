use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde_json::json;

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str = "https://app.warp.dev/graphql/v2?op=GetRequestLimitInfo";

pub struct WarpProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl WarpProvider {
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
        std::env::var("WARP_API_KEY")
            .or_else(|_| std::env::var("WARP_TOKEN"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn endpoint(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().to_string()
        } else {
            std::env::var("WARP_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for WarpProvider {
    fn name(&self) -> &str {
        "warp"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("Warp API key not configured. Set api_key or WARP_API_KEY/WARP_TOKEN")?;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .post(self.endpoint())
            .headers(headers)
            .json(&json!({
                "operationName": "GetRequestLimitInfo",
                "query": "query GetRequestLimitInfo { requestLimitInfo { isUnlimited nextRefreshTime requestLimit requestsUsedSinceLastRefresh } }",
                "variables": {}
            }))
            .send()
            .await
            .context("Failed to connect to Warp GraphQL API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Warp API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Warp usage response")?;
        let info =
            find_limit_info(&json).context("Warp response did not include request limits")?;
        let unlimited = bool_field(info, &["isUnlimited", "unlimited"]).unwrap_or(false);
        let limit = number_field(info, &["requestLimit", "limit"]);
        let used = number_field(info, &["requestsUsedSinceLastRefresh", "used"]).unwrap_or(0.0);
        let resets_at = string_field(info, &["nextRefreshTime", "resetAt"])
            .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let used_percent = if unlimited {
            0.0
        } else if let Some(limit) = limit.filter(|limit| *limit > 0.0) {
            (used / limit * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: if unlimited { "Unlimited" } else { "Monthly" }.to_string(),
                used_percent,
                limit,
                used: Some(used),
                unit: Some("credits".to_string()),
                resets_at,
            }],
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn find_limit_info(value: &serde_json::Value) -> Option<&serde_json::Value> {
    if value.get("requestLimit").is_some()
        || value.get("requestsUsedSinceLastRefresh").is_some()
        || value.get("isUnlimited").is_some()
    {
        return Some(value);
    }
    match value {
        serde_json::Value::Object(map) => map.values().find_map(find_limit_info),
        serde_json::Value::Array(values) => values.iter().find_map(find_limit_info),
        _ => None,
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

fn bool_field(value: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| value.get(*key)?.as_bool())
}
