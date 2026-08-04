use anyhow::{Context, Result};
use chrono::Utc;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const OPENCODE_SERVER_URL: &str = "https://opencode.ai/_server";
const OPENCODE_WORKSPACES_FUNCTION_ID: &str =
    "def39973159c7f0483d8793a822b8dbb10d067e12c65455fcb4608459ba0234f";
const OPENCODE_SUBSCRIPTION_FUNCTION_ID: &str =
    "7abeebee372f304e050aaaf92be863f4a86490e382f8c79db68fd94040d691b4";
const OPENCODE_GO_SUBSCRIPTION_FUNCTION_ID: &str =
    "c83b78a614689c38ebee981f9b39a8b377716db85c1fd7dbab604adc02d3313d";
static SERVER_INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct OpenCodeProvider {
    client: reqwest::Client,
    workspace_id: Mutex<Option<String>>,
    auth_cookie: Mutex<Option<String>>,
    auto_webview_login: bool,
}

impl OpenCodeProvider {
    pub fn new(
        workspace_id: Option<String>,
        auth_cookie: Option<String>,
        auto_webview_login: bool,
        proxy: Option<&str>,
    ) -> Self {
        Self {
            client: http_client(proxy),
            workspace_id: Mutex::new(workspace_id.and_then(|value| normalize_workspace_id(&value))),
            auth_cookie: Mutex::new(auth_cookie.and_then(|value| normalize_auth_cookie(&value))),
            auto_webview_login,
        }
    }

    fn configured_workspace_id(&self) -> Option<String> {
        self.workspace_id
            .lock()
            .clone()
            .or_else(|| {
                crate::secrets::get("opencode", "workspace_id")
                    .ok()
                    .flatten()
                    .and_then(|value| normalize_workspace_id(&value))
            })
            .or_else(|| {
                std::env::var("OPENCODE_WORKSPACE_ID")
                    .or_else(|_| std::env::var("CODEXBAR_OPENCODEGO_WORKSPACE_ID"))
                    .or_else(|_| std::env::var("CODEXBAR_OPENCODE_WORKSPACE_ID"))
                    .or_else(|_| std::env::var("CODEXBAR_OPENCODE_GO_WORKSPACE_ID"))
                    .ok()
                    .and_then(|value| normalize_workspace_id(&value))
            })
    }

    fn remember_workspace_id(&self, workspace_id: &str) {
        *self.workspace_id.lock() = Some(workspace_id.to_string());
        if let Err(err) = crate::secrets::set("opencode", "workspace_id", workspace_id) {
            tracing::warn!("Failed to remember OpenCode workspace ID: {err}");
        }
    }

    fn webview_target_url(&self) -> Option<String> {
        self.configured_workspace_id()
            .map(|workspace_id| format!("https://opencode.ai/workspace/{workspace_id}/go"))
    }

