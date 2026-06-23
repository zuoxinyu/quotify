use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;
use std::path::{Path, PathBuf};

use super::{Provider, UsageData, UsageWindow, http_client};

const ANTIGRAVITY_QUOTA_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota";
const ANTIGRAVITY_LOAD_CODE_ASSIST_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const GOOGLE_PROJECTS_URL: &str = "https://cloudresourcemanager.googleapis.com/v1/projects";
const ANTIGRAVITY_LS_LOG: &str = "Antigravity\\logs\\language_server.log";
const ANTIGRAVITY_MAIN_LOG: &str = "Antigravity\\logs\\main.log";
const ANTIGRAVITY_GET_AVAILABLE_MODELS_PATH: &str =
    "/exa.language_server_pb.LanguageServerService/GetAvailableModels";

pub struct AntigravityProvider {
    api_key: Option<String>,
    client: reqwest::Client,
}

#[derive(Clone)]
struct AntigravityCredentials {
    path: Option<PathBuf>,
    json: serde_json::Value,
    access_token: String,
    refresh_token: Option<String>,
    expiry_millis: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum QuotaWindowType {
    FiveHour,
    Daily,
    Weekly,
    Unknown,
}

#[derive(Clone)]
struct AntigravityQuotaEntry {
    model_id: String,
    remaining_percent: f64,
    reset_time: Option<chrono::DateTime<Utc>>,
    window_type: QuotaWindowType,
}

impl AntigravityProvider {
    pub fn new(api_key: Option<String>, proxy: Option<&str>) -> Self {
        Self {
            api_key,
            client: http_client(proxy),
        }
    }

    fn oauth_credentials_paths() -> Vec<PathBuf> {
        let Some(home) = dirs::home_dir() else {
            return Vec::new();
        };
        vec![
            home.join(".codexbar")
                .join("antigravity")
                .join("oauth_creds.json"),
            home.join(".antigravity").join("oauth_creds.json"),
        ]
    }

    fn read_oauth_credentials() -> Option<AntigravityCredentials> {
        if let Ok(content) = std::env::var("ANTIGRAVITY_OAUTH_CREDENTIALS_JSON")
            && let Some(credentials) = Self::parse_oauth_credentials(None, &content)
        {
            return Some(credentials);
        }

        if let Some(credentials) = read_windows_keyring_credentials() {
            return Some(credentials);
        }

        for path in Self::oauth_credentials_paths() {
            if !path.exists() {
                continue;
            }

            let content = std::fs::read_to_string(&path).ok()?;
            if let Some(credentials) = Self::parse_oauth_credentials(Some(path), &content) {
                return Some(credentials);
            }
        }

        None
    }

    fn parse_oauth_credentials(
        path: Option<PathBuf>,
        content: &str,
    ) -> Option<AntigravityCredentials> {
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;
        parse_antigravity_credentials_json(path, json)
    }

    fn credentials_from_json(
        path: Option<PathBuf>,
        json: serde_json::Value,
    ) -> Option<AntigravityCredentials> {
        parse_antigravity_credentials_json(path, json)
    }

    fn read_cli_auth_type() -> Option<String> {
        let path = dirs::home_dir()?.join(".antigravity").join("settings.json");
        if !path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;
        find_string_by_key(&json, &["selectedAuthType", "authType", "auth_type"])
            .map(|s| s.to_ascii_lowercase())
    }

