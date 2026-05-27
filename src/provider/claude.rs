use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;

use super::{CreditsInfo, Provider, UsageData, UsageWindow};
use crate::cookies;

pub struct ClaudeProvider {
    credentials_path: Option<String>,
    session_key: Option<String>,
    api_key: Option<String>,
    access_token: Option<String>,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<ClaudeOauth>,
}

#[derive(Debug, Deserialize)]
struct ClaudeOauth {
    #[serde(alias = "accessToken")]
    access_token: Option<String>,
    #[serde(alias = "refreshToken")]
    #[expect(dead_code)]
    refresh_token: Option<String>,
}

#[expect(dead_code)]
#[derive(Debug, Deserialize)]
struct OrgResponse {
    uuid: String,
    name: Option<String>,
}

#[expect(dead_code)]
#[derive(Debug, Deserialize)]
struct UsageResponse {
    daily_usage: Option<WindowUsage>,
    session_limit: Option<WindowUsage>,
}

#[expect(dead_code)]
#[derive(Debug, Deserialize)]
struct WindowUsage {
    used: Option<f64>,
    limit: Option<f64>,
    used_percent: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeWebOrganizationResponse {
    uuid: String,
    name: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeSettingsFile {
    #[serde(default)]
    env: Option<ClaudeSettingsEnv>,
}

#[derive(Debug, Deserialize)]
struct ClaudeSettingsEnv {
    #[serde(rename = "ANTHROPIC_API_KEY")]
    #[serde(default)]
    anthropic_api_key: Option<String>,
    #[serde(rename = "ANTHROPIC_BASE_URL")]
    #[serde(default)]
    anthropic_base_url: Option<String>,
}

#[expect(dead_code)]
#[derive(Debug, Deserialize)]
struct StatsCacheFile {
    #[serde(default)]
    model_usage: Option<serde_json::Map<String, serde_json::Value>>,
}

fn parse_usage_windows(usage: &serde_json::Value) -> Vec<UsageWindow> {
    let mut windows = Vec::new();
    if let Some(obj) = usage.as_object() {
        for (key, value) in obj {
            if let Some(w) = value.as_object() {
                let used_pct = w
                    .get("used_percent")
                    .or_else(|| w.get("percentage"))
                    .or_else(|| w.get("utilization"))
                    .and_then(|v| v.as_f64());

                let used = w.get("used").and_then(|v| v.as_f64());
                let limit = w.get("limit").and_then(|v| v.as_f64());

                let resets_at = w
                    .get("resets_at")
                    .or_else(|| w.get("reset_at"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.to_utc());

                if used_pct.is_some() || used.is_some() || limit.is_some() {
                    windows.push(UsageWindow {
                        label: key.clone(),
                        used_percent: used_pct.unwrap_or(0.0),
                        limit,
                        used,
                        unit: None,
                        resets_at,
                    });
                }
            }
        }
    }
    windows
}

fn parse_percent(value: &serde_json::Value) -> Option<f64> {
    value
        .get("used_percent")
        .or_else(|| value.get("percentage"))
        .or_else(|| value.get("utilization"))
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
}

fn parse_reset_time(value: &serde_json::Value) -> Option<chrono::DateTime<Utc>> {
    value
        .get("resets_at")
        .or_else(|| value.get("reset_at"))
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.to_utc())
}

fn parse_claude_usage_response(
    usage: &serde_json::Value,
) -> (Vec<UsageWindow>, Option<CreditsInfo>) {
    let mut windows = Vec::new();

    let known_windows = [
        ("five_hour", "Session (5h)"),
        ("seven_day", "Weekly"),
        ("seven_day_sonnet", "Weekly (Sonnet)"),
        ("seven_day_opus", "Weekly (Opus)"),
    ];

    for (key, label) in known_windows {
        let Some(value) = usage.get(key) else {
            continue;
        };
        let Some(window) = value.as_object() else {
            continue;
        };

        let value = serde_json::Value::Object(window.clone());
        let used_percent = parse_percent(&value);
        let used = value.get("used").and_then(|v| v.as_f64());
        let limit = value.get("limit").and_then(|v| v.as_f64());

        if used_percent.is_some() || used.is_some() || limit.is_some() {
            windows.push(UsageWindow {
                label: label.to_string(),
                used_percent: used_percent.unwrap_or(0.0),
                limit,
                used,
                unit: None,
                resets_at: parse_reset_time(&value),
            });
        }
    }

    let credits = usage.get("extra_usage").and_then(|extra| {
        let used = extra.get("used_credits")?.as_f64()?;
        let limit = extra
            .get("monthly_limit")
            .or_else(|| extra.get("monthly_credit_limit"))?
            .as_f64()?;
        let currency = extra
            .get("currency")
            .and_then(|v| v.as_str())
            .unwrap_or("USD");
        Some(CreditsInfo {
            balance: (limit - used) / 100.0,
            currency: currency.to_string(),
            total_granted: Some(limit / 100.0),
            topped_up: None,
        })
    });

    if windows.is_empty() {
        windows = parse_usage_windows(usage);
    }

    (windows, credits)
}

impl ClaudeProvider {
    pub fn new(
        credentials_path: Option<String>,
        session_key: Option<String>,
        api_key: Option<String>,
        access_token: Option<String>,
    ) -> Self {
        Self {
            credentials_path,
            session_key,
            api_key,
            access_token,
            client: reqwest::Client::new(),
        }
    }

    fn default_credentials_path() -> Option<std::path::PathBuf> {
        let home = dirs::home_dir()?;
        Some(home.join(".claude").join(".credentials.json"))
    }

    fn read_oauth_token(&self) -> Option<String> {
        let path = self
            .credentials_path
            .as_ref()
            .map(std::path::PathBuf::from)
            .or_else(Self::default_credentials_path)?;

        if !path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&path).ok()?;
        if let Ok(creds) = serde_json::from_str::<CredentialsFile>(&content)
            && let Some(token) = creds
                .claude_ai_oauth
                .and_then(|o| o.access_token)
                .filter(|t| !t.is_empty())
        {
            return Some(token);
        }

        let json: serde_json::Value = serde_json::from_str(&content).ok()?;
        [
            "/claudeAiOauth/accessToken",
            "/claudeAiOauth/access_token",
            "/oauth/accessToken",
            "/oauth/access_token",
            "/accessToken",
            "/access_token",
        ]
        .iter()
        .find_map(|path| {
            json.pointer(path)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })
    }

    fn read_settings(&self) -> Option<(String, Option<String>)> {
        let home = dirs::home_dir()?;
        let settings_path = home.join(".claude").join("settings.json");
        if !settings_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&settings_path).ok()?;
        let settings: ClaudeSettingsFile = serde_json::from_str(&content).ok()?;

        let env = settings.env?;
        let api_key = env.anthropic_api_key.filter(|k| !k.is_empty())?;
        let base_url = env.anthropic_base_url.filter(|u| !u.is_empty());
        Some((api_key, base_url))
    }

