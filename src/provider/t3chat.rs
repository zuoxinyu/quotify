use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{COOKIE, HeaderMap, HeaderValue};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str = "https://t3.chat/api/trpc/getCustomerData";

pub struct T3ChatProvider {
    cookie: String,
    base_url: String,
    client: reqwest::Client,
}

impl T3ChatProvider {
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
        std::env::var("T3_CHAT_COOKIE")
            .or_else(|_| std::env::var("T3CHAT_COOKIE"))
            .ok()
            .filter(|cookie| !cookie.trim().is_empty())
            .map(|cookie| cookie_header(cookie.trim()))
    }

    fn endpoint(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().to_string()
        } else {
            std::env::var("T3_CHAT_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for T3ChatProvider {
    fn name(&self) -> &str {
        "t3chat"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let cookie = self.resolve_cookie().context(
            "T3 Chat cookie not configured. Set api_key, T3_CHAT_COOKIE, or T3CHAT_COOKIE",
        )?;
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(&cookie)?);

        let resp = self
            .client
            .get(self.endpoint())
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to T3 Chat customer data API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("T3 Chat customer data API error: {status} - {body}");
        }

        let text = resp
            .text()
            .await
            .context("Failed to read T3 Chat customer data response")?;
        let json = parse_jsonl_or_json(&text)?;
        let mut windows = Vec::new();
        push_window(&mut windows, &json, "Base", &["base"]);
        push_window(&mut windows, &json, "Overage", &["overage", "monthly"]);
        if windows.is_empty() {
            push_window(&mut windows, &json, "Usage", &["usage", "credits"]);
        }
        if windows.is_empty() {
            anyhow::bail!("T3 Chat response did not contain parseable quota data");
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

fn push_window(
    windows: &mut Vec<UsageWindow>,
    value: &serde_json::Value,
    label: &str,
    hints: &[&str],
) {
    let Some(node) = find_object(value, hints) else {
        return;
    };
    let used = find_number(node, &["used", "usage", "consumed"]).unwrap_or(0.0);
    let limit = find_number(node, &["limit", "quota", "total"]);
    if used == 0.0 && limit.is_none() {
        return;
    }
    let reset = find_string(node, &["resetAt", "resetsAt", "nextReset"])
        .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let used_percent = limit
        .filter(|limit| *limit > 0.0)
        .map(|limit| (used / limit * 100.0).clamp(0.0, 100.0))
        .unwrap_or(0.0);
    windows.push(UsageWindow {
        label: label.to_string(),
        used_percent,
        limit,
        used: Some(used),
        unit: Some("credits".to_string()),
        resets_at: reset,
    });
}

fn parse_jsonl_or_json(text: &str) -> Result<serde_json::Value> {
    if let Ok(json) = serde_json::from_str(text) {
        return Ok(json);
    }
    let mut values = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            values.push(json);
        }
    }
    if values.is_empty() {
        anyhow::bail!("response was neither JSON nor JSONL");
    }
    Ok(serde_json::Value::Array(values))
}

fn find_object<'a>(value: &'a serde_json::Value, hints: &[&str]) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                if hints
                    .iter()
                    .any(|hint| key.to_ascii_lowercase().contains(hint))
                    && val.is_object()
                {
                    return Some(val);
                }
                if let Some(found) = find_object(val, hints) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(|v| find_object(v, hints)),
        _ => None,
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
            }
            None
        }
        _ => None,
    }
}

fn find_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|v| v.as_str()).map(str::to_string))
}

fn cookie_header(raw: &str) -> String {
    raw.strip_prefix("Cookie:")
        .or_else(|| raw.strip_prefix("cookie:"))
        .unwrap_or(raw)
        .trim()
        .to_string()
}