    fn has_configured_api_key(&self) -> bool {
        self.api_key
            .as_ref()
            .is_some_and(|key| !key.trim().is_empty())
            || std::env::var("ANTIGRAVITY_API_KEY")
                .ok()
                .is_some_and(|key| !key.trim().is_empty())
            || std::env::var("GOOGLE_API_KEY")
                .ok()
                .is_some_and(|key| !key.trim().is_empty())
    }
}

fn parse_antigravity_credentials_json(
    path: Option<PathBuf>,
    json: serde_json::Value,
) -> Option<AntigravityCredentials> {
    let token = json.get("token").unwrap_or(&json);
    let access_token = string_field(token, &["access_token", "accessToken"])?;
    let refresh_token = string_field(token, &["refresh_token", "refreshToken"]);
    let expiry_millis = token.get("expiry").and_then(expiry_millis).or_else(|| {
        token
            .get("expiry_date")
            .or_else(|| token.get("expiryDate"))
            .and_then(expiry_millis)
    });

    Some(AntigravityCredentials {
        path,
        json,
        access_token,
        refresh_token,
        expiry_millis,
    })
}

#[cfg(target_os = "windows")]
fn read_windows_keyring_credentials() -> Option<AntigravityCredentials> {
    let json = read_windows_credential_json("gemini:antigravity")?;
    AntigravityProvider::credentials_from_json(None, json)
}

#[cfg(not(target_os = "windows"))]
fn read_windows_keyring_credentials() -> Option<AntigravityCredentials> {
    None
}

#[async_trait::async_trait]
impl Provider for AntigravityProvider {
    fn name(&self) -> &str {
        "antigravity"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        // Try local language server first to inspect what models it exposes
        match self.fetch_local_language_server_usage().await {
            Ok(windows) if !windows.is_empty() => {
                return Ok(UsageData {
                    provider: self.name().to_string(),
                    windows,
                    credits: None,
                    fetched_at: Utc::now(),
                    error: None,
                });
            }
            Ok(_) => {
                tracing::debug!("Antigravity local language server returned no model quota data");
            }
            Err(e) => {
                tracing::debug!("Antigravity local language server quota fetch failed: {e}");
            }
        }

        // Try remote fetch if local fails
        if let Some(credentials) = Self::read_oauth_credentials() {
            match self.fetch_remote_quota(credentials).await {
                Ok(windows) => {
                    if !windows.is_empty() {
                        return Ok(UsageData {
                            provider: self.name().to_string(),
                            windows,
                            credits: None,
                            fetched_at: Utc::now(),
                            error: None,
                        });
                    }
                }
                Err(e) => {
                    tracing::debug!("Antigravity remote quota fetch failed: {e}");
                }
            }
        }

        let auth_type = Self::read_cli_auth_type();
        if auth_type
            .as_deref()
            .is_some_and(|auth| auth.contains("api-key") || auth.contains("vertex"))
            && Self::read_oauth_credentials().is_none()
        {
            anyhow::bail!(
                "Antigravity usage requires Antigravity CLI OAuth login; API key and Vertex modes do not expose quota usage"
            );
        }

        let credentials = Self::read_oauth_credentials().with_context(|| {
            if self.has_configured_api_key() {
                "Antigravity API key is configured, but usage quota requires Antigravity OAuth credentials from ANTIGRAVITY_OAUTH_CREDENTIALS_JSON or ~/.codexbar/antigravity/oauth_creds.json"
            } else {
                "Antigravity OAuth credentials not found. Configure ANTIGRAVITY_OAUTH_CREDENTIALS_JSON or ~/.codexbar/antigravity/oauth_creds.json"
            }
        })?;

        let windows = self.fetch_remote_quota(credentials).await?;
        if windows.is_empty() {
            anyhow::bail!("Antigravity quota response did not contain model usage data");
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

enum QuotaOrSummary {
    Quota(serde_json::Value),
    Summary(serde_json::Value),
}

impl AntigravityProvider {
    async fn fetch_remote_quota(
        &self,
        credentials: AntigravityCredentials,
    ) -> Result<Vec<UsageWindow>> {
        let access_token = self.resolve_access_token(credentials.clone()).await?;
        let quota_summary_or_quota = match self.fetch_remote_quota_with_token(&access_token).await {
            Ok(val) => val,
            Err(e) if is_auth_error(&e) => {
                tracing::debug!("Antigravity quota token rejected, refreshing and retrying: {e}");
                let refresh_token = credentials.refresh_token.clone().context(
                    "Antigravity OAuth token was rejected and no refresh token is available",
                )?;
                let refreshed = self
                    .refresh_access_token(credentials, &refresh_token)
                    .await
                    .context("Antigravity OAuth token was rejected and refresh failed")?;
                self.fetch_remote_quota_with_token(&refreshed).await?
            }
            Err(e) => return Err(e),
        };

        match quota_summary_or_quota {
            QuotaOrSummary::Summary(summary) => {
                let windows = parse_antigravity_quota_summary(&summary);
                if !windows.is_empty() {
                    return Ok(windows);
                }
                anyhow::bail!("Antigravity remote summary parsed to empty windows");
            }
            QuotaOrSummary::Quota(quota) => {
                let windows = parse_antigravity_quota(&quota);
                Ok(windows)
            }
        }
    }

    async fn fetch_remote_quota_with_token(&self, access_token: &str) -> Result<QuotaOrSummary> {
        let fallback_proj = self
            .discover_project_fallback(access_token)
            .await
            .ok()
            .flatten();

        let project = match self.load_code_assist_project(access_token).await {
            Ok(project) => project,
            Err(e) => {
                tracing::debug!("Antigravity loadCodeAssist project discovery failed: {e}");
                fallback_proj
            }
        };

        let body = if let Some(project) = project.as_deref().filter(|p| !p.is_empty()) {
            json!({ "project": project })
        } else {
            json!({})
        };

        // Try retrieveUserQuotaSummary first
        let summary_urls = [
            "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
            "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
        ];

        for url in summary_urls {
            if let Ok(resp) = self
                .client
                .post(url)
                .bearer_auth(access_token)
                .json(&body)
                .send()
                .await
            {
                if resp.status().is_success() {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        if json.get("response").and_then(|r| r.get("groups")).is_some()
                            || json.get("groups").is_some()
                        {
                            return Ok(QuotaOrSummary::Summary(json));
                        }
                    }
                }
            }
        }

        // Fallback to retrieveUserQuota
        let resp = self
            .client
            .post(ANTIGRAVITY_QUOTA_URL)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await
            .context("Failed to call Antigravity quota API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Antigravity quota API error: {status} - {body}");
        }

        let json = resp
            .json()
            .await
            .context("Failed to parse Antigravity quota response")?;

        Ok(QuotaOrSummary::Quota(json))
    }

    async fn fetch_local_language_server_usage(&self) -> Result<Vec<UsageWindow>> {
        let base_url = local_language_server_http_url()
            .context("Antigravity local language server HTTP port not found")?;
        let local_client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .context("Failed to build Antigravity local language server HTTP client")?;
        let html = local_client
            .get(&base_url)
            .send()
            .await
            .context("Failed to read Antigravity local language server page")?
            .text()
            .await
            .context("Failed to read Antigravity local language server HTML")?;
        let csrf = extract_antigravity_csrf_token(&html)
            .or_else(local_language_server_csrf_token)
            .context("Antigravity local language server CSRF token not found")?;

        // 1. Try to get summary first (which includes weekly and five hour limits)
        let summary_url = format!(
            "{base_url}/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary"
        );
        if let Ok(resp) = local_client
            .post(&summary_url)
            .header("x-codeium-csrf-token", &csrf)
            .json(&json!({}))
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    let parsed = parse_antigravity_quota_summary(&json);
                    if !parsed.is_empty() {
                        return Ok(parsed);
                    }
                }
            }
        }

