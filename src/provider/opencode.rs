use anyhow::{Context, Result};
use chrono::Utc;

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const OPENCODE_SERVER_URL: &str = "https://opencode.ai/_server";
const OPENCODE_WORKSPACES_FUNCTION_ID: &str =
    "0c8d84b0a700eb0de440ca4c9105b42d6c9ede971d6bf592fa4f91bbeaaa1e6b";
const OPENCODE_SUBSCRIPTION_FUNCTION_ID: &str =
    "7abeebee372f304e050aaaf92be863f4a86490e382f8c79db68fd94040d691b4";

pub struct OpenCodeProvider {
    provider_name: &'static str,
    client: reqwest::Client,
    workspace_id: Option<String>,
    auth_cookie: Option<String>,
}

impl OpenCodeProvider {
    pub fn new(
        workspace_id: Option<String>,
        auth_cookie: Option<String>,
        proxy: Option<&str>,
    ) -> Self {
        Self::new_with_name("opencode", workspace_id, auth_cookie, proxy)
    }

    pub fn new_with_name(
        provider_name: &'static str,
        workspace_id: Option<String>,
        auth_cookie: Option<String>,
        proxy: Option<&str>,
    ) -> Self {
        Self {
            provider_name,
            client: http_client(proxy),
            workspace_id: workspace_id.and_then(|value| normalize_workspace_id(&value)),
            auth_cookie: auth_cookie.and_then(|value| normalize_auth_cookie(&value)),
        }
    }

    fn configured_workspace_id(&self) -> Option<String> {
        self.workspace_id.clone().or_else(|| {
            std::env::var("OPENCODE_WORKSPACE_ID")
                .or_else(|_| std::env::var("CODEXBAR_OPENCODEGO_WORKSPACE_ID"))
                .or_else(|_| std::env::var("CODEXBAR_OPENCODE_WORKSPACE_ID"))
                .or_else(|_| std::env::var("CODEXBAR_OPENCODE_GO_WORKSPACE_ID"))
                .ok()
                .and_then(|value| normalize_workspace_id(&value))
        })
    }

    pub fn has_workspace_hint() -> bool {
        std::env::var("OPENCODE_WORKSPACE_ID")
            .or_else(|_| std::env::var("CODEXBAR_OPENCODEGO_WORKSPACE_ID"))
            .or_else(|_| std::env::var("CODEXBAR_OPENCODE_WORKSPACE_ID"))
            .or_else(|_| std::env::var("CODEXBAR_OPENCODE_GO_WORKSPACE_ID"))
            .ok()
            .and_then(|value| normalize_workspace_id(&value))
            .is_some()
    }

    pub fn has_auth_cookie_hint() -> bool {
        std::env::var("OPENCODE_AUTH_COOKIE")
            .ok()
            .and_then(|value| normalize_auth_cookie(&value))
            .is_some()
    }

    async fn find_dashboard_cookie_header(&self) -> Result<String> {
        if let Some(cookie) = &self.auth_cookie {
            return Ok(cookie.clone());
        }

        if let Ok(cookie) = std::env::var("OPENCODE_AUTH_COOKIE")
            && let Some(cookie) = normalize_auth_cookie(&cookie)
        {
            return Ok(cookie);
        }

        anyhow::bail!(
            "OpenCode requires auth_cookie in config or OPENCODE_AUTH_COOKIE; automatic browser cookie reading is disabled"
        )
    }
}

