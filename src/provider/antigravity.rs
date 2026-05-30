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

#[derive(Clone)]
struct AntigravityQuotaEntry {
    model_id: String,
    remaining_percent: f64,
    reset_time: Option<chrono::DateTime<Utc>>,
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

        let access_token = self.resolve_access_token(credentials.clone()).await?;
        let quota = match self.fetch_quota_with_token(&access_token).await {
            Ok(quota) => quota,
            Err(e) if is_auth_error(&e) => {
                tracing::debug!("Antigravity quota token rejected, refreshing and retrying: {e}");
                let refresh_token = credentials.refresh_token.clone().context(
                    "Antigravity OAuth token was rejected and no refresh token is available",
                )?;
                let refreshed = self
                    .refresh_access_token(credentials, &refresh_token)
                    .await
                    .context("Antigravity OAuth token was rejected and refresh failed")?;
                self.fetch_quota_with_token(&refreshed).await?
            }
            Err(e) => return Err(e),
        };
        let windows = parse_antigravity_quota(&quota);

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

impl AntigravityProvider {
    async fn fetch_quota_with_token(&self, access_token: &str) -> Result<serde_json::Value> {
        let project = match self.load_code_assist_project(access_token).await {
            Ok(project) => {
                tracing::debug!("Antigravity loadCodeAssist project: {project:?}");
                project
            }
            Err(e) => {
                tracing::debug!("Antigravity loadCodeAssist project discovery failed: {e}");
                self.discover_project_fallback(access_token)
                    .await
                    .ok()
                    .flatten()
            }
        };

        self.retrieve_user_quota(access_token, project.as_deref())
            .await
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

        let url = format!("{base_url}{ANTIGRAVITY_GET_AVAILABLE_MODELS_PATH}");
        let resp = local_client
            .post(url)
            .header("x-codeium-csrf-token", csrf)
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
        Ok(select_antigravity_project(&json))
    }

    async fn retrieve_user_quota(
        &self,
        access_token: &str,
        project: Option<&str>,
    ) -> Result<serde_json::Value> {
        let body = if let Some(project) = project.filter(|project| !project.is_empty()) {
            json!({ "project": project })
        } else {
            json!({})
        };

        let resp = self
            .client
            .post(ANTIGRAVITY_QUOTA_URL)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to call Antigravity quota API: {e:?}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Antigravity quota API error: {status} - {body}");
        }

        let json = resp
            .json()
            .await
            .context("Failed to parse Antigravity quota response")?;
        tracing::debug!("Antigravity quota raw response: {json:?}");
        Ok(json)
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

        entries.push((
            label,
            AntigravityQuotaEntry {
                model_id,
                remaining_percent,
                reset_time: resets_at,
            },
        ));
    }

    if entries.is_empty() {
        return Vec::new();
    }

    let groups = [
        ("Claude", ModelMatcher::Claude),
        ("Gemini Pro", ModelMatcher::Needles(&["pro"])),
        ("Gemini Flash", ModelMatcher::Needles(&["flash"])),
    ];

    let mut windows = Vec::new();
    let mut matched_indices = std::collections::HashSet::new();

    for (label, matcher) in groups {
        let mut group_entries = Vec::new();
        for (idx, (display_name, entry)) in entries.iter().enumerate() {
            if !matched_indices.contains(&idx)
                && (matcher.matches(&entry.model_id) || matcher.matches(display_name))
            {
                matched_indices.insert(idx);
                group_entries.push(entry);
            }
        }

        if let Some(window) = best_antigravity_window(label, group_entries.into_iter()) {
            windows.push(window);
        }
    }

    // Add any unmatched entries as individual windows
    for (idx, (display_name, entry)) in entries.iter().enumerate() {
        if !matched_indices.contains(&idx) {
            let label = display_name
                .strip_prefix("Gemini ")
                .unwrap_or(display_name)
                .to_string();
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
    }

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
    collect_quota_entries(value, None, &mut entries);

    if entries.is_empty() {
        return Vec::new();
    }

    let groups = [
        ("Claude", ModelMatcher::Claude),
        ("Gemini Pro", ModelMatcher::Needles(&["pro"])),
        ("Gemini Flash", ModelMatcher::Needles(&["flash"])),
    ];

    let mut windows = Vec::new();
    let mut matched_indices = std::collections::HashSet::new();

    for (label, matcher) in groups {
        let mut group_entries = Vec::new();
        for (idx, entry) in entries.iter().enumerate() {
            if !matched_indices.contains(&idx) && matcher.matches(&entry.model_id) {
                matched_indices.insert(idx);
                group_entries.push(entry);
            }
        }

        if let Some(window) = best_antigravity_window(label, group_entries.into_iter()) {
            windows.push(window);
        }
    }

    // Add any unmatched entries as individual windows
    for (idx, entry) in entries.iter().enumerate() {
        if !matched_indices.contains(&idx) {
            let used_percent = (100.0 - entry.remaining_percent).clamp(0.0, 100.0);
            windows.push(UsageWindow {
                label: entry.model_id.clone(),
                used_percent,
                limit: Some(100.0),
                used: Some(used_percent),
                unit: Some("%".to_string()),
                resets_at: entry.reset_time,
            });
        }
    }

    windows
}

fn collect_quota_entries(
    value: &serde_json::Value,
    model_hint: Option<&str>,
    out: &mut Vec<AntigravityQuotaEntry>,
) {
    match value {
        serde_json::Value::Object(map) => {
            let model_id = string_field(
                value,
                &["modelId", "model_id", "model", "modelName", "name"],
            )
            .or_else(|| {
                model_hint
                    .filter(|hint| {
                        let lower = hint.to_ascii_lowercase();
                        lower.contains("antigravity")
                            || lower.contains("gemini")
                            || lower.contains("claude")
                    })
                    .map(ToString::to_string)
            });

            let remaining = map
                .get("remainingFraction")
                .or_else(|| map.get("remaining_fraction"))
                .or_else(|| map.get("remainingPercent"))
                .or_else(|| map.get("remaining_percent"))
                .and_then(number_value);

            if let (Some(model_id), Some(remaining)) = (model_id, remaining) {
                let remaining_percent = if remaining <= 1.0 {
                    remaining * 100.0
                } else {
                    remaining
                }
                .clamp(0.0, 100.0);

                let reset_time = string_field(value, &["resetTime", "reset_time", "resetsAt"])
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.to_utc());

                out.push(AntigravityQuotaEntry {
                    model_id,
                    remaining_percent,
                    reset_time,
                });
            }

            for (key, child) in map {
                collect_quota_entries(child, Some(key), out);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_quota_entries(child, model_hint, out);
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

#[derive(Clone, Copy)]
enum ModelMatcher {
    Claude,
    Needles(&'static [&'static str]),
}

impl ModelMatcher {
    fn matches(self, model_id: &str) -> bool {
        let lower = model_id.to_ascii_lowercase();
        match self {
            Self::Claude => lower.contains("claude"),
            Self::Needles(needles) => needles.iter().all(|needle| lower.contains(needle)),
        }
    }
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