        // 2. Fall back to GetAvailableModels if summary fails
        let url = format!("{base_url}{ANTIGRAVITY_GET_AVAILABLE_MODELS_PATH}");
        let resp = local_client
            .post(url)
            .header("x-codeium-csrf-token", &csrf)
            .json(&json!({}))
            .send()
            .await
            .context("Failed to call Antigravity local GetAvailableModels")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Antigravity local GetAvailableModels error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Antigravity local GetAvailableModels response")?;
        Ok(parse_antigravity_available_models(&json))
    }

    async fn resolve_access_token(&self, credentials: AntigravityCredentials) -> Result<String> {
        let current_access_token = credentials.access_token.clone();
        let expired = credentials
            .expiry_millis
            .is_some_and(|expiry| expiry <= Utc::now().timestamp_millis() + 60_000);

        if !expired {
            return Ok(current_access_token);
        }

        let Some(refresh_token) = credentials.refresh_token.clone() else {
            return Ok(current_access_token);
        };

        match self.refresh_access_token(credentials, &refresh_token).await {
            Ok(token) => Ok(token),
            Err(e) => {
                tracing::debug!("Antigravity OAuth token refresh failed: {e}");
                Ok(current_access_token)
            }
        }
    }

    async fn refresh_access_token(
        &self,
        mut credentials: AntigravityCredentials,
        refresh_token: &str,
    ) -> Result<String> {
        let (client_id, client_secret) =
            find_antigravity_oauth_client().context("Antigravity CLI OAuth client not found")?;

        let resp = self
            .client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .context("Failed to refresh Antigravity OAuth token")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Antigravity OAuth refresh failed: {status} - {body}");
        }

        let refreshed: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Antigravity OAuth refresh response")?;
        let access_token = string_field(&refreshed, &["access_token", "accessToken"])
            .context("Antigravity OAuth refresh response missing access token")?;
        let expires_in = refreshed
            .get("expires_in")
            .and_then(|v| v.as_i64())
            .unwrap_or(3600);

        credentials.json["access_token"] = json!(access_token);
        credentials.json["expiry_date"] =
            json!(Utc::now().timestamp_millis() + expires_in.saturating_mul(1000));
        if let Some(id_token) = string_field(&refreshed, &["id_token", "idToken"]) {
            credentials.json["id_token"] = json!(id_token);
        }

        if let Some(path) = &credentials.path {
            std::fs::write(path, serde_json::to_string_pretty(&credentials.json)?).with_context(
                || format!("Failed to write refreshed Antigravity credentials to {path:?}"),
            )?;
        }

        Ok(access_token)
    }

    async fn load_code_assist_project(&self, access_token: &str) -> Result<Option<String>> {
        let resp = self
            .client
            .post(ANTIGRAVITY_LOAD_CODE_ASSIST_URL)
            .bearer_auth(access_token)
            .json(&json!({
                "metadata": {
                    "ideType": "ANTIGRAVITY",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "GEMINI"
                }
            }))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to call Antigravity loadCodeAssist: {e:?}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Antigravity loadCodeAssist error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Antigravity loadCodeAssist response")?;
        tracing::debug!("Antigravity loadCodeAssist raw response: {json:?}");
        Ok(find_string_by_key(
            &json,
            &["cloudaicompanionProject", "project"],
        ))
    }

    async fn discover_project_fallback(&self, access_token: &str) -> Result<Option<String>> {
        let resp = self
            .client
            .get(GOOGLE_PROJECTS_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .context("Failed to list Google Cloud projects")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Google Cloud project list error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Google Cloud projects response")?;
        println!(
            "LISTED PROJECTS: {}",
            serde_json::to_string_pretty(&json).unwrap()
        );
        Ok(select_antigravity_project(&json))
    }
}

