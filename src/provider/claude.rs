use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{
    API_BUDGET_WINDOW_DAYS, CreditsInfo, Provider, UsageData, UsageWindow,
    completed_utc_day_window, http_client,
};

const OAUTH_PROFILE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

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
    #[allow(dead_code)]
    refresh_token: Option<String>,
    #[serde(default, alias = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(default, alias = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ClaudeOauthCredentials {
    access_token: String,
    subscription_tier: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct OrgResponse {
    uuid: String,
    name: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct UsageResponse {
    daily_usage: Option<WindowUsage>,
    session_limit: Option<WindowUsage>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct WindowUsage {
    used: Option<f64>,
    limit: Option<f64>,
    used_percent: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ClaudeWebOrganizationResponse {
    uuid: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    organization_type: Option<String>,
    #[serde(default)]
    rate_limit_tier: Option<String>,
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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct StatsCacheFile {
    #[serde(default)]
    model_usage: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct AdminCostReportResponse {
    #[serde(default)]
    data: Vec<AdminCostBucket>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_page: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AdminCostBucket {
    starting_at: String,
    #[serde(default)]
    results: Vec<AdminCostResult>,
}

#[derive(Debug, Deserialize)]
struct AdminCostResult {
    amount: serde_json::Value,
    currency: String,
}

#[derive(Debug, Default, PartialEq)]
struct AdminCostTotals {
    last_full_day: f64,
    seven_days: f64,
    thirty_days: f64,
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

fn normalize_subscription_tier(
    subscription_type: Option<&str>,
    rate_limit_tier: Option<&str>,
) -> Option<String> {
    let subscription_type = subscription_type?.trim().to_ascii_lowercase();
    let rate_limit_tier = rate_limit_tier.map(str::trim).map(str::to_ascii_lowercase);

    let tier = match subscription_type.as_str() {
        "claude_pro" | "pro" => "Pro",
        "claude_max" | "max" => match rate_limit_tier.as_deref() {
            Some("default_claude_max_5x") => "Max 5x",
            Some("default_claude_max_20x") => "Max 20x",
            _ => "Max",
        },
        "claude_team" | "team" => "Team",
        "claude_enterprise" | "enterprise" => "Enterprise",
        "claude_free" | "free" => "Free",
        "claude_api" | "api" => "API",
        _ => return None,
    };

    Some(tier.to_string())
}

fn parse_oauth_credentials(content: &str) -> Option<ClaudeOauthCredentials> {
    let json: serde_json::Value = serde_json::from_str(content).ok()?;
    let decoded = serde_json::from_value::<CredentialsFile>(json.clone()).ok();
    let oauth = decoded
        .as_ref()
        .and_then(|credentials| credentials.claude_ai_oauth.as_ref());

    let find_string = |paths: &[&str]| {
        paths.iter().find_map(|path| {
            json.pointer(path)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
    };

    let access_token = oauth
        .and_then(|oauth| oauth.access_token.as_deref())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .or_else(|| {
            find_string(&[
                "/claudeAiOauth/accessToken",
                "/claudeAiOauth/access_token",
                "/oauth/accessToken",
                "/oauth/access_token",
                "/accessToken",
                "/access_token",
            ])
        })?
        .to_string();

    let subscription_type = oauth
        .and_then(|oauth| oauth.subscription_type.as_deref())
        .or_else(|| {
            find_string(&[
                "/claudeAiOauth/subscriptionType",
                "/claudeAiOauth/subscription_type",
                "/oauth/subscriptionType",
                "/oauth/subscription_type",
                "/subscriptionType",
                "/subscription_type",
            ])
        });
    let rate_limit_tier = oauth
        .and_then(|oauth| oauth.rate_limit_tier.as_deref())
        .or_else(|| {
            find_string(&[
                "/claudeAiOauth/rateLimitTier",
                "/claudeAiOauth/rate_limit_tier",
                "/oauth/rateLimitTier",
                "/oauth/rate_limit_tier",
                "/rateLimitTier",
                "/rate_limit_tier",
            ])
        });

    Some(ClaudeOauthCredentials {
        access_token,
        subscription_tier: normalize_subscription_tier(subscription_type, rate_limit_tier),
    })
}

fn subscription_tier_from_oauth_profile(profile: &serde_json::Value) -> Option<String> {
    let organization = profile.get("organization")?;
    normalize_subscription_tier(
        organization
            .get("organization_type")
            .and_then(|value| value.as_str()),
        organization
            .get("rate_limit_tier")
            .and_then(|value| value.as_str()),
    )
}

fn resolve_oauth_subscription_tier(
    profile_result: Result<Option<String>>,
    cached_tier: Option<String>,
) -> Option<String> {
    match profile_result {
        Ok(profile_tier) => profile_tier.or(cached_tier),
        Err(error) => {
            tracing::debug!("Claude OAuth profile fetch failed: {error}");
            cached_tier
        }
    }
}

fn select_web_organization(orgs: &serde_json::Value) -> Option<ClaudeWebOrganizationResponse> {
    if let Ok(decoded) = serde_json::from_value::<Vec<ClaudeWebOrganizationResponse>>(orgs.clone())
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
                    !(org.capabilities.len() == 1
                        && org.capabilities[0].eq_ignore_ascii_case("api"))
                })
            })
            .or_else(|| decoded.first())?;
        return Some(selected.clone());
    }

    let org = orgs.as_array()?.first()?;
    Some(ClaudeWebOrganizationResponse {
        uuid: org.get("uuid")?.as_str()?.to_string(),
        capabilities: Vec::new(),
        organization_type: org
            .get("organization_type")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        rate_limit_tier: org
            .get("rate_limit_tier")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
    })
}

impl ClaudeProvider {
    pub fn new(
        credentials_path: Option<String>,
        session_key: Option<String>,
        api_key: Option<String>,
        access_token: Option<String>,
        proxy: Option<&str>,
    ) -> Self {
        Self {
            credentials_path,
            session_key: session_key.and_then(|key| normalize_session_key(&key)),
            api_key,
            access_token,
            client: http_client(proxy),
        }
    }

    fn default_credentials_path() -> Option<std::path::PathBuf> {
        let home = dirs::home_dir()?;
        Some(home.join(".claude").join(".credentials.json"))
    }

    fn read_oauth_credentials(&self) -> Option<ClaudeOauthCredentials> {
        let path = self
            .credentials_path
            .as_ref()
            .map(std::path::PathBuf::from)
            .or_else(Self::default_credentials_path)?;

        if !path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&path).ok()?;
        parse_oauth_credentials(&content)
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

    fn configured_admin_api_key(&self) -> Option<(String, Option<String>)> {
        if let Some(api_key) = self
            .api_key
            .as_ref()
            .filter(|key| is_admin_api_key(key))
            .cloned()
        {
            return Some((api_key, None));
        }

        let base_url = || {
            std::env::var("ANTHROPIC_BASE_URL")
                .ok()
                .filter(|url| !url.is_empty())
        };

        if let Ok(admin_key) = std::env::var("ANTHROPIC_ADMIN_KEY")
            && !admin_key.is_empty()
        {
            return Some((admin_key, base_url()));
        }

        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY")
            && is_admin_api_key(&api_key)
        {
            return Some((api_key, base_url()));
        }

        self.read_settings()
            .filter(|(api_key, _)| is_admin_api_key(api_key))
    }

    fn read_settings_env() -> Option<(String, Option<String>)> {
        if let Ok(admin_key) = std::env::var("ANTHROPIC_ADMIN_KEY")
            && !admin_key.is_empty()
        {
            let base_url = std::env::var("ANTHROPIC_BASE_URL")
                .ok()
                .filter(|u| !u.is_empty());
            return Some((admin_key, base_url));
        }
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())?;
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .ok()
            .filter(|u| !u.is_empty());
        Some((api_key, base_url))
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
        let mut subscription_tier = None;
        let mut source = "unknown";
        let admin_api_key = self.configured_admin_api_key();

        // Method 1: Try OAuth token (manual config / env var / credentials file)
        let configured_oauth_token = self.access_token.clone().or_else(|| {
            std::env::var("CLAUDE_ACCESS_TOKEN")
                .ok()
                .filter(|t| !t.is_empty())
        });
        let stored_oauth = configured_oauth_token
            .is_none()
            .then(|| self.read_oauth_credentials())
            .flatten();
        let (oauth_token, cached_subscription_tier) =
            if let Some(access_token) = configured_oauth_token {
                (Some(access_token), None)
            } else if let Some(credentials) = stored_oauth {
                (
                    Some(credentials.access_token),
                    credentials.subscription_tier,
                )
            } else {
                (None, None)
            };

        if let Some(access_token) = oauth_token {
            match self.fetch_via_oauth(&access_token).await {
                Ok((w, profile_tier)) => {
                    windows = w;
                    subscription_tier =
                        resolve_oauth_subscription_tier(profile_tier, cached_subscription_tier);
                    source = "oauth";
                }
                Err(e) => {
                    tracing::debug!("Claude OAuth fetch failed: {e}");
                }
            }
        }

        // Method 2: Try manual session cookie (config / env var)
        if windows.is_empty() {
            let session_key = self.session_key.clone().or_else(|| {
                std::env::var("CLAUDE_SESSION_KEY")
                    .ok()
                    .and_then(|key| normalize_session_key(&key))
            });

            if let Some(session_key) = session_key {
                tracing::debug!("Using Claude sessionKey");
                match self.fetch_via_cookie(&session_key).await {
                    Ok((w, c, tier)) => {
                        windows = w;
                        credits = c;
                        subscription_tier = tier;
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
            let api_key_and_url = self
                .api_key
                .clone()
                .map(|k| (k, None))
                .or_else(|| self.read_api_key());

            if let Some((api_key, base_url)) = api_key_and_url {
                let is_selected_admin_key = admin_api_key
                    .as_ref()
                    .is_some_and(|(admin_key, _)| admin_key == &api_key);

                if !is_selected_admin_key {
                    match self.fetch_via_api_key(&api_key, base_url.as_deref()).await {
                        Ok(w) => {
                            windows = w;
                            subscription_tier = Some("API".to_string());
                            source = "api_key";
                        }
                        Err(e) => {
                            tracing::debug!("Claude API key fetch failed: {e}");
                        }
                    }
                }
            }
        }

        // Admin API costs complement OAuth/session subscription quota. An explicitly
        // configured Admin key should not be ignored merely because quota fetches
        // succeeded through another authentication method.
        if let Some((api_key, base_url)) = admin_api_key {
            let base_url = base_url.as_deref().unwrap_or("https://api.anthropic.com");
            match self.fetch_via_admin_api(&api_key, base_url).await {
                Ok(cost_windows) => {
                    merge_admin_cost_windows(&mut windows, cost_windows);
                    if source == "unknown" {
                        subscription_tier = Some("API".to_string());
                    }
                    source = if source == "unknown" {
                        "admin_api"
                    } else {
                        "quota+admin_api"
                    };
                }
                Err(e) => {
                    tracing::warn!("Claude Admin cost fetch failed: {e}");
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
            subscription_tier,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

impl ClaudeProvider {
    async fn fetch_via_oauth(
        &self,
        access_token: &str,
    ) -> Result<(Vec<UsageWindow>, Result<Option<String>>)> {
        let usage_request = async {
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

            Ok::<_, anyhow::Error>(windows)
        };

        let profile_request = async {
            tokio::time::timeout(
                OAUTH_PROFILE_TIMEOUT,
                self.fetch_oauth_subscription_tier(access_token),
            )
            .await
            .unwrap_or_else(|_| anyhow::bail!("Claude OAuth profile request timed out"))
        };
        let (windows, profile_result) = tokio::join!(usage_request, profile_request);
        Ok((windows?, profile_result))
    }

    async fn fetch_oauth_subscription_tier(&self, access_token: &str) -> Result<Option<String>> {
        let resp = self
            .client
            .get("https://api.anthropic.com/api/oauth/profile")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .context("Failed to fetch Claude OAuth profile")?;

        if !resp.status().is_success() {
            anyhow::bail!("Claude OAuth profile error: {}", resp.status());
        }

        let profile: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Claude OAuth profile response")?;
        Ok(subscription_tier_from_oauth_profile(&profile))
    }

    async fn fetch_via_cookie(
        &self,
        session_key: &str,
    ) -> Result<(Vec<UsageWindow>, Option<CreditsInfo>, Option<String>)> {
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

        let org =
            select_web_organization(&orgs).context("No Claude org found in cookie response")?;
        let org_id = org.uuid;
        let subscription_tier = normalize_subscription_tier(
            org.organization_type.as_deref(),
            org.rate_limit_tier.as_deref(),
        );

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

        Ok((windows, credits, subscription_tier))
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
                balance: (limit - used) / 100.0,
                currency: currency.to_string(),
                total_granted: Some(limit / 100.0),
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
        if is_admin_api_key(api_key) {
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

    async fn fetch_via_admin_api(&self, api_key: &str, base_url: &str) -> Result<Vec<UsageWindow>> {
        // Daily buckets are UTC-aligned. Query the latest 30 complete buckets so
        // neither edge is silently dropped by a partial-day timestamp.
        let (starting_at, ending_at) = completed_utc_day_window(Utc::now());
        let mut totals = AdminCostTotals::default();
        let mut page = None;
        let mut seen_pages = std::collections::HashSet::new();

        loop {
            let url = admin_cost_report_url(base_url, &starting_at, &ending_at, page.as_deref())?;

            let resp = self
                .client
                .get(url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await
                .context("Failed to fetch Claude cost report via Admin API")?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Claude Admin API error: {status} - {body}");
            }

            let report: AdminCostReportResponse = resp
                .json()
                .await
                .context("Failed to parse Claude cost report response")?;

            totals += parse_admin_cost_report(&report, ending_at);

            if !report.has_more {
                break;
            }

            let next_page = report
                .next_page
                .filter(|next_page| !next_page.is_empty())
                .context("Claude Admin cost report has_more without next_page")?;
            if !seen_pages.insert(next_page.clone()) {
                anyhow::bail!("Claude Admin cost report repeated a pagination cursor");
            }
            page = Some(next_page);
        }

        Ok(vec![
            UsageWindow {
                label: "Last Full Day Spend".to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(totals.last_full_day),
                unit: Some("USD".to_string()),
                resets_at: None,
            },
            UsageWindow {
                label: "7d Spend".to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(totals.seven_days),
                unit: Some("USD".to_string()),
                resets_at: None,
            },
            UsageWindow {
                label: "30d Spend".to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(totals.thirty_days),
                unit: Some("USD".to_string()),
                resets_at: None,
            },
        ])
    }
}

impl std::ops::AddAssign for AdminCostTotals {
    fn add_assign(&mut self, rhs: Self) {
        self.last_full_day += rhs.last_full_day;
        self.seven_days += rhs.seven_days;
        self.thirty_days += rhs.thirty_days;
    }
}

fn is_admin_api_key(api_key: &str) -> bool {
    api_key.starts_with("sk-ant-admin01-") || api_key.starts_with("sk-ant-admin-")
}

fn admin_cost_report_url(
    base_url: &str,
    starting_at: &DateTime<Utc>,
    ending_at: &DateTime<Utc>,
    page: Option<&str>,
) -> Result<reqwest::Url> {
    let endpoint = format!(
        "{}/v1/organizations/cost_report",
        base_url.trim_end_matches('/')
    );
    let mut url = reqwest::Url::parse(&endpoint)
        .with_context(|| format!("Invalid Claude Admin API URL: {endpoint}"))?;

    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("starting_at", &starting_at.to_rfc3339())
            .append_pair("ending_at", &ending_at.to_rfc3339())
            .append_pair("bucket_width", "1d")
            .append_pair("limit", &(API_BUDGET_WINDOW_DAYS + 1).to_string());
        if let Some(page) = page {
            query.append_pair("page", page);
        }
    }

    Ok(url)
}

fn parse_admin_cost_report(
    report: &AdminCostReportResponse,
    ending_at: DateTime<Utc>,
) -> AdminCostTotals {
    let start_of_last_full_day = ending_at - chrono::Duration::days(1);
    let start_of_seven_days = ending_at - chrono::Duration::days(7);
    let start_of_thirty_days = ending_at - chrono::Duration::days(API_BUDGET_WINDOW_DAYS);
    let mut totals = AdminCostTotals::default();

    for bucket in &report.data {
        let Ok(starting_at) = DateTime::parse_from_rfc3339(&bucket.starting_at) else {
            continue;
        };
        let starting_at = starting_at.to_utc();
        if starting_at < start_of_thirty_days || starting_at >= ending_at {
            continue;
        }

        let cost = bucket
            .results
            .iter()
            .filter(|result| result.currency.eq_ignore_ascii_case("USD"))
            .filter_map(|result| parse_cost_usd(&result.amount))
            .sum::<f64>();

        totals.thirty_days += cost;
        if starting_at >= start_of_seven_days {
            totals.seven_days += cost;
        }
        if starting_at >= start_of_last_full_day {
            totals.last_full_day += cost;
        }
    }

    totals
}

fn merge_admin_cost_windows(windows: &mut Vec<UsageWindow>, cost_windows: Vec<UsageWindow>) {
    const COST_WINDOW_LABELS: [&str; 4] = [
        "Today's Spend",
        "Last Full Day Spend",
        "7d Spend",
        "30d Spend",
    ];
    windows.retain(|window| !COST_WINDOW_LABELS.contains(&window.label.as_str()));
    windows.extend(cost_windows);
}

fn normalize_session_key(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let cookie = trimmed
        .strip_prefix("Cookie:")
        .or_else(|| trimmed.strip_prefix("cookie:"))
        .unwrap_or(trimmed)
        .trim();

    for pair in cookie.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix("sessionKey=") {
            let value = value.trim();
            if value.starts_with("sk-ant-") {
                return Some(value.to_string());
            }
        }
    }

    if cookie.starts_with("sk-ant-") {
        Some(cookie.to_string())
    } else {
        None
    }
}

fn parse_cost_usd(value: &serde_json::Value) -> Option<f64> {
    let cents = if let Some(s) = value.as_str() {
        s.parse::<f64>().ok()
    } else {
        value.as_f64()
    }?;

    cents.is_finite().then_some(cents / 100.0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    #[test]
    fn recognizes_official_admin_api_key_prefix() {
        assert!(is_admin_api_key("sk-ant-admin01-example"));
        assert!(is_admin_api_key("sk-ant-admin-legacy"));
        assert!(!is_admin_api_key("sk-ant-api03-example"));
    }

    #[test]
    fn normalizes_supported_subscription_tiers_and_max_levels() {
        assert_eq!(
            normalize_subscription_tier(Some("claude_pro"), None).as_deref(),
            Some("Pro")
        );
        assert_eq!(
            normalize_subscription_tier(Some("max"), Some("default_claude_max_5x")).as_deref(),
            Some("Max 5x")
        );
        assert_eq!(
            normalize_subscription_tier(Some("claude_max"), Some("default_claude_max_20x"))
                .as_deref(),
            Some("Max 20x")
        );
        assert_eq!(
            normalize_subscription_tier(Some("claude_team"), None).as_deref(),
            Some("Team")
        );
        assert_eq!(
            normalize_subscription_tier(Some("enterprise"), None).as_deref(),
            Some("Enterprise")
        );
        assert_eq!(
            normalize_subscription_tier(Some("claude_free"), None).as_deref(),
            Some("Free")
        );
        assert_eq!(
            normalize_subscription_tier(Some("api"), None).as_deref(),
            Some("API")
        );
    }

    #[test]
    fn parses_cached_oauth_subscription_metadata() {
        let credentials = parse_oauth_credentials(
            r#"{
                "claudeAiOauth": {
                    "accessToken": "redacted-test-token",
                    "refreshToken": "redacted-refresh-token",
                    "subscriptionType": "max",
                    "rateLimitTier": "default_claude_max_20x"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(credentials.access_token, "redacted-test-token");
        assert_eq!(credentials.subscription_tier.as_deref(), Some("Max 20x"));
    }

    #[test]
    fn parses_legacy_oauth_metadata_paths() {
        let credentials = parse_oauth_credentials(
            r#"{
                "oauth": {
                    "access_token": "legacy-redacted-token",
                    "subscription_type": "claude_pro",
                    "rate_limit_tier": "default_claude_zero"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(credentials.access_token, "legacy-redacted-token");
        assert_eq!(credentials.subscription_tier.as_deref(), Some("Pro"));
    }

    #[test]
    fn profile_tier_overrides_cached_tier_and_failure_falls_back() {
        let profile = json!({
            "organization": {
                "organization_type": "claude_max",
                "rate_limit_tier": "default_claude_max_5x"
            }
        });
        let remote = subscription_tier_from_oauth_profile(&profile);

        assert_eq!(
            resolve_oauth_subscription_tier(Ok(remote), Some("Pro".to_string())).as_deref(),
            Some("Max 5x")
        );
        assert_eq!(
            resolve_oauth_subscription_tier(
                Err(anyhow::anyhow!("profile unavailable")),
                Some("Pro".to_string())
            )
            .as_deref(),
            Some("Pro")
        );
    }

    #[test]
    fn missing_or_unknown_profile_tier_is_not_inferred_as_free() {
        assert_eq!(
            subscription_tier_from_oauth_profile(&json!({
                "organization": {
                    "billing_type": "apple_subscription"
                }
            })),
            None
        );
        assert_eq!(
            subscription_tier_from_oauth_profile(&json!({
                "organization": {
                    "organization_type": "future_plan"
                }
            })),
            None
        );
        assert_eq!(normalize_subscription_tier(None, None), None);
    }

    #[test]
    fn selects_chat_organization_and_uses_its_tier() {
        let orgs = json!([
            {
                "uuid": "api-only",
                "capabilities": ["api"],
                "organization_type": "claude_enterprise"
            },
            {
                "uuid": "chat-org",
                "capabilities": ["chat", "api"],
                "organization_type": "claude_max",
                "rate_limit_tier": "default_claude_max_20x"
            }
        ]);

        let selected = select_web_organization(&orgs).unwrap();
        assert_eq!(selected.uuid, "chat-org");
        assert_eq!(
            normalize_subscription_tier(
                selected.organization_type.as_deref(),
                selected.rate_limit_tier.as_deref()
            )
            .as_deref(),
            Some("Max 20x")
        );
    }

    #[test]
    fn parses_bucketed_cost_results_from_cents() {
        let report: AdminCostReportResponse = serde_json::from_value(json!({
            "data": [
                {
                    "starting_at": "2026-07-28T00:00:00Z",
                    "ending_at": "2026-07-29T00:00:00Z",
                    "results": [
                        {"amount": "100.00", "currency": "USD"},
                        {"amount": "999.00", "currency": "EUR"}
                    ]
                },
                {
                    "starting_at": "2026-07-25T00:00:00Z",
                    "ending_at": "2026-07-26T00:00:00Z",
                    "results": [
                        {"amount": "250.50", "currency": "usd"}
                    ]
                },
                {
                    "starting_at": "2026-07-15T00:00:00Z",
                    "ending_at": "2026-07-16T00:00:00Z",
                    "results": [
                        {"amount": 50, "currency": "USD"}
                    ]
                },
                {
                    "starting_at": "2026-06-28T00:00:00Z",
                    "ending_at": "2026-06-29T00:00:00Z",
                    "results": [
                        {"amount": "900.00", "currency": "USD"}
                    ]
                }
            ],
            "has_more": true,
            "next_page": "page_next"
        }))
        .unwrap();
        let ending_at = Utc.with_ymd_and_hms(2026, 7, 29, 0, 0, 0).unwrap();

        let totals = parse_admin_cost_report(&report, ending_at);

        assert!((totals.last_full_day - 1.0).abs() < f64::EPSILON);
        assert!((totals.seven_days - 3.505).abs() < 1e-12);
        assert!((totals.thirty_days - 4.005).abs() < 1e-12);
        assert!(report.has_more);
        assert_eq!(report.next_page.as_deref(), Some("page_next"));
    }

    #[test]
    fn constructs_daily_paginated_cost_report_request() {
        let starting_at = Utc.with_ymd_and_hms(2026, 6, 29, 0, 0, 0).unwrap();
        let ending_at = Utc.with_ymd_and_hms(2026, 7, 29, 0, 0, 0).unwrap();

        let url = admin_cost_report_url(
            "https://api.anthropic.com/",
            &starting_at,
            &ending_at,
            Some("page/next+opaque"),
        )
        .unwrap();
        let query = url
            .query_pairs()
            .into_owned()
            .collect::<BTreeMap<String, String>>();

        assert_eq!(url.path(), "/v1/organizations/cost_report");
        assert_eq!(
            query.get("starting_at").map(String::as_str),
            Some("2026-06-29T00:00:00+00:00")
        );
        assert_eq!(
            query.get("ending_at").map(String::as_str),
            Some("2026-07-29T00:00:00+00:00")
        );
        assert_eq!(query.get("bucket_width").map(String::as_str), Some("1d"));
        assert_eq!(query.get("limit").map(String::as_str), Some("31"));
        assert_eq!(
            query.get("page").map(String::as_str),
            Some("page/next+opaque")
        );
    }

    #[test]
    fn admin_cost_merge_preserves_subscription_quota_windows() {
        let mut windows = vec![
            UsageWindow {
                label: "Session (5h)".to_string(),
                used_percent: 42.0,
                limit: None,
                used: None,
                unit: None,
                resets_at: None,
            },
            UsageWindow {
                label: "30d Spend".to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(1.0),
                unit: Some("USD".to_string()),
                resets_at: None,
            },
        ];
        let cost_windows = vec![UsageWindow {
            label: "30d Spend".to_string(),
            used_percent: 0.0,
            limit: None,
            used: Some(12.34),
            unit: Some("USD".to_string()),
            resets_at: None,
        }];

        merge_admin_cost_windows(&mut windows, cost_windows);

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].label, "Session (5h)");
        assert_eq!(windows[0].used_percent, 42.0);
        assert_eq!(windows[1].label, "30d Spend");
        assert_eq!(windows[1].used, Some(12.34));
    }
}