    #[allow(dead_code)]
    fn read_stats_cache(&self) -> Option<Vec<UsageWindow>> {
        let home = dirs::home_dir()?;
        let stats_path = home.join(".claude").join("stats-cache.json");
        if !stats_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&stats_path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;

        let mut windows = Vec::new();
        let mut total_input: f64 = 0.0;
        let mut total_output: f64 = 0.0;

        if let Some(model_usage) = json.get("modelUsage").and_then(|v| v.as_object()) {
            for (_model, data) in model_usage {
                if let Some(obj) = data.as_object() {
                    let input = obj
                        .get("inputTokens")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    let output = obj
                        .get("outputTokens")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    total_input += input;
                    total_output += output;
                }
            }
        }

        if total_input > 0.0 || total_output > 0.0 {
            windows.push(UsageWindow {
                label: "Token Usage (local stats)".to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(total_input + total_output),
                unit: Some("tokens".to_string()),
                resets_at: None,
            });
        }

        if windows.is_empty() {
            None
        } else {
            Some(windows)
        }
    }

    fn read_api_key(&self) -> Option<(String, Option<String>)> {
        if let Some(result) = Self::read_settings_env() {
            return Some(result);
        }
        self.read_settings()
    }

    fn read_settings_env() -> Option<(String, Option<String>)> {
        if let Ok(admin_key) = std::env::var("ANTHROPIC_ADMIN_KEY") {
            if !admin_key.is_empty() {
                let base_url = std::env::var("ANTHROPIC_BASE_URL").ok().filter(|u| !u.is_empty());
                return Some((admin_key, base_url));
            }
        }
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())?;
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .ok()
            .filter(|u| !u.is_empty());
        Some((api_key, base_url))
    }

    async fn read_cookie_session_key(&self) -> Option<String> {
        cookies::find_cookie_multiple(&["claude.ai", ".claude.ai"], "sessionKey")
            .await
            .ok()
            .filter(|k| k.starts_with("sk-ant-"))
    }
}