fn string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    })
}

fn expiry_millis(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(number) => number.as_i64().map(|raw| {
            if raw < 10_000_000_000 {
                raw.saturating_mul(1000)
            } else {
                raw
            }
        }),
        serde_json::Value::String(s) => s
            .parse::<i64>()
            .ok()
            .map(|raw| {
                if raw < 10_000_000_000 {
                    raw.saturating_mul(1000)
                } else {
                    raw
                }
            })
            .or_else(|| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|dt| dt.timestamp_millis())
            }),
        _ => None,
    }
}

fn find_string_by_key(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(found) = map
                    .get(*key)
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    return Some(found.to_string());
                }
            }

            map.values()
                .find_map(|child| find_string_by_key(child, keys))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_by_key(child, keys)),
        _ => None,
    }
}

fn local_language_server_http_url() -> Option<String> {
    let appdata = std::env::var("APPDATA").ok()?;
    let log = std::fs::read_to_string(PathBuf::from(appdata).join(ANTIGRAVITY_LS_LOG)).ok()?;
    let regex = regex::Regex::new(r"listening on random port at (\d+) for HTTP").ok()?;
    let port = regex
        .captures_iter(&log)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .last()?;
    Some(format!("http://127.0.0.1:{port}"))
}

fn local_language_server_csrf_token() -> Option<String> {
    let appdata = std::env::var("APPDATA").ok()?;
    let log = std::fs::read_to_string(PathBuf::from(appdata).join(ANTIGRAVITY_MAIN_LOG)).ok()?;
    let regex = regex::Regex::new(r"--csrf_token\s+([0-9a-fA-F-]+)").ok()?;
    regex
        .captures_iter(&log)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .last()
}

