use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use reqwest::header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://platform.stepfun.com";

pub struct StepFunProvider {
    token: String,
    base_url: String,
    client: reqwest::Client,
}

impl StepFunProvider {
    pub fn new(token: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            token,
            base_url,
            client: http_client(proxy),
        }
    }

    fn resolve_token(&self) -> Option<String> {
        if !self.token.trim().is_empty() {
            return Some(normalize_token(self.token.trim()));
        }
        std::env::var("STEPFUN_TOKEN")
            .or_else(|_| std::env::var("OASIS_TOKEN"))
            .ok()
            .filter(|token| !token.trim().is_empty())
            .map(|token| normalize_token(token.trim()))
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("STEPFUN_BASE_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .map(|url| url.trim().trim_end_matches('/').to_string())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for StepFunProvider {
    fn name(&self) -> &str {
        "stepfun"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let token = self
            .resolve_token()
            .context("StepFun token not configured. Set api_key, STEPFUN_TOKEN, or OASIS_TOKEN")?;
        let base_url = self.base_url();
        let rate_url =
            format!("{base_url}/api/step.openapi.devcenter.Dashboard/QueryStepPlanRateLimit");
        let status_url =
            format!("{base_url}/api/step.openapi.devcenter.Dashboard/GetStepPlanStatus");
        let headers = auth_headers(&token)?;

        let rate_resp = self
            .client
            .post(rate_url)
            .headers(headers.clone())
            .json(&serde_json::json!({}))
            .send()
            .await
            .context("Failed to connect to StepFun rate limit API")?;
        if !rate_resp.status().is_success() {
            let status = rate_resp.status();
            let body = rate_resp.text().await.unwrap_or_default();
            anyhow::bail!("StepFun rate limit API error: {status} - {body}");
        }
        let rate_json: serde_json::Value = rate_resp
            .json()
            .await
            .context("Failed to parse StepFun rate limit response")?;

        let plan_name = match self
            .client
            .post(status_url)
            .headers(headers)
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|json| find_string(&json, &["name", "planName", "plan_name"])),
            _ => None,
        };

        let root = rate_json.get("data").unwrap_or(&rate_json);
        let mut windows = Vec::new();
        let five_hour_label = plan_name
            .as_deref()
            .map(|name| format!("5-hour {name}"))
            .unwrap_or_else(|| "5-hour".to_string());
        push_rate_window(
            &mut windows,
            root,
            &five_hour_label,
            &["five_hour_usage_left_rate", "fiveHourUsageLeftRate"],
            &["five_hour_usage_reset_time", "fiveHourUsageResetTime"],
        );
        push_rate_window(
            &mut windows,
            root,
            "Weekly",
            &["weekly_usage_left_rate", "weeklyUsageLeftRate"],
            &["weekly_usage_reset_time", "weeklyUsageResetTime"],
        );
        if windows.is_empty() {
            anyhow::bail!("StepFun response did not contain parseable rate limit data");
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

fn auth_headers(token: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        COOKIE,
        HeaderValue::from_str(&format!("Oasis-Token={token}"))?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

fn push_rate_window(
    windows: &mut Vec<UsageWindow>,
    value: &serde_json::Value,
    label: &str,
    left_rate_keys: &[&str],
    reset_keys: &[&str],
) {
    let Some(left_rate) = find_number(value, left_rate_keys) else {
        return;
    };
    let used_percent = ((1.0 - left_rate) * 100.0).clamp(0.0, 100.0);
    windows.push(UsageWindow {
        label: label.to_string(),
        used_percent,
        limit: Some(100.0),
        used: Some(used_percent),
        unit: Some("%".to_string()),
        resets_at: find_reset(value, reset_keys),
    });
}

fn normalize_token(raw: &str) -> String {
    raw.strip_prefix("Oasis-Token=")
        .or_else(|| raw.strip_prefix("oasis-token="))
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

fn find_reset(value: &serde_json::Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    match value {
        serde_json::Value::Object(map) => keys
            .iter()
            .find_map(|key| map.get(*key).and_then(parse_reset))
            .or_else(|| map.values().find_map(|value| find_reset(value, keys))),
        serde_json::Value::Array(values) => values.iter().find_map(|value| find_reset(value, keys)),
        _ => None,
    }
}

fn json_number(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

fn parse_reset(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    if let Some(raw) = value.as_str() {
        return DateTime::parse_from_rfc3339(raw)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| raw.parse::<i64>().ok().and_then(timestamp_to_utc));
    }
    value.as_i64().and_then(timestamp_to_utc)
}

fn timestamp_to_utc(raw: i64) -> Option<DateTime<Utc>> {
    if raw > 10_000_000_000 {
        Utc.timestamp_millis_opt(raw).single()
    } else if raw > 0 {
        Utc.timestamp_opt(raw, 0).single()
    } else {
        None
    }
}
