use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;

use super::{CreditsInfo, Provider, UsageData, UsageWindow};
use crate::cookies;

const OPENCODE_SERVER_URL: &str = "https://opencode.ai/_server";
const OPENCODE_WORKSPACES_FUNCTION_ID: &str =
    "6e46dc687363358d99b3aff307cf93451c5f4ea8930ccf7d419eb45d6653ea1b";
const OPENCODE_SUBSCRIPTION_FUNCTION_ID: &str =
    "7abeebee372f304e050aaaf92be863f4a86490e382f8c79db68fd94040d691b4";

pub struct OpenCodeProvider {
    api_key: Option<String>,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct OpenCodeAuthEntry {
    #[serde(rename = "type")]
    auth_type: Option<String>,
    key: Option<String>,
    access: Option<String>,
    #[expect(dead_code)]
    refresh: Option<String>,
    #[serde(default)]
    #[expect(dead_code)]
    expires: Option<i64>,
}

#[expect(dead_code)]
struct OpenCodeBalanceResponse {
    balance: Option<f64>,
    credits: Option<f64>,
    total: Option<f64>,
}

struct SessionStats {
    total_cost: f64,
    total_input: i64,
    total_output: i64,
    total_cache_read: i64,
    #[expect(dead_code)]
    total_cache_write: i64,
    #[expect(dead_code)]
    session_count: i64,
}

impl OpenCodeProvider {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }

    fn resolve_api_key(&self) -> Option<String> {
        if let Some(ref key) = self.api_key
            && !key.is_empty() {
                return Some(key.clone());
            }

        if let Ok(key) = std::env::var("OPENCODE_API_KEY")
            && !key.is_empty() {
                return Some(key);
            }

        Self::read_auth_file()
    }

    fn read_auth_file() -> Option<String> {
        let home = dirs::home_dir()?;
        let path = home
            .join(".local")
            .join("share")
            .join("opencode")
            .join("auth.json");
        if !path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&path).ok()?;
        let auth: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&content).ok()?;

        let priority_keys = ["opencode-go", "opencode", "default"];
        for pk in &priority_keys {
            if let Some(entry_val) = auth.get(*pk)
                && let Ok(entry) = serde_json::from_value::<OpenCodeAuthEntry>(entry_val.clone()) {
                    if entry.auth_type.as_deref() == Some("api")
                        && let Some(key) = entry.key.filter(|k| !k.is_empty()) {
                            return Some(key);
                        }
                    if entry.auth_type.as_deref() == Some("oauth")
                        && let Some(access) = entry.access.filter(|k| !k.is_empty()) {
                            return Some(access);
                        }
                }
        }

        for (_name, entry_val) in &auth {
            if let Ok(entry) = serde_json::from_value::<OpenCodeAuthEntry>(entry_val.clone())
                && entry.auth_type.as_deref() == Some("api")
                    && let Some(key) = entry.key.filter(|k| !k.is_empty()) {
                        return Some(key);
                    }
        }

        None
    }

    fn is_go_key(key: &str) -> bool {
        key.starts_with("sk-opcode-go-") || key.contains("-go-")
    }

    fn has_go_auth() -> bool {
        let home = dirs::home_dir().unwrap_or_default();
        let path = home
            .join(".local")
            .join("share")
            .join("opencode")
            .join("auth.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            content.contains("opencode-go")
        } else {
            false
        }
    }

    fn configured_workspace_id() -> Option<String> {
        std::env::var("OPENCODE_WORKSPACE_ID")
            .or_else(|_| std::env::var("CODEXBAR_OPENCODE_WORKSPACE_ID"))
            .ok()
            .and_then(|value| normalize_workspace_id(&value))
    }
}