fn extract_antigravity_csrf_token(html: &str) -> Option<String> {
    let regex = regex::Regex::new(r#""csrfToken"\s*:\s*"([^"]+)""#).ok()?;
    regex
        .captures(html)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

fn parse_antigravity_available_models(value: &serde_json::Value) -> Vec<UsageWindow> {
    let response = value.get("response").unwrap_or(value);
    let Some(models) = response.get("models").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut model_ids = available_model_sort_order(response);
    if model_ids.is_empty() {
        model_ids.extend(models.keys().cloned());
        model_ids.sort();
    }

    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for model_id in model_ids {
        if !seen.insert(model_id.clone()) {
            continue;
        }

        let Some(model) = models.get(&model_id) else {
            continue;
        };
        let Some(quota) = model.get("quotaInfo").or_else(|| model.get("quota_info")) else {
            continue;
        };
        let Some(remaining) = quota
            .get("remainingFraction")
            .or_else(|| quota.get("remaining_fraction"))
            .and_then(number_value)
        else {
            continue;
        };

        let label = string_field(model, &["displayName", "display_name", "label"])
            .unwrap_or_else(|| model_id.clone());
        let remaining_percent = if remaining <= 1.0 {
            remaining * 100.0
        } else {
            remaining
        }
        .clamp(0.0, 100.0);
        let resets_at = if remaining_percent >= 100.0 {
            None
        } else {
            string_field(quota, &["resetTime", "reset_time", "resetsAt"])
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.to_utc())
        };

        let mut win = detect_window_type(&model_id);
        if win == QuotaWindowType::Unknown {
            win = detect_window_type(&label);
        }

        entries.push(AntigravityQuotaEntry {
            model_id: model_id.clone(),
            remaining_percent,
            reset_time: resets_at,
            window_type: win,
        });
    }

    group_antigravity_entries(&entries)
}

fn map_antigravity_label(group_name: &str, bucket_name: &str) -> String {
    let lower_group = group_name.to_ascii_lowercase();
    let lower_bucket = bucket_name.to_ascii_lowercase();

    let prefix = if lower_group.contains("gemini") {
        "Gemini"
    } else if lower_group.contains("claude") || lower_group.contains("gpt") {
        "Claude"
    } else {
        group_name
    };

    let suffix = if lower_bucket.contains("five_hour")
        || lower_bucket.contains("fivehour")
        || lower_bucket.contains("five hour")
        || lower_bucket.contains("5h")
        || lower_bucket.contains("5hour")
        || lower_bucket.contains("session")
    {
        "5h"
    } else if lower_bucket.contains("seven_day")
        || lower_bucket.contains("seven day")
        || lower_bucket.contains("sevenday")
        || lower_bucket.contains("7d")
        || lower_bucket.contains("7day")
        || lower_bucket.contains("weekly")
        || lower_bucket.contains("week")
    {
        "Weekly"
    } else if lower_bucket.contains("day")
        || lower_bucket.contains("daily")
        || lower_bucket.contains("24h")
        || lower_bucket.contains("24hour")
    {
        "Daily"
    } else {
        bucket_name
    };

    if prefix == group_name && suffix == bucket_name {
        format!("{} - {}", group_name, bucket_name)
    } else {
        format!("{} {}", prefix, suffix)
    }
}

fn parse_antigravity_quota_summary(value: &serde_json::Value) -> Vec<UsageWindow> {
    let mut windows = Vec::new();
    let response = value.get("response").unwrap_or(value);
    let Some(groups) = response.get("groups").and_then(|v| v.as_array()) else {
        return windows;
    };

    for group in groups {
        let group_name = string_field(group, &["displayName", "display_name"]).unwrap_or_default();
        if group_name.is_empty() {
            continue;
        }

        let Some(buckets) = group.get("buckets").and_then(|v| v.as_array()) else {
            continue;
        };

        for bucket in buckets {
            let bucket_name =
                string_field(bucket, &["displayName", "display_name"]).unwrap_or_default();
            if bucket_name.is_empty() {
                continue;
            }

            let Some(remaining) = bucket
                .get("remainingFraction")
                .or_else(|| bucket.get("remaining_fraction"))
                .and_then(number_value)
            else {
                continue;
            };

            let remaining_percent = if remaining <= 1.0 {
                remaining * 100.0
            } else {
                remaining
            }
            .clamp(0.0, 100.0);

            let used_percent = (100.0 - remaining_percent).clamp(0.0, 100.0);

            let resets_at = string_field(bucket, &["resetTime", "reset_time", "resetsAt"])
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.to_utc());

            let label = map_antigravity_label(&group_name, &bucket_name);

            windows.push(UsageWindow {
                label,
                used_percent,
                limit: Some(100.0),
                used: Some(used_percent),
                unit: Some("%".to_string()),
                resets_at,
            });
        }
    }

    let priority = |label: &str| -> usize {
        if label.contains("Gemini 5h") {
            0
        } else if label.contains("Gemini Daily") {
            1
        } else if label.contains("Gemini Weekly") {
            2
        } else if label.contains("Claude 5h") {
            3
        } else if label.contains("Claude Daily") {
            4
        } else if label.contains("Claude Weekly") {
            5
        } else {
            6
        }
    };
    windows.sort_by_key(|w| priority(&w.label));

    windows
}

fn available_model_sort_order(response: &serde_json::Value) -> Vec<String> {
    let mut ids = Vec::new();
    let Some(sorts) = response
        .get("agentModelSorts")
        .or_else(|| response.get("agent_model_sorts"))
        .and_then(|v| v.as_array())
    else {
        return ids;
    };

    for sort in sorts {
        if let Some(groups) = sort.get("groups").and_then(|v| v.as_array()) {
            for group in groups {
                if let Some(model_ids) = group
                    .get("modelIds")
                    .or_else(|| group.get("model_ids"))
                    .and_then(|v| v.as_array())
                {
                    ids.extend(
                        model_ids
                            .iter()
                            .filter_map(|id| id.as_str().map(ToString::to_string)),
                    );
                }
            }
        }
    }

    ids
}

#[cfg(target_os = "windows")]
fn read_windows_credential_json(target: &str) -> Option<serde_json::Value> {
    #[repr(C)]
    struct FileTime {
        low_date_time: u32,
        high_date_time: u32,
    }

    #[repr(C)]
    struct CredentialW {
        flags: u32,
        type_: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: FileTime,
        credential_blob_size: u32,
        credential_blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut std::ffi::c_void,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn CredReadW(
            target_name: *const u16,
            type_: u32,
            flags: u32,
            credential: *mut *mut CredentialW,
        ) -> i32;
        fn CredFree(buffer: *mut std::ffi::c_void);
    }

    const CRED_TYPE_GENERIC: u32 = 1;

    let mut target_wide: Vec<u16> = target.encode_utf16().collect();
    target_wide.push(0);

    let mut credential: *mut CredentialW = std::ptr::null_mut();
    let ok = unsafe { CredReadW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };

    if ok == 0 || credential.is_null() {
        return None;
    }

    let result = unsafe {
        let credential_ref = &*credential;
        let blob = std::slice::from_raw_parts(
            credential_ref.credential_blob,
            credential_ref.credential_blob_size as usize,
        );
        std::str::from_utf8(blob)
            .ok()
            .map(|text| text.trim_end_matches('\0'))
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
    };

    unsafe {
        CredFree(credential.cast());
    }

    result
}