    fn remember_auth_cookie(&self, auth_cookie: &str) {
        *self.auth_cookie.lock() = normalize_auth_cookie(auth_cookie);
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

    async fn resolve_cookie_header(&self) -> Result<String> {
        if let Some(cookie) = self.auth_cookie.lock().clone() {
            return Ok(cookie);
        }

        if let Ok(cookie) = std::env::var("OPENCODE_AUTH_COOKIE")
            && let Some(cookie) = normalize_auth_cookie(&cookie)
        {
            return Ok(cookie);
        }

        if let Ok(Some(cookie)) = crate::secrets::get("opencode", "auth_cookie")
            && let Some(cookie) = normalize_auth_cookie(&cookie)
        {
            return Ok(cookie);
        }
        if let Ok(Some(cookie)) = crate::secrets::get("opencodego", "auth_cookie")
            && let Some(cookie) = normalize_auth_cookie(&cookie)
        {
            return Ok(cookie);
        }

        anyhow::bail!(
            "OpenCode auth cookie not found. Please configure it in your config file or set the OPENCODE_AUTH_COOKIE env var."
        )
    }
}

#[async_trait::async_trait]
impl Provider for OpenCodeProvider {
    fn name(&self) -> &str {
        "opencode"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let cookie_header = match self.resolve_cookie_header().await {
            Ok(cookie) => cookie,
            Err(_) if self.auto_webview_login => {
                tracing::info!("OpenCode credentials missing. Attempting WebView2 login...");
                let cookie = crate::webview_login::login_and_store_async_at(
                    "opencode",
                    false,
                    self.webview_target_url(),
                )
                .await?;
                self.remember_auth_cookie(&cookie);
                cookie
            }
            Err(err) => {
                return Err(crate::webview_login::login_required_error(self.name(), err));
            }
        };

        let (windows, credits) = match self.fetch_via_server_cookie(&cookie_header).await {
            Ok(result) => result,
            Err(err) if is_webview_auth_failure(&err) && self.auto_webview_login => {
                tracing::info!(
                    "OpenCode credentials rejected. Reusing the WebView2 browser session..."
                );
                let fresh_cookie = crate::webview_login::login_and_store_async_at(
                    "opencode",
                    false,
                    self.webview_target_url(),
                )
                .await?;
                self.remember_auth_cookie(&fresh_cookie);
                match self.fetch_via_server_cookie(&fresh_cookie).await {
                    Ok(result) => result,
                    Err(retry_err) => {
                        return Err(crate::webview_login::login_required_error(
                            self.name(),
                            format!("automatic login did not restore access: {retry_err:#}"),
                        ));
                    }
                }
            }
            Err(err) if is_webview_auth_failure(&err) => {
                return Err(crate::webview_login::login_required_error(
                    self.name(),
                    safe_auth_failure_summary(&err),
                ));
            }
            Err(err) => {
                return Err(err.context(format!(
                    "Failed to fetch {} usage",
                    super::display_name(self.name())
                )));
            }
        };

        Ok(UsageData {
            provider: self.name().to_string(),
            windows,
            credits,
            subscription_tier: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn is_webview_auth_failure(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}").to_ascii_lowercase();
    [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "returned html",
        "did not contain usage data",
        "workspace id not found",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn safe_auth_failure_summary(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    let lower = text.to_ascii_lowercase();
    if lower.contains("workspace id not found") {
        return "the saved session did not expose an OpenCode workspace".to_string();
    }
    if lower.contains("did not contain usage data") {
        let shapes = regex::Regex::new(r"kind=[a-z]+ length=\d+ markers=[a-z,-]+")
            .ok()
            .map(|regex| {
                regex
                    .find_iter(&text)
                    .map(|matched| matched.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return if shapes.is_empty() {
            "the authenticated response did not contain recognizable usage data".to_string()
        } else {
            format!(
                "the authenticated response did not contain recognizable usage data ({})",
                shapes.join("; ")
            )
        };
    }
    if lower.contains("401") || lower.contains("unauthorized") {
        return "the saved credentials were rejected with HTTP 401".to_string();
    }
    if lower.contains("403") || lower.contains("forbidden") {
        return "the saved credentials were rejected with HTTP 403".to_string();
    }
    if lower.contains("returned html") {
        return "the saved session was redirected to an HTML login page".to_string();
    }
    "the saved credentials were rejected".to_string()
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
                .call_server_function(
                    cookie_header,
                    OPENCODE_WORKSPACES_FUNCTION_ID,
                    &[],
                    "https://opencode.ai/",
                )
                .await
                .context("Failed to fetch OpenCode workspaces")?;
            parse_workspace_id(&body).context("OpenCode workspace ID not found")?
        };
        self.remember_workspace_id(&workspace_id);

        match self
            .fetch_go_page(cookie_header, &workspace_id)
            .await
            .with_context(|| format!("Failed to fetch OpenCode Go page for {workspace_id}"))
        {
            Ok(result) => return Ok(result),
            Err(err) => tracing::debug!("OpenCode Go page fetch failed: {err:#}"),
        }

        let go_referer = format!("https://opencode.ai/workspace/{workspace_id}/go");
        let go_result = self
            .call_server_function(
                cookie_header,
                OPENCODE_GO_SUBSCRIPTION_FUNCTION_ID,
                &[serde_json::Value::String(workspace_id.clone())],
                &go_referer,
            )
            .await;
        let go_failure = match go_result {
            Ok(body) => {
                tracing::debug!(
                    "OpenCode Go server response shape: {}",
                    response_shape(&body)
                );
                let result = parse_subscription_usage(&body);
                if !result.0.is_empty() {
                    return Ok(result);
                }
                format!(
                    "response did not contain usage data ({})",
                    response_shape(&body)
                )
            }
            Err(err) => format!("{err:#}"),
        };
        tracing::debug!("OpenCode Go subscription fetch failed: {go_failure}");

        let billing_referer = format!("https://opencode.ai/workspace/{workspace_id}/billing");
        let body = self
            .call_server_function(
                cookie_header,
                OPENCODE_SUBSCRIPTION_FUNCTION_ID,
                &[serde_json::Value::String(workspace_id.clone())],
                &billing_referer,
            )
            .await
            .with_context(|| {
                format!(
                    "OpenCode Go usage failed ({go_failure}); standard OpenCode usage also failed"
                )
            })?;
        tracing::debug!(
            "Standard OpenCode server response shape: {}",
            response_shape(&body)
        );

        let (windows, credits) = parse_subscription_usage(&body);
        if windows.is_empty() {
            anyhow::bail!(
                "OpenCode Go usage failed ({go_failure}); standard OpenCode response did not contain usage data ({})",
                response_shape(&body)
            );
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
        if looks_signed_out(&body) {
            anyhow::bail!("OpenCode Go page unauthorized: signed-out session");
        }
        if body.starts_with("<!DOCTYPE") && !body.contains("rollingUsage") {
            anyhow::bail!("OpenCode Go page did not contain usage data");
        }

        tracing::debug!("OpenCode Go page response shape: {}", response_shape(&body));

        let (windows, credits) = parse_subscription_usage(&body);
        if windows.is_empty() {
            anyhow::bail!(
                "OpenCode Go page response did not contain usage data ({})",
                response_shape(&body)
            );
        }

        Ok((windows, credits))
    }

    async fn call_server_function(
        &self,
        cookie_header: &str,
        function_id: &str,
        args: &[serde_json::Value],
        referer: &str,
    ) -> Result<String> {
        let args_json = serde_json::to_string(args)?;
        let mut get_request = self
            .client
            .get(OPENCODE_SERVER_URL)
            .query(&[("id", function_id)]);
        if !args.is_empty() {
            get_request = get_request.query(&[("args", args_json.as_str())]);
        }

        let get_request = get_request
            .header("X-Server-Id", function_id)
            .header(
                "X-Server-Instance",
                server_instance_id(),
            )
            .header("Cookie", cookie_header)
            .header("Origin", "https://opencode.ai")
            .header("Referer", referer)
            .header(
                "Accept",
                "text/javascript, application/json;q=0.9, */*;q=0.8",
            )
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36",
            );

        let mut last_error = match get_request.send().await {
            Ok(resp) if resp.status().is_success() => {
                let body = resp.text().await.unwrap_or_default();
                if looks_signed_out(&body) {
                    anyhow::bail!("OpenCode server unauthorized: signed-out session");
                }
                if !looks_like_html(&body) {
                    return Ok(body);
                }
                Some("OpenCode server returned HTML".to_string())
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                Some(format!("OpenCode server error {status}: {body}"))
            }
            Err(err) => Some(err.to_string()),
        };

        let form_body = if args.is_empty() {
            vec![("id", function_id.to_string())]
        } else {
            vec![("id", function_id.to_string()), ("args", args_json.clone())]
        };

        let attempts = [
            // 1. Modern Vinxi/SolidStart format: just the JSON array of arguments
            ServerPayload::Json(serde_json::Value::Array(args.to_vec())),
            // 2. Legacy format: {"id": ..., "args": ...}
            ServerPayload::Json(json_body(function_id, args)),
            // 3. Form format (legacy)
            ServerPayload::Form(form_body),
        ];

        for payload in attempts {
            let mut request = self
                .client
                .post(OPENCODE_SERVER_URL)
                .header("x-server-id", function_id)
                .header("x-server-instance", server_instance_id())
                .header("Cookie", cookie_header)
                .header("Origin", "https://opencode.ai")
                .header("Referer", referer)
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
                    if looks_signed_out(&body) {
                        anyhow::bail!("OpenCode server unauthorized: signed-out session");
                    }
                    if looks_like_html(&body) {
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

fn looks_like_html(body: &str) -> bool {
    let trimmed = body.trim_start();
    trimmed.starts_with("<!DOCTYPE") || trimmed.starts_with("<html")
}

fn looks_signed_out(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    [
        "auth/authorize",
        "sign in",
        "not associated with an account",
        "actor of type \"public\"",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn response_shape(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "kind=empty length=0".to_string();
    }

    let kind = if looks_like_html(trimmed) {
        "html"
    } else if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        "json"
    } else {
        "text"
    };
    let lower = trimmed.to_ascii_lowercase();
    let mut markers = Vec::new();
    for (name, aliases) in [
        (
            "rolling",
            ["rollingusage", "rolling_usage", "rollingwindow"],
        ),
        ("weekly", ["weeklyusage", "weekly_usage", "weeklywindow"]),
        (
            "monthly",
            ["monthlyusage", "monthly_usage", "monthlywindow"],
        ),
        ("workspace", ["wrk_", "\"workspace\"", "workspaceid"]),
        (
            "signed-out",
            ["auth/authorize", "sign in", "actor of type \"public\""],
        ),
    ] {
        if aliases.iter().any(|alias| lower.contains(alias)) {
            markers.push(name);
        }
    }
    if trimmed.eq_ignore_ascii_case("null") {
        markers.push("null");
    }

    format!(
        "kind={kind} length={} markers={}",
        trimmed.len(),
        if markers.is_empty() {
            "none".to_string()
        } else {
            markers.join(",")
        }
    )
}

fn server_instance_id() -> String {
    let timestamp = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let counter = SERVER_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("server-fn:quotify-{timestamp:x}-{counter:x}")
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
        return None;
    }

    let auth_cookies = cookie
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            let (name, value) = part.split_once('=')?;
            if matches!(
                name.trim().to_ascii_lowercase().as_str(),
                "auth" | "__host-auth"
            ) && !value.trim().is_empty()
            {
                Some(format!("{}={}", name.trim(), value.trim()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if !auth_cookies.is_empty() {
        Some(auth_cookies.join("; "))
    } else if cookie.contains('=') {
        None
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
        if let Some((pct, reset)) = usage_from_json(
            &json,
            &[
                "rollingUsage",
                "rolling",
                "rolling_usage",
                "rollingWindow",
                "rolling_window",
            ],
        ) {
            windows.push(usage_window("Rolling Usage", pct, reset));
        }
        if let Some((pct, reset)) = usage_from_json(
            &json,
            &[
                "weeklyUsage",
                "weekly",
                "weekly_usage",
                "weeklyWindow",
                "weekly_window",
            ],
        ) {
            windows.push(usage_window("Weekly Usage", pct, reset));
        }
        if let Some((pct, reset)) = usage_from_json(
            &json,
            &[
                "monthlyUsage",
                "monthly",
                "monthly_usage",
                "monthlyWindow",
                "monthly_window",
            ],
        ) {
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
        for (label, aliases) in [
            (
                "Rolling Usage",
                &["rollingUsage", "rolling", "rolling_usage", "rollingWindow"][..],
            ),
            (
                "Weekly Usage",
                &["weeklyUsage", "weekly", "weekly_usage", "weeklyWindow"][..],
            ),
            (
                "Monthly Usage",
                &["monthlyUsage", "monthly", "monthly_usage", "monthlyWindow"][..],
            ),
        ] {
            if let Some((percent, reset)) = usage_from_text(body, aliases) {
                windows.push(usage_window(label, percent, reset));
            }
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
        // Callers normalize their source-specific units before constructing a
        // display window. Applying that conversion here would turn a valid
        // one-percent value into 100 percent.
        used_percent: pct.clamp(0.0, 100.0),
        limit: None,
        used: None,
        unit: None,
        resets_at: reset_in_sec
            .map(|sec| chrono::Utc::now() + chrono::Duration::seconds(sec as i64)),
    }
}

fn normalize_percent(percent: f64) -> f64 {
    let percent = if (0.0..=1.0).contains(&percent) {
        percent * 100.0
    } else {
        percent
    };
    percent.clamp(0.0, 100.0)
}

fn usage_from_json(json: &serde_json::Value, aliases: &[&str]) -> Option<(f64, Option<f64>)> {
    let usage = find_object_by_keys(json, aliases)?;
    parse_window_object(usage)
}

fn parse_window_object(
    usage: &serde_json::Map<String, serde_json::Value>,
) -> Option<(f64, Option<f64>)> {
    const PERCENT_KEYS: &[&str] = &[
        "usagePercent",
        "usedPercent",
        "percentUsed",
        "percent",
        "usage_percent",
        "used_percent",
        "utilization",
        "utilizationPercent",
        "utilization_percent",
        "usage",
    ];
    const RESET_KEYS: &[&str] = &[
        "resetInSec",
        "resetInSeconds",
        "resetSeconds",
        "reset_sec",
        "reset_in_sec",
        "resetsInSec",
        "resetsInSeconds",
        "resetIn",
        "resetSec",
    ];
    const RESET_AT_KEYS: &[&str] = &[
        "resetAt",
        "resetsAt",
        "reset_at",
        "resets_at",
        "nextReset",
        "next_reset",
        "renewAt",
        "renew_at",
    ];

    // OpenCode's original `usagePercent` field is already expressed in percent
    // units, including values such as `1` for one percent. The broader parser
    // below also accepts fraction-style aliases, so retain the matched field's
    // unit instead of treating this specific OpenCode field as a fraction.
    let opencode_usage_percent = map_number_by_keys(usage, &["usagePercent"]);
    let direct_percent = opencode_usage_percent.or_else(|| map_number_by_keys(usage, PERCENT_KEYS));
    let percent = direct_percent.or_else(|| {
        let used =
            map_number_by_keys(usage, &["used", "usage", "consumed", "count", "usedTokens"])?;
        let limit = map_number_by_keys(
            usage,
            &["limit", "total", "quota", "max", "cap", "tokenLimit"],
        )?;
        (limit > 0.0).then_some((used / limit) * 100.0)
    })?;
    let percent = if opencode_usage_percent.is_some() {
        percent.clamp(0.0, 100.0)
    } else if direct_percent.is_some() {
        normalize_percent(percent)
    } else {
        percent.clamp(0.0, 100.0)
    };
    let reset = map_number_by_keys(usage, RESET_KEYS)
        .or_else(|| map_value_by_keys(usage, RESET_AT_KEYS).and_then(reset_seconds_from_value));
    Some((percent, reset))
}

fn find_object_by_keys<'a>(
    value: &'a serde_json::Value,
    keys: &[&str],
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(obj) = map.iter().find_map(|(key, value)| {
                keys.iter()
                    .any(|candidate| key.eq_ignore_ascii_case(candidate))
                    .then(|| value.as_object())
                    .flatten()
            }) {
                return Some(obj);
            }
            map.values()
                .find_map(|child| find_object_by_keys(child, keys))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| find_object_by_keys(child, keys)),
        _ => None,
    }
}

fn map_value_by_keys<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    map.iter().find_map(|(key, value)| {
        keys.iter()
            .any(|candidate| key.eq_ignore_ascii_case(candidate))
            .then_some(value)
    })
}

fn map_number_by_keys(
    map: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<f64> {
    map_value_by_keys(map, keys).and_then(number_value)
}

fn reset_seconds_from_value(value: &serde_json::Value) -> Option<f64> {
    if let Some(number) = number_value(value) {
        let timestamp = if number > 1_000_000_000_000.0 {
            number / 1000.0
        } else if number > 1_000_000_000.0 {
            number
        } else {
            return None;
        };
        return Some((timestamp - Utc::now().timestamp() as f64).max(0.0));
    }
    let timestamp = value.as_str()?;
    let reset_at = chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .with_timezone(&Utc);
    Some((reset_at - Utc::now()).num_seconds().max(0) as f64)
}

fn usage_from_text(body: &str, outer_keys: &[&str]) -> Option<(f64, Option<f64>)> {
    const PERCENT_KEYS: &[&str] = &[
        "usagePercent",
        "usedPercent",
        "percentUsed",
        "percent",
        "usage_percent",
        "used_percent",
        "utilization",
        "utilizationPercent",
        "utilization_percent",
    ];
    const RESET_KEYS: &[&str] = &[
        "resetInSec",
        "resetInSeconds",
        "resetSeconds",
        "reset_sec",
        "reset_in_sec",
        "resetsInSec",
        "resetsInSeconds",
        "resetIn",
        "resetSec",
    ];

    let (percent_field, percent) = outer_keys.iter().find_map(|outer_key| {
        PERCENT_KEYS.iter().find_map(|field| {
            regex_after_key_f64(body, outer_key, field).map(|percent| (*field, percent))
        })
    })?;
    let reset = outer_keys.iter().find_map(|outer_key| {
        RESET_KEYS
            .iter()
            .find_map(|field| regex_after_key_f64(body, outer_key, field))
    });
    let percent = if percent_field.eq_ignore_ascii_case("usagePercent") {
        percent.clamp(0.0, 100.0)
    } else {
        normalize_percent(percent)
    };
    Some((percent, reset))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_webview_auth_failures() {
        assert!(is_webview_auth_failure(&anyhow::anyhow!(
            "OpenCode server error 401 Unauthorized"
        )));
        assert!(is_webview_auth_failure(&anyhow::anyhow!(
            "OpenCode server returned HTML"
        )));
        assert!(is_webview_auth_failure(&anyhow::anyhow!(
            "OpenCode workspace ID not found"
        )));
        assert!(!is_webview_auth_failure(&anyhow::anyhow!(
            "connection timed out"
        )));
        assert!(looks_signed_out(
            r#"Error: actor of type "public" is not associated with an account"#
        ));
    }

    #[test]
    fn normalizes_only_opencode_auth_cookies() {
        assert_eq!(
            normalize_auth_cookie("theme=dark; auth=one; __Host-auth=two; analytics=x").as_deref(),
            Some("auth=one; __Host-auth=two")
        );
        assert_eq!(
            normalize_auth_cookie("raw-token").as_deref(),
            Some("auth=raw-token")
        );
        assert!(normalize_auth_cookie("theme=dark").is_none());
    }

    #[test]
    fn parses_current_json_usage_aliases_and_reset_timestamps() {
        let body = r#"{
            "data": {
                "rolling_window": {
                    "used_percent": 0.25,
                    "reset_in_sec": 120
                },
                "weekly": {
                    "used": 40,
                    "limit": 100,
                    "resets_at": "2099-01-01T00:00:00Z"
                },
                "monthlyUsage": {
                    "utilizationPercent": "75",
                    "resetInSeconds": "3600"
                }
            }
        }"#;

        let (windows, _) = parse_subscription_usage(body);
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].used_percent, 25.0);
        assert_eq!(windows[1].used_percent, 40.0);
        assert_eq!(windows[2].used_percent, 75.0);
        assert!(windows.iter().all(|window| window.resets_at.is_some()));
    }

    #[test]
    fn preserves_opencode_usage_percent_unit_at_one_percent() {
        let body = r#"{
            "rollingUsage": {
                "usagePercent": 1,
                "resetInSec": 120
            }
        }"#;

        let (windows, _) = parse_subscription_usage(body);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].label, "Rolling Usage");
        assert_eq!(windows[0].used_percent, 1.0);
    }

    #[test]
    fn preserves_usage_percent_units_in_text_payloads_for_all_windows() {
        let body = r#"
            rollingUsage: $R[1] = { usagePercent: 1, resetInSec: 120 }
            weeklyUsage: $R[2] = { usagePercent: 1, resetInSec: 240 }
            monthlyUsage: $R[3] = { usagePercent: 13, resetInSec: 360 }
        "#;

        let (windows, _) = parse_subscription_usage(body);
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].used_percent, 1.0);
        assert_eq!(windows[1].used_percent, 1.0);
        assert_eq!(windows[2].used_percent, 13.0);
    }
}