#[async_trait::async_trait]
impl Provider for OpenCodeProvider {
    fn name(&self) -> &str {
        "opencode"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self.resolve_api_key();
        let mut windows = Vec::new();
        let mut credits = None;

        // Method 1: Try authenticated opencode.ai dashboard server functions.
        if let Ok(cookie_header) = cookies::find_cookie_header(&["opencode.ai", ".opencode.ai"]) {
            tracing::debug!("Found opencode.ai browser cookies, trying server functions");
            match self.fetch_via_server_cookie(&cookie_header).await {
                Ok((w, c)) => {
                    windows = w;
                    credits = c;
                }
                Err(e) => {
                    tracing::debug!("OpenCode server cookie fetch failed: {e}");
                }
            }
        }

        // Method 2: Try API endpoint (if key available)
        if let Some(ref key) = api_key
            && windows.is_empty() {
                let base = if Self::is_go_key(key) || Self::has_go_auth() {
                    "https://opencode.ai/zen/go/v1"
                } else {
                    "https://opencode.ai/zen/v1"
                };

                let endpoints = vec![
                    format!("{}/balance", base),
                    format!("{}/usage", base),
                    "https://opencode.ai/api/v1/balance".to_string(),
                    "https://opencode.ai/api/v1/usage".to_string(),
                ];

                for endpoint in &endpoints {
                    let resp = self
                        .client
                        .get(endpoint)
                        .header("Authorization", format!("Bearer {key}"))
                        .send()
                        .await;

                    match resp {
                        Ok(r) if r.status().is_success() => {
                            if let Ok(value) = r.json::<serde_json::Value>().await {
                                tracing::debug!(
                                    "OpenCode API response from {endpoint}: {value:#?}"
                                );

                                let total = value
                                    .get("total")
                                    .or_else(|| value.get("total_credits"))
                                    .or_else(|| value.get("limit"))
                                    .and_then(|v| v.as_f64());

                                let used = value
                                    .get("used")
                                    .or_else(|| value.get("used_credits"))
                                    .and_then(|v| v.as_f64());

                                let balance_val = value
                                    .get("balance")
                                    .or_else(|| value.get("remaining"))
                                    .or_else(|| value.get("credits"))
                                    .and_then(|v| v.as_f64());

                                if let Some(bal) = balance_val {
                                    let total_with_used =
                                        total.unwrap_or(bal + used.unwrap_or(0.0));
                                    let used_pct = if total_with_used > 0.0 {
                                        (used.unwrap_or(0.0) / total_with_used * 100.0).min(100.0)
                                    } else {
                                        0.0
                                    };

                                    credits = Some(CreditsInfo {
                                        balance: bal,
                                        currency: "USD".to_string(),
                                        total_granted: Some(total_with_used),
                                        topped_up: None,
                                    });

                                    windows.push(UsageWindow {
                                        label: "Usage".to_string(),
                                        used_percent: used_pct,
                                        limit: Some(total_with_used),
                                        used,
                                        unit: Some("USD".to_string()),
                                        resets_at: None,
                                    });
                                    break;
                                } else if let Some(tot) = total {
                                    let used_pct = if tot > 0.0 {
                                        (used.unwrap_or(0.0) / tot * 100.0).min(100.0)
                                    } else {
                                        0.0
                                    };
                                    windows.push(UsageWindow {
                                        label: "Usage".to_string(),
                                        used_percent: used_pct,
                                        limit: Some(tot),
                                        used,
                                        unit: Some("USD".to_string()),
                                        resets_at: None,
                                    });
                                    break;
                                }
                            }
                        }
                        Ok(r) => {
                            tracing::debug!("OpenCode endpoint {endpoint} returned {}", r.status());
                            continue;
                        }
                        Err(e) => {
                            tracing::debug!("OpenCode endpoint {endpoint} error: {e}");
                            continue;
                        }
                    }
                }
            }


        if windows.is_empty() {
            windows.push(UsageWindow {
                label: "No data".to_string(),
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
            credits,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

impl OpenCodeProvider {
    async fn fetch_via_server_cookie(
        &self,
        cookie_header: &str,
    ) -> Result<(Vec<UsageWindow>, Option<CreditsInfo>)> {
        let workspace_id = if let Some(workspace_id) = Self::configured_workspace_id() {
            workspace_id
        } else {
            let body = self
                .post_server_function(cookie_header, OPENCODE_WORKSPACES_FUNCTION_ID, &[])
                .await
                .context("Failed to fetch OpenCode workspaces")?;
            parse_workspace_id(&body).context("OpenCode workspace ID not found")?
        };

        let body = self
            .post_server_function(
                cookie_header,
                OPENCODE_SUBSCRIPTION_FUNCTION_ID,
                &[serde_json::Value::String(workspace_id.clone())],
            )
            .await
            .with_context(|| format!("Failed to fetch OpenCode subscription for {workspace_id}"))?;

        tracing::debug!(
            "OpenCode server response (first 500 chars): {}",
            &body[..body.len().min(500)]
        );

        let (windows, credits) = parse_subscription_usage(&body);
        if windows.is_empty() {
            anyhow::bail!("OpenCode subscription response did not contain usage data");
        }

        Ok((windows, credits))
    }

    async fn post_server_function(
        &self,
        cookie_header: &str,
        function_id: &str,
        args: &[serde_json::Value],
    ) -> Result<String> {
        let args_json = serde_json::to_string(args)?;
        let form_body = if args.is_empty() {
            vec![("id", function_id.to_string())]
        } else {
            vec![("id", function_id.to_string()), ("args", args_json.clone())]
        };

        let attempts = [
            ServerPayload::Form(form_body),
            ServerPayload::Json(json_body(function_id, args)),
        ];

        let mut last_error = None;
        for payload in attempts {
            let mut request = self
                .client
                .post(OPENCODE_SERVER_URL)
                .header("Cookie", cookie_header)
                .header("Origin", "https://opencode.ai")
                .header("Referer", "https://opencode.ai/")
                .header(
                    "Accept",
                    "text/javascript, application/json, text/plain, */*",
                )
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
                );

            request = match payload {
                ServerPayload::Form(form) => request.form(&form),
                ServerPayload::Json(body) => request.json(&body),
            };

            match request.send().await {
                Ok(resp) if resp.status().is_success() => {
                    let body = resp.text().await.unwrap_or_default();
                    if body.starts_with("<!DOCTYPE") || body.starts_with("<html") {
                        last_error = Some("OpenCode server returned HTML".to_string());
                    } else {
                        return Ok(body);
                    }
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    last_error = Some(format!("OpenCode server error {status}: {body}"));
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                }
            }
        }

        anyhow::bail!(
            "{}",
            last_error.unwrap_or_else(|| "OpenCode server call failed".to_string())
        )
    }
}

enum ServerPayload {
    Form(Vec<(&'static str, String)>),
    Json(serde_json::Value),
}

fn json_body(function_id: &str, args: &[serde_json::Value]) -> serde_json::Value {
    serde_json::json!({
        "id": function_id,
        "args": args,
    })
}

fn normalize_workspace_id(value: &str) -> Option<String> {
    regex::Regex::new(r#"wrk_[A-Za-z0-9_-]+"#)
        .ok()?
        .find(value)
        .map(|m| m.as_str().to_string())
}

fn parse_workspace_id(body: &str) -> Option<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(found) = find_string_matching(&json, |s| s.starts_with("wrk_")) {
            return Some(found);
        }

    normalize_workspace_id(body)
}

fn parse_subscription_usage(body: &str) -> (Vec<UsageWindow>, Option<CreditsInfo>) {
    let mut windows = Vec::new();
    let mut credits = None;

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some((pct, reset)) = usage_from_json(&json, "rollingUsage") {
            windows.push(usage_window("Rolling Usage", pct, reset));
        }
        if let Some((pct, reset)) = usage_from_json(&json, "weeklyUsage") {
            windows.push(usage_window("Weekly Usage", pct, reset));
        }

        if let Some(balance) = find_number_by_keys(&json, &["credits", "balance"]) {
            credits = Some(CreditsInfo {
                balance,
                currency: "USD".to_string(),
                total_granted: None,
                topped_up: None,
            });
        }
    }

    if windows.is_empty() {
        let rolling_pct = regex_after_key_f64(body, "rollingUsage", "usagePercent");
        let weekly_pct = regex_after_key_f64(body, "weeklyUsage", "usagePercent");
        let rolling_reset = regex_after_key_f64(body, "rollingUsage", "resetInSec");
        let weekly_reset = regex_after_key_f64(body, "weeklyUsage", "resetInSec");

        if let Some(pct) = rolling_pct {
            windows.push(usage_window("Rolling Usage", pct, rolling_reset));
        }
        if let Some(pct) = weekly_pct {
            windows.push(usage_window("Weekly Usage", pct, weekly_reset));
        }

        if credits.is_none()
            && let Some(balance) = regex_extract_f64(
                body,
                r#"(?s)(?:credits|balance)["']?\s*[:=,]\s*(\d+(?:\.\d+)?)"#,
            ) {
                credits = Some(CreditsInfo {
                    balance,
                    currency: "USD".to_string(),
                    total_granted: None,
                    topped_up: None,
                });
            }
    }

    (windows, credits)
}

fn usage_window(label: &str, pct: f64, reset_in_sec: Option<f64>) -> UsageWindow {
    UsageWindow {
        label: label.to_string(),
        used_percent: pct.clamp(0.0, 100.0),
        limit: None,
        used: None,
        unit: None,
        resets_at: reset_in_sec
            .map(|sec| chrono::Utc::now() + chrono::Duration::seconds(sec as i64)),
    }
}

fn usage_from_json(json: &serde_json::Value, key: &str) -> Option<(f64, Option<f64>)> {
    let usage = find_object_by_key(json, key)?;
    let pct = usage.get("usagePercent").and_then(number_value)?;
    let reset = usage.get("resetInSec").and_then(number_value);
    Some((pct, reset))
}

fn find_object_by_key<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(obj) = map.get(key).and_then(|v| v.as_object()) {
                return Some(obj);
            }
            map.values()
                .find_map(|child| find_object_by_key(child, key))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| find_object_by_key(child, key)),
        _ => None,
    }
}

fn find_string_matching(
    value: &serde_json::Value,
    pred: impl Fn(&str) -> bool + Copy,
) -> Option<String> {
    match value {
        serde_json::Value::String(s) if pred(s) => Some(s.to_string()),
        serde_json::Value::Object(map) => map
            .values()
            .find_map(|child| find_string_matching(child, pred)),
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_matching(child, pred)),
        _ => None,
    }
}

fn find_number_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(number) = map.get(*key).and_then(number_value) {
                    return Some(number);
                }
            }
            map.values()
                .find_map(|child| find_number_by_keys(child, keys))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| find_number_by_keys(child, keys)),
        _ => None,
    }
}

fn number_value(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse::<f64>().ok()))
}

fn regex_after_key_f64(text: &str, outer_key: &str, field: &str) -> Option<f64> {
    let pattern = format!(
        r#"(?s){}.*?{}["']?\s*[:=,]\s*(\d+(?:\.\d+)?)"#,
        regex::escape(outer_key),
        regex::escape(field)
    );
    regex_extract_f64(text, &pattern)
}

fn regex_extract_f64(text: &str, pattern: &str) -> Option<f64> {
    let re = regex::Regex::new(pattern).ok()?;
    let caps = re.captures(text)?;
    let val_str = caps.get(1)?.as_str();
    val_str.parse::<f64>().ok()
}