fn select_antigravity_project(value: &serde_json::Value) -> Option<String> {
    let projects = value.get("projects").and_then(|v| v.as_array())?;

    projects
        .iter()
        .find_map(|project| {
            let project_id = string_field(project, &["projectId", "project_id", "id"])?;
            if project_id.starts_with("gen-lang-client") || has_generative_language_label(project) {
                Some(project_id)
            } else {
                None
            }
        })
        .or_else(|| {
            projects
                .iter()
                .filter_map(|project| string_field(project, &["projectId", "project_id", "id"]))
                .next()
        })
}

fn has_generative_language_label(project: &serde_json::Value) -> bool {
    let Some(labels) = project.get("labels").and_then(|v| v.as_object()) else {
        return false;
    };

    labels.iter().any(|(key, value)| {
        key.contains("generative-language")
            || value
                .as_str()
                .is_some_and(|label| label.contains("generative-language"))
    })
}

fn is_auth_error(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    text.contains("401")
        || text.contains("403")
        || text.contains("UNAUTHENTICATED")
        || text.contains("invalid authentication credentials")
}

fn find_antigravity_oauth_client() -> Option<(String, String)> {
    if let (Ok(id), Ok(secret)) = (
        std::env::var("ANTIGRAVITY_OAUTH_CLIENT_ID"),
        std::env::var("ANTIGRAVITY_OAUTH_CLIENT_SECRET"),
    ) && !id.is_empty()
        && !secret.is_empty()
    {
        return Some((id, secret));
    }

    let mut roots = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        roots.push(PathBuf::from(appdata).join("npm").join("node_modules"));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(
            home.join(".bun")
                .join("install")
                .join("global")
                .join("node_modules"),
        );
    }

    for root in &roots {
        if let Some(path) = find_file_named(root, "oauth2.js", 8)
            && let Some(client) = parse_oauth_client_from_file(&path)
        {
            return Some(client);
        }
    }

    for root in roots {
        if let Some(client) = find_oauth_client_in_js(&root, 8) {
            return Some(client);
        }
    }

    None
}

fn find_oauth_client_in_js(root: &Path, depth: usize) -> Option<(String, String)> {
    if depth == 0 || !root.exists() {
        return None;
    }

    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path.extension().is_some_and(|ext| ext == "js")
            && path.to_string_lossy().contains("antigravity")
            && let Some(client) = parse_oauth_client_from_file(&path)
        {
            return Some(client);
        }
        if path.is_dir()
            && let Some(client) = find_oauth_client_in_js(&path, depth - 1)
        {
            return Some(client);
        }
    }

    None
}

fn find_file_named(root: &Path, file_name: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 || !root.exists() {
        return None;
    }

    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(file_name))
            && path.to_string_lossy().contains("antigravity")
        {
            return Some(path);
        }
        if path.is_dir()
            && let Some(found) = find_file_named(&path, file_name, depth - 1)
        {
            return Some(found);
        }
    }

    None
}

fn parse_oauth_client_from_file(path: &Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    let id = regex::Regex::new(r#"(?m)OAUTH_CLIENT_ID\s*[:=]\s*["']([^"']+)["']"#)
        .ok()?
        .captures(&content)?
        .get(1)?
        .as_str()
        .to_string();
    let secret = regex::Regex::new(r#"(?m)OAUTH_CLIENT_SECRET\s*[:=]\s*["']([^"']+)["']"#)
        .ok()?
        .captures(&content)?
        .get(1)?
        .as_str()
        .to_string();
    Some((id, secret))
}

fn parse_antigravity_quota(value: &serde_json::Value) -> Vec<UsageWindow> {
    let mut entries = Vec::new();
    collect_quota_entries(value, None, QuotaWindowType::Unknown, None, &mut entries);

    if entries.is_empty() {
        return Vec::new();
    }

    group_antigravity_entries(&entries)
}

fn collect_quota_entries(
    value: &serde_json::Value,
    model_hint: Option<&str>,
    mut current_window: QuotaWindowType,
    mut current_model_id: Option<String>,
    out: &mut Vec<AntigravityQuotaEntry>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(hint) = model_hint {
                let w = detect_window_type(hint);
                if w != QuotaWindowType::Unknown {
                    current_window = w;
                }

                let lower_hint = hint.to_ascii_lowercase();
                if lower_hint.contains("gemini")
                    || lower_hint.contains("claude")
                    || lower_hint.contains("gpt")
                {
                    current_model_id = Some(hint.to_string());
                }
            }

            let explicit_model_id = string_field(
                value,
                &["modelId", "model_id", "model", "modelName", "name"],
            );
            if let Some(mid) = explicit_model_id {
                current_model_id = Some(mid);
            }

            let remaining = map
                .get("remainingFraction")
                .or_else(|| map.get("remaining_fraction"))
                .or_else(|| map.get("remainingPercent"))
                .or_else(|| map.get("remaining_percent"))
                .and_then(number_value);

            if let Some(remaining) = remaining {
                if let Some(ref model_id) = current_model_id {
                    let remaining_percent = if remaining <= 1.0 {
                        remaining * 100.0
                    } else {
                        remaining
                    }
                    .clamp(0.0, 100.0);

                    let reset_time = string_field(value, &["resetTime", "reset_time", "resetsAt"])
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.to_utc());

                    let mut final_window = current_window;
                    let w = detect_window_type(model_id);
                    if w != QuotaWindowType::Unknown {
                        final_window = w;
                    }

                    out.push(AntigravityQuotaEntry {
                        model_id: model_id.clone(),
                        remaining_percent,
                        reset_time,
                        window_type: final_window,
                    });
                }
            }

            for (key, child) in map {
                let mut child_window = current_window;
                let w = detect_window_type(key);
                if w != QuotaWindowType::Unknown {
                    child_window = w;
                }
                collect_quota_entries(
                    child,
                    Some(key),
                    child_window,
                    current_model_id.clone(),
                    out,
                );
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_quota_entries(
                    child,
                    model_hint,
                    current_window,
                    current_model_id.clone(),
                    out,
                );
            }
        }
        _ => {}
    }
}

