use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str = "https://api.github.com/copilot_internal/user";

pub struct CopilotProvider {
    token: String,
    base_url: String,
    client: reqwest::Client,
}

impl CopilotProvider {
    pub fn new(token: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            token,
            base_url,
            client: http_client(proxy),
        }
    }

    fn resolve_token(&self) -> Option<String> {
        if !self.token.trim().is_empty() {
            return Some(self.token.trim().to_string());
        }
        std::env::var("GITHUB_COPILOT_TOKEN")
            .or_else(|_| std::env::var("COPILOT_TOKEN"))
            .or_else(|_| std::env::var("GITHUB_TOKEN"))
            .ok()
            .filter(|token| !token.trim().is_empty())
    }

    fn endpoint(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().to_string()
        } else {
            std::env::var("COPILOT_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for CopilotProvider {
    fn name(&self) -> &str {
        "copilot"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let token = self.resolve_token().context(
            "Copilot token not configured. Set api_key, GITHUB_COPILOT_TOKEN, or COPILOT_TOKEN",
        )?;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {token}").parse()?);
        headers.insert(USER_AGENT, HeaderValue::from_static("Quotify"));

        let resp = self
            .client
            .get(self.endpoint())
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to GitHub Copilot usage API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Copilot usage API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Copilot usage response")?;
        let root = json.get("data").unwrap_or(&json);
        let mut windows = Vec::new();

        push_window(
            &mut windows,
            "Premium",
            find_number(
                root,
                &[
                    "premium_interactions",
                    "premiumInteractions",
                    "premium_used",
                    "premiumUsed",
                ],
            ),
            find_number(
                root,
                &[
                    "premium_interactions_limit",
                    "premiumInteractionsLimit",
                    "premium_limit",
                    "premiumLimit",
                ],
            ),
            find_reset(
                root,
                &[
                    "premium_interactions_reset_date",
                    "premiumResetDate",
                    "resetDate",
                ],
            ),
        );
        push_window(
            &mut windows,
            "Chat",
            find_number(
                root,
                &[
                    "chat_interactions",
                    "chatInteractions",
                    "chat_used",
                    "chatUsed",
                ],
            ),
            find_number(
                root,
                &[
                    "chat_interactions_limit",
                    "chatInteractionsLimit",
                    "chat_limit",
                    "chatLimit",
                ],
            ),
            find_reset(
                root,
                &["chat_interactions_reset_date", "chatResetDate", "resetDate"],
            ),
        );

        if windows.is_empty() {
            windows.push(UsageWindow {
                label: "Connected".to_string(),
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
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn push_window(
    windows: &mut Vec<UsageWindow>,
    label: &str,
    used: Option<f64>,
    limit: Option<f64>,
    resets_at: Option<DateTime<Utc>>,
) {
    let Some(used) = used else {
        return;
    };
    let used_percent = limit
        .filter(|limit| *limit > 0.0)
        .map(|limit| (used / limit * 100.0).clamp(0.0, 100.0))
        .unwrap_or(0.0);
    windows.push(UsageWindow {
        label: label.to_string(),
        used_percent,
        limit,
        used: Some(used),
        unit: Some("requests".to_string()),
        resets_at,
    });
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
                if let Some(number) = find_number(val, keys) {
                    return Some(number);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(|v| find_number(v, keys)),
        _ => None,
    }
}

fn find_reset(value: &serde_json::Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    find_string(value, keys)
        .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn find_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                if keys.iter().any(|needle| key.eq_ignore_ascii_case(needle))
                    && let Some(raw) = val.as_str()
                {
                    return Some(raw.to_string());
                }
                if let Some(raw) = find_string(val, keys) {
                    return Some(raw);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(|v| find_string(v, keys)),
        _ => None,
    }
}
