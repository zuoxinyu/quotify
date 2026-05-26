use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;
use std::path::{Path, PathBuf};

use super::{Provider, UsageData, UsageWindow};

const ANTIGRAVITY_QUOTA_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota";
const ANTIGRAVITY_LOAD_CODE_ASSIST_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const GOOGLE_PROJECTS_URL: &str = "https://cloudresourcemanager.googleapis.com/v1/projects";

pub struct AntigravityProvider {
    api_key: Option<String>,
    client: reqwest::Client,
}

#[derive(Clone)]
struct AntigravityCredentials {
    path: PathBuf,
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
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }

    fn oauth_credentials_path() -> Option<PathBuf> {
        Some(dirs::home_dir()?.join(".gemini").join("oauth_creds.json"))
    }

    fn read_oauth_credentials() -> Option<AntigravityCredentials> {
        let path = Self::oauth_credentials_path()?;
        if !path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;
        let access_token = string_field(&json, &["access_token", "accessToken"])?;
        let refresh_token = string_field(&json, &["refresh_token", "refreshToken"]);
        let expiry_millis = json
            .get("expiry_date")
            .or_else(|| json.get("expiryDate"))
            .and_then(expiry_millis);

        Some(AntigravityCredentials {
            path,
            json,
            access_token,
            refresh_token,
            expiry_millis,
        })
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

#[async_trait::async_trait]
impl Provider for AntigravityProvider {
    fn name(&self) -> &str {
        "antigravity"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
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
                "Antigravity API key is configured, but usage quota requires Antigravity CLI OAuth credentials at ~/.gemini/oauth_creds.json"
            } else {
                "Antigravity OAuth credentials not found. Run Antigravity CLI OAuth login first"
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
            Ok(project) => project,
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

        std::fs::write(
            &credentials.path,
            serde_json::to_string_pretty(&credentials.json)?,
        )
        .with_context(|| {
            format!(
                "Failed to write refreshed Antigravity credentials to {:?}",
                credentials.path
            )
        })?;

        Ok(access_token)
    }

    async fn load_code_assist_project(&self, access_token: &str) -> Result<Option<String>> {
        let resp = self
            .client
            .post(ANTIGRAVITY_LOAD_CODE_ASSIST_URL)
            .bearer_auth(access_token)
            .json(&json!({
                "metadata": {
                    "ideType": "ANTIGRAVITY_CLI",
                    "pluginType": "ANTIGRAVITY"
                }
            }))
            .send()
            .await
            .context("Failed to call Antigravity loadCodeAssist")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Antigravity loadCodeAssist error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Antigravity loadCodeAssist response")?;
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
            .context("Failed to call Antigravity quota API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Antigravity quota API error: {status} - {body}");
        }

        resp.json()
            .await
            .context("Failed to parse Antigravity quota response")
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
        serde_json::Value::String(s) => s.parse::<i64>().ok().map(|raw| {
            if raw < 10_000_000_000 {
                raw.saturating_mul(1000)
            } else {
                raw
            }
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
        ("Pro", "pro"),
        ("Flash Lite", "flash-lite"),
        ("Flash", "flash"),
    ];

    let mut windows = Vec::new();
    for (label, needle) in groups {
        if let Some(window) = best_antigravity_window(
            label,
            entries
                .iter()
                .filter(|entry| entry.model_id.to_ascii_lowercase().contains(needle)),
        ) {
            windows.push(window);
        }
    }

    if windows.is_empty() {
        for entry in entries {
            let used_percent = (100.0 - entry.remaining_percent).clamp(0.0, 100.0);
            windows.push(UsageWindow {
                label: entry.model_id,
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
                    .filter(|hint| hint.to_ascii_lowercase().contains("antigravity"))
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