#[async_trait::async_trait]
impl Provider for OpenCodeProvider {
    fn name(&self) -> &str {
        self.provider_name
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        // OpenCode Go does not expose a stable public quota API. Use the logged-in
        // dashboard cookies and parse the same server data that the web app loads.
        let cookie_header = self
            .find_dashboard_cookie_header()
            .await
            .context("OpenCode requires auth_cookie in config or OPENCODE_AUTH_COOKIE")?;
        tracing::debug!("Found opencode.ai auth cookie, trying server functions");

        let (windows, credits) = self
            .fetch_via_server_cookie(&cookie_header)
            .await
            .context("Failed to fetch OpenCode Go usage from opencode.ai dashboard")?;

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
        let workspace_id = if let Some(workspace_id) = self.configured_workspace_id() {
            tracing::debug!("Using configured OpenCode workspace ID: {workspace_id}");
            workspace_id
        } else {
            let body = self
                .post_server_function(cookie_header, OPENCODE_WORKSPACES_FUNCTION_ID, &[])
                .await
                .context("Failed to fetch OpenCode workspaces")?;
            parse_workspace_id(&body).context("OpenCode workspace ID not found")?
        };

        match self
            .fetch_go_page(cookie_header, &workspace_id)
            .await
            .with_context(|| format!("Failed to fetch OpenCode Go page for {workspace_id}"))
        {
            Ok(result) => return Ok(result),
            Err(err) => tracing::debug!("OpenCode Go page fetch failed: {err:#}"),
        }

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

    async fn fetch_go_page(
        &self,
        cookie_header: &str,
        workspace_id: &str,
    ) -> Result<(Vec<UsageWindow>, Option<CreditsInfo>)> {
        let url = format!("https://opencode.ai/workspace/{workspace_id}/go");
        let resp = self
            .client
            .get(&url)
            .header("Cookie", cookie_header)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Referer", "https://opencode.ai/")
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            )
            .send()
            .await
            .context("Failed to request OpenCode Go page")?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("OpenCode Go page error {status}: {body}");
        }
        if body.starts_with("<!DOCTYPE") && !body.contains("rollingUsage") {
            anyhow::bail!("OpenCode Go page did not contain usage data");
        }

        tracing::debug!(
            "OpenCode Go page response (first 500 chars): {}",
            &body[..body.len().min(500)]
        );

        let (windows, credits) = parse_subscription_usage(&body);
        if windows.is_empty() {
            anyhow::bail!("OpenCode Go page response did not contain usage data");
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

fn normalize_auth_cookie(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let cookie = trimmed
        .strip_prefix("Cookie:")
        .or_else(|| trimmed.strip_prefix("cookie:"))
        .unwrap_or(trimmed)
        .trim();

    if cookie.is_empty() {
        None
    } else if cookie.contains('=') || cookie.contains(';') {
        Some(cookie.to_string())
    } else {
        Some(format!("auth={cookie}"))
    }
}

fn parse_workspace_id(body: &str) -> Option<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(found) = find_string_matching(&json, |s| s.starts_with("wrk_"))
    {
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
        if let Some((pct, reset)) = usage_from_json(&json, "monthlyUsage") {
            windows.push(usage_window("Monthly Usage", pct, reset));
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
        let monthly_pct = regex_after_key_f64(body, "monthlyUsage", "usagePercent");
        let rolling_reset = regex_after_key_f64(body, "rollingUsage", "resetInSec");
        let weekly_reset = regex_after_key_f64(body, "weeklyUsage", "resetInSec");
        let monthly_reset = regex_after_key_f64(body, "monthlyUsage", "resetInSec");

        if let Some(pct) = rolling_pct {
            windows.push(usage_window("Rolling Usage", pct, rolling_reset));
        }
        if let Some(pct) = weekly_pct {
            windows.push(usage_window("Weekly Usage", pct, weekly_reset));
        }
        if let Some(pct) = monthly_pct {
            windows.push(usage_window("Monthly Usage", pct, monthly_reset));
        }

        if credits.is_none()
            && let Some(balance) = regex_extract_f64(
                body,
                r#"(?s)(?:credits|balance)["']?\s*[:=,]\s*(\d+(?:\.\d+)?)"#,
            )
        {
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
    let strict_pattern = format!(
        r#"(?s)["']?{}["']?\s*[:=]\s*(?:\$R\[\d+\]\s*=\s*)?\{{[^{{}}]*["']?{}["']?\s*[:=,]\s*(\d+(?:\.\d+)?)"#,
        regex::escape(outer_key),
        regex::escape(field)
    );
    regex_extract_f64(text, &strict_pattern).or_else(|| {
        let loose_pattern = format!(
            r#"(?s)["']?{}["']?\s*[:=]\s*(?:\$R\[\d+\]\s*=\s*)?\{{.*?["']?{}["']?\s*[:=,]\s*(\d+(?:\.\d+)?)"#,
            regex::escape(outer_key),
            regex::escape(field)
        );
        regex_extract_f64(text, &loose_pattern)
    })
}

fn regex_extract_f64(text: &str, pattern: &str) -> Option<f64> {
    let re = regex::Regex::new(pattern).ok()?;
    let caps = re.captures(text)?;
    let val_str = caps.get(1)?.as_str();
    val_str.parse::<f64>().ok()
}