fn number_value(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse::<f64>().ok()))
}

fn detect_window_type(name: &str) -> QuotaWindowType {
    let lower = name.to_ascii_lowercase();
    if lower.contains("five_hour")
        || lower.contains("fivehour")
        || lower.contains("five hour")
        || lower.contains("5h")
        || lower.contains("5hour")
        || lower.contains("session")
    {
        QuotaWindowType::FiveHour
    } else if lower.contains("seven_day")
        || lower.contains("seven day")
        || lower.contains("sevenday")
        || lower.contains("7d")
        || lower.contains("7day")
        || lower.contains("weekly")
        || lower.contains("week")
    {
        QuotaWindowType::Weekly
    } else if lower.contains("day")
        || lower.contains("daily")
        || lower.contains("24h")
        || lower.contains("24hour")
    {
        QuotaWindowType::Daily
    } else {
        QuotaWindowType::Unknown
    }
}

fn group_antigravity_entries(entries: &[AntigravityQuotaEntry]) -> Vec<UsageWindow> {
    let mut gemini_5h = Vec::new();
    let mut gemini_daily = Vec::new();
    let mut gemini_weekly = Vec::new();

    let mut claude_gpt_5h = Vec::new();
    let mut claude_gpt_daily = Vec::new();
    let mut claude_gpt_weekly = Vec::new();

    let mut unmatched = Vec::new();

    for entry in entries {
        let is_gemini = entry.model_id.to_ascii_lowercase().contains("gemini");
        let is_claude_gpt = entry.model_id.to_ascii_lowercase().contains("claude")
            || entry.model_id.to_ascii_lowercase().contains("gpt");

        let mut win = entry.window_type;
        if win == QuotaWindowType::Unknown {
            if let Some(reset) = entry.reset_time {
                let duration = reset.signed_duration_since(chrono::Utc::now());
                let hours = duration.num_hours();
                if hours > 36 {
                    win = QuotaWindowType::Weekly;
                } else if hours > 6 {
                    win = QuotaWindowType::Daily;
                } else {
                    win = QuotaWindowType::FiveHour;
                }
            } else {
                win = QuotaWindowType::FiveHour;
            }
        }

        if is_gemini {
            match win {
                QuotaWindowType::FiveHour => gemini_5h.push(entry),
                QuotaWindowType::Daily => gemini_daily.push(entry),
                QuotaWindowType::Weekly => gemini_weekly.push(entry),
                QuotaWindowType::Unknown => gemini_5h.push(entry),
            }
        } else if is_claude_gpt {
            match win {
                QuotaWindowType::FiveHour => claude_gpt_5h.push(entry),
                QuotaWindowType::Daily => claude_gpt_daily.push(entry),
                QuotaWindowType::Weekly => claude_gpt_weekly.push(entry),
                QuotaWindowType::Unknown => claude_gpt_5h.push(entry),
            }
        } else {
            unmatched.push(entry);
        }
    }

    let mut windows = Vec::new();

    if let Some(w) = best_antigravity_window("Gemini 5h", gemini_5h.into_iter()) {
        windows.push(w);
    }
    if let Some(w) = best_antigravity_window("Gemini Daily", gemini_daily.into_iter()) {
        windows.push(w);
    }
    if let Some(w) = best_antigravity_window("Gemini Weekly", gemini_weekly.into_iter()) {
        windows.push(w);
    }
    if let Some(w) = best_antigravity_window("Claude 5h", claude_gpt_5h.into_iter()) {
        windows.push(w);
    }
    if let Some(w) = best_antigravity_window("Claude Daily", claude_gpt_daily.into_iter()) {
        windows.push(w);
    }
    if let Some(w) = best_antigravity_window("Claude Weekly", claude_gpt_weekly.into_iter()) {
        windows.push(w);
    }

    // Add any unmatched entries as individual windows
    for entry in unmatched {
        let label = entry.model_id.clone();
        let used_percent = (100.0 - entry.remaining_percent).clamp(0.0, 100.0);
        windows.push(UsageWindow {
            label,
            used_percent,
            limit: Some(100.0),
            used: Some(used_percent),
            unit: Some("%".to_string()),
            resets_at: entry.reset_time,
        });
    }

    windows
}

