use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap};
use serde_json::json;
use std::path::PathBuf;

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://www.codebuff.com";

pub struct CodebuffProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl CodebuffProvider {
    pub fn new(api_key: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            api_key,
            base_url,
            client: http_client(proxy),
        }
    }

    pub fn credentials_file_exists() -> bool {
        credentials_path().is_some_and(|path| path.exists())
    }

    fn resolve_token(&self) -> Option<(String, bool)> {
        if let Ok(key) = std::env::var("CODEBUFF_API_KEY")
            && !key.trim().is_empty()
        {
            return Some((key.trim().to_string(), false));
        }
        if !self.api_key.trim().is_empty() {
            return Some((self.api_key.trim().to_string(), false));
        }
        read_cli_token().map(|token| (token, true))
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("CODEBUFF_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for CodebuffProvider {
    fn name(&self) -> &str {
        "codebuff"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let (token, session_token) = self.resolve_token().context(
            "Codebuff token not configured. Set api_key/CODEBUFF_API_KEY or run codebuff login",
        )?;
        let base_url = self.base_url();

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {token}").parse()?);
        headers.insert(CONTENT_TYPE, "application/json".parse()?);

        let usage_resp = self
            .client
            .post(format!("{base_url}/api/v1/usage"))
            .headers(headers.clone())
            .json(&json!({ "fingerprintId": "quotify-usage" }))
            .send()
            .await
            .context("Failed to connect to Codebuff usage API")?;

        if !usage_resp.status().is_success() {
            let status = usage_resp.status();
            let body = usage_resp.text().await.unwrap_or_default();
            anyhow::bail!("Codebuff usage API error: {status} - {body}");
        }

        let usage_json: serde_json::Value = usage_resp
            .json()
            .await
            .context("Failed to parse Codebuff usage response")?;
        let usage = usage_json.get("data").unwrap_or(&usage_json);
        let quota = number_field(usage, &["quota", "creditLimit", "creditsLimit", "limit"]);
        let used = number_field(usage, &["usage", "used", "creditsUsed"]).unwrap_or(0.0);
        let remaining = number_field(
            usage,
            &["remaining", "balance", "credits", "creditsRemaining"],
        );
        let total = quota.or_else(|| remaining.map(|remaining| remaining + used));
        let used_percent = total
            .filter(|total| *total > 0.0)
            .map(|total| (used / total * 100.0).clamp(0.0, 100.0))
            .unwrap_or(0.0);
        let resets_at = string_field(usage, &["nextQuotaReset", "resetAt", "resetsAt"])
            .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let mut windows = vec![UsageWindow {
            label: "Credits".to_string(),
            used_percent,
            limit: total,
            used: Some(used),
            unit: Some("credits".to_string()),
            resets_at,
        }];

        if session_token
            && let Ok(subscription) = fetch_subscription(&self.client, &base_url, &headers).await
        {
            let sub = subscription.get("data").unwrap_or(&subscription);
            let weekly_used = number_field(sub, &["weeklyUsed", "weekly_used"]);
            let weekly_limit = number_field(sub, &["weeklyLimit", "weekly_limit"]);
            if let (Some(weekly_used), Some(weekly_limit)) = (weekly_used, weekly_limit)
                && weekly_limit > 0.0
            {
                windows.push(UsageWindow {
                    label: "Weekly".to_string(),
                    used_percent: (weekly_used / weekly_limit * 100.0).clamp(0.0, 100.0),
                    limit: Some(weekly_limit),
                    used: Some(weekly_used),
                    unit: Some("requests".to_string()),
                    resets_at: None,
                });
            }
        }

        Ok(UsageData {
            provider: self.name().to_string(),
            windows,
            credits: remaining.map(|balance| CreditsInfo {
                balance,
                currency: "credits".to_string(),
                total_granted: total,
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

async fn fetch_subscription(
    client: &reqwest::Client,
    base_url: &str,
    headers: &HeaderMap,
) -> Result<serde_json::Value> {
    let resp = client
        .get(format!("{base_url}/api/user/subscription"))
        .headers(headers.clone())
        .send()
        .await
        .context("Failed to connect to Codebuff subscription API")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Codebuff subscription API error: {status} - {body}");
    }
    resp.json()
        .await
        .context("Failed to parse Codebuff subscription response")
}

fn credentials_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join(".config")
            .join("manicode")
            .join("credentials.json")
    })
}

fn read_cli_token() -> Option<String> {
    let path = credentials_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get("default")
        .and_then(|default| default.get("authToken"))
        .or_else(|| json.get("authToken"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

fn number_field(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
    })
}

fn string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|v| v.as_str()).map(str::to_string))
}