#[async_trait::async_trait]
impl Provider for ClaudeProvider {
    fn name(&self) -> &str {
        "claude"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let mut windows = Vec::new();
        let mut credits = None;
        let mut source = "unknown";

        // Method 1: Try OAuth token (manual config / env var / credentials file)
        let oauth_token = self.access_token.clone()
            .or_else(|| std::env::var("CLAUDE_ACCESS_TOKEN").ok().filter(|t| !t.is_empty()))
            .or_else(|| self.read_oauth_token());

        if let Some(access_token) = oauth_token {
            match self.fetch_via_oauth(&access_token).await {
                Ok(w) => {
                    windows = w;
                    source = "oauth";
                }
                Err(e) => {
                    tracing::debug!("Claude OAuth fetch failed: {e}");
                }
            }
        }

        // Method 2: Try browser cookie / manual cookie (manual config / env var / browser cookie)
        if windows.is_empty() {
            let mut session_key = self.session_key.clone()
                .or_else(|| std::env::var("CLAUDE_SESSION_KEY").ok().filter(|k| !k.is_empty()));

            if session_key.is_none() {
                session_key = self.read_cookie_session_key().await;
            }

            if let Some(session_key) = session_key {
                tracing::debug!("Using Claude sessionKey");
                match self.fetch_via_cookie(&session_key).await {
                    Ok((w, c)) => {
                        windows = w;
                        credits = c;
                        source = "cookie";
                    }
                    Err(e) => {
                        tracing::debug!("Claude cookie fetch failed: {e}");
                    }
                }
            }
        }

        // Method 3: Try API key (manual config / env vars / settings files)
        if windows.is_empty() {
            let api_key_and_url = self.api_key.clone().map(|k| (k, None))
                .or_else(|| self.read_api_key());

            if let Some((api_key, base_url)) = api_key_and_url {
                match self.fetch_via_api_key(&api_key, base_url.as_deref()).await {
                    Ok(w) => {
                        windows = w;
                        source = "api_key";
                    }
                    Err(e) => {
                        tracing::debug!("Claude API key fetch failed: {e}");
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

        for w in &mut windows {
            if w.unit.is_none() && w.label.contains("(local") {
                w.unit = Some("tokens".to_string());
            }
        }

        let _ = source;

        Ok(UsageData {
            provider: self.name().to_string(),
            windows,
            credits,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

impl ClaudeProvider {
    async fn fetch_via_oauth(&self, access_token: &str) -> Result<Vec<UsageWindow>> {
        let resp = self
            .client
            .get("https://api.anthropic.com/api/oauth/usage")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("anthropic-beta", "oauth-2025-04-20")
            .send()
            .await
            .context("Failed to fetch Claude OAuth usage")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Claude OAuth usage error: {status} - {body}");
        }

        let usage: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Claude OAuth usage response")?;

        let (windows, _) = parse_claude_usage_response(&usage);
        if windows.is_empty() {
            anyhow::bail!("Claude OAuth usage response did not contain quota windows");
        }

        Ok(windows)
    }

    async fn fetch_via_cookie(
        &self,
        session_key: &str,
    ) -> Result<(Vec<UsageWindow>, Option<CreditsInfo>)> {
        let orgs: serde_json::Value = self
            .client
            .get("https://claude.ai/api/organizations")
            .header("Cookie", format!("sessionKey={session_key}"))
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            )
            .send()
            .await
            .context("Failed to fetch Claude orgs via cookie")?
            .json()
            .await
            .context("Failed to parse Claude orgs")?;

        let org_id = self
            .select_web_organization_id(&orgs)
            .context("No Claude org found in cookie response")?;

        let usage: serde_json::Value = self
            .client
            .get(format!(
                "https://claude.ai/api/organizations/{org_id}/usage"
            ))
            .header("Cookie", format!("sessionKey={session_key}"))
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            )
            .send()
            .await
            .context("Failed to fetch Claude usage via cookie")?
            .json()
            .await
            .context("Failed to parse Claude usage via cookie")?;

        tracing::debug!("Claude cookie usage response: {usage:#?}");

        let (windows, mut credits) = parse_claude_usage_response(&usage);

        // Try overage/credits endpoint
        if credits.is_none() {
            credits = self.fetch_credits_cookie(session_key, &org_id).await.ok();
        }

        Ok((windows, credits))
    }

    fn select_web_organization_id(&self, orgs: &serde_json::Value) -> Option<String> {
        if let Ok(decoded) =
            serde_json::from_value::<Vec<ClaudeWebOrganizationResponse>>(orgs.clone())
        {
            let selected = decoded
                .iter()
                .find(|org| {
                    org.capabilities
                        .iter()
                        .any(|capability| capability.eq_ignore_ascii_case("chat"))
                })
                .or_else(|| {
                    decoded.iter().find(|org| {
                        let caps: Vec<String> = org
                            .capabilities
                            .iter()
                            .map(|capability| capability.to_ascii_lowercase())
                            .collect();
                        !(caps.len() == 1 && caps[0] == "api")
                    })
                })
                .or_else(|| decoded.first())?;

            tracing::debug!("Selected Claude org: {:?}", selected.name);
            return Some(selected.uuid.clone());
        }

        orgs.as_array()
            .and_then(|arr| arr.first())
            .and_then(|org| org.get("uuid"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
    }

    async fn fetch_credits_cookie(&self, session_key: &str, org_id: &str) -> Result<CreditsInfo> {
        let resp: serde_json::Value = self
            .client
            .get(format!(
                "https://claude.ai/api/organizations/{org_id}/overage_spend_limit"
            ))
            .header("Cookie", format!("sessionKey={session_key}"))
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            )
            .send()
            .await
            .context("Failed to fetch Claude credits via cookie")?
            .json()
            .await
            .context("Failed to parse Claude credits")?;

        let used_credits = resp.get("used_credits").and_then(|v| v.as_f64());
        let monthly_limit = resp.get("monthly_credit_limit").and_then(|v| v.as_f64());
        let currency = resp
            .get("currency")
            .and_then(|v| v.as_str())
            .unwrap_or("USD");

        if let (Some(used), Some(limit)) = (used_credits, monthly_limit) {
            Ok(CreditsInfo {
                balance: limit - used,
                currency: currency.to_string(),
                total_granted: Some(limit),
                topped_up: None,
            })
        } else {
            anyhow::bail!("No credits data in overage response")
        }
    }

    async fn fetch_via_api_key(
        &self,
        api_key: &str,
        base_url: Option<&str>,
    ) -> Result<Vec<UsageWindow>> {
        let base = base_url.unwrap_or("https://api.anthropic.com");
        if api_key.starts_with("sk-ant-admin-") {
            return self.fetch_via_admin_api(api_key, base).await;
        }

        let url = format!("{}/v1/usage", base.trim_end_matches('/'));

        let resp = self
            .client
            .get(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .context("Failed to fetch Claude usage via API key")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Claude API error: {status} - {body}");
        }

        let usage: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Claude usage response")?;

        tracing::debug!("Claude API key response: {usage:#?}");

        Ok(parse_usage_windows(&usage))
    }

    async fn fetch_via_admin_api(
        &self,
        api_key: &str,
        base_url: &str,
    ) -> Result<Vec<UsageWindow>> {
        let ending_at = Utc::now();
        let starting_at = ending_at - chrono::Duration::days(30);

        let url = format!("{}/v1/organizations/cost_report", base_url.trim_end_matches('/'));

        let resp = self
            .client
            .get(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .query(&[
                ("starting_at", starting_at.to_rfc3339()),
                ("ending_at", ending_at.to_rfc3339()),
            ])
            .send()
            .await
            .context("Failed to fetch Claude cost report via Admin API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Claude Admin API error: {status} - {body}");
        }

        #[derive(Debug, serde::Deserialize)]
        struct AdminCostReportResponse {
            data: Option<Vec<AdminCostItem>>,
        }

        #[derive(Debug, serde::Deserialize)]
        struct AdminCostItem {
            bucket_start_time: String,
            cost_usd: serde_json::Value,
        }

        let report: AdminCostReportResponse = resp
            .json()
            .await
            .context("Failed to parse Claude cost report response")?;

        let mut today_cost = 0.0;
        let mut seven_day_cost = 0.0;
        let mut thirty_day_cost = 0.0;

        let now = Utc::now();
        let start_of_today = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_local_timezone(Utc).unwrap();
        let start_of_seven_days = now - chrono::Duration::days(7);

        if let Some(items) = report.data {
            for item in items {
                let cost = parse_cost_usd(&item.cost_usd);
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&item.bucket_start_time) {
                    let dt_utc = dt.to_utc();
                    if dt_utc >= start_of_today {
                        today_cost += cost;
                    }
                    if dt_utc >= start_of_seven_days {
                        seven_day_cost += cost;
                    }
                    thirty_day_cost += cost;
                }
            }
        }

        let mut windows = Vec::new();
        windows.push(UsageWindow {
            label: "Today's Spend".to_string(),
            used_percent: 0.0,
            limit: None,
            used: Some(today_cost),
            unit: Some("USD".to_string()),
            resets_at: None,
        });
        windows.push(UsageWindow {
            label: "7d Spend".to_string(),
            used_percent: 0.0,
            limit: None,
            used: Some(seven_day_cost),
            unit: Some("USD".to_string()),
            resets_at: None,
        });
        windows.push(UsageWindow {
            label: "30d Spend".to_string(),
            used_percent: 0.0,
            limit: None,
            used: Some(thirty_day_cost),
            unit: Some("USD".to_string()),
            resets_at: None,
        });

        Ok(windows)
    }
}

fn parse_cost_usd(v: &serde_json::Value) -> f64 {
    if let Some(s) = v.as_str() {
        s.parse::<f64>().unwrap_or(0.0)
    } else if let Some(f) = v.as_f64() {
        f
    } else if let Some(i) = v.as_i64() {
        i as f64
    } else {
        0.0
    }
}