fn best_antigravity_window<'a>(
    label: &str,
    entries: impl Iterator<Item = &'a AntigravityQuotaEntry>,
) -> Option<UsageWindow> {
    let entry = entries.min_by(|a, b| {
        a.remaining_percent
            .partial_cmp(&b.remaining_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let used_percent = (100.0 - entry.remaining_percent).clamp(0.0, 100.0);

    Some(UsageWindow {
        label: label.to_string(),
        used_percent,
        limit: Some(100.0),
        used: Some(used_percent),
        unit: Some("%".to_string()),
        resets_at: entry.reset_time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_antigravity_quota_grouping() {
        let quota_json = json!({
            "five_hour": {
                "gemini-1.5-pro": {
                    "remainingFraction": 0.8
                },
                "claude-3-5-sonnet": {
                    "remainingFraction": 0.6
                }
            },
            "seven_day": {
                "gemini-1.5-pro": {
                    "remainingFraction": 0.4
                },
                "claude-3-5-sonnet": {
                    "remainingFraction": 0.2
                }
            }
        });

        let windows = parse_antigravity_quota(&quota_json);
        assert_eq!(windows.len(), 4);

        let get_window = |label: &str| windows.iter().find(|w| w.label == label).unwrap();

        let g5h = get_window("Gemini 5h");
        assert_eq!(g5h.used_percent, 20.0);

        let gweekly = get_window("Gemini Weekly");
        assert_eq!(gweekly.used_percent, 60.0);

        let c5h = get_window("Claude 5h");
        assert_eq!(c5h.used_percent, 40.0);

        let cweekly = get_window("Claude Weekly");
        assert_eq!(cweekly.used_percent, 80.0);
    }

    #[test]
    fn test_parse_antigravity_quota_summary() {
        let summary_json = json!({
            "response": {
                "groups": [
                    {
                        "displayName": "Gemini Models",
                        "buckets": [
                            {
                                "bucketId": "gemini-weekly",
                                "displayName": "Weekly Limit",
                                "remainingFraction": 0.8287296,
                                "resetTime": "2026-06-28T06:52:37Z"
                            },
                            {
                                "bucketId": "gemini-5h",
                                "displayName": "Five Hour Limit",
                                "remainingFraction": 0.6096169,
                                "resetTime": "2026-06-22T14:38:32Z"
                            }
                        ]
                    },
                    {
                        "displayName": "Claude and GPT models",
                        "buckets": [
                            {
                                "bucketId": "3p-weekly",
                                "displayName": "Weekly Limit",
                                "remainingFraction": 1.0,
                                "resetTime": "2026-06-29T12:12:45Z"
                            },
                            {
                                "bucketId": "3p-5h",
                                "displayName": "Five Hour Limit",
                                "remainingFraction": 1.0,
                                "resetTime": "2026-06-22T17:12:45Z"
                            }
                        ]
                    }
                ]
            }
        });

        let windows = parse_antigravity_quota_summary(&summary_json);
        assert_eq!(windows.len(), 4);

        // Priority sorting should ensure Five Hour Limit comes before Weekly Limit
        assert_eq!(windows[0].label, "Gemini 5h");
        assert_eq!(windows[1].label, "Gemini Weekly");
        assert_eq!(windows[2].label, "Claude 5h");
        assert_eq!(windows[3].label, "Claude Weekly");

        // Remaining 0.6096169 => used (1.0 - 0.6096169) * 100 = 39.03831
        let delta = 1e-4;
        assert!((windows[0].used_percent - 39.03831).abs() < delta);
        assert!((windows[1].used_percent - 17.12704).abs() < delta);
        assert_eq!(windows[2].used_percent, 0.0);
        assert_eq!(windows[3].used_percent, 0.0);

        assert_eq!(
            windows[0].resets_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-06-22T14:38:32Z")
                    .unwrap()
                    .to_utc()
            )
        );
    }
}
