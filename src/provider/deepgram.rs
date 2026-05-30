use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://api.deepgram.com/v1";

pub struct DeepgramProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl DeepgramProvider {
    pub fn new(api_key: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            api_key,
            base_url,
            client: http_client(proxy),
        }
    }

    fn resolve_api_key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.trim().to_string());
        }
        std::env::var("DEEPGRAM_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("DEEPGRAM_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for DeepgramProvider {
    fn name(&self) -> &str {
        "deepgram"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("Deepgram API key not configured. Set api_key or DEEPGRAM_API_KEY")?;
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Token {api_key}").parse()?);

        let base_url = self.base_url();
        let project_ids = if let Ok(project_id) = std::env::var("DEEPGRAM_PROJECT_ID") {
            vec![project_id]
        } else {
            self.list_projects(&base_url, &headers).await?
        };

        if project_ids.is_empty() {
            anyhow::bail!("Deepgram API returned no projects for this API key");
        }

        let start = (Utc::now() - Duration::days(30))
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        let end = Utc::now().date_naive().format("%Y-%m-%d").to_string();
        let mut requests = 0.0;
        let mut audio_hours = 0.0;
        let mut tokens = 0.0;

        for project_id in &project_ids {
            let usage = self
                .client
                .get(format!("{base_url}/projects/{project_id}/usage/breakdown"))
                .headers(headers.clone())
                .query(&[("start", start.as_str()), ("end", end.as_str())])
                .send()
                .await
                .with_context(|| {
                    format!("Failed to connect to Deepgram usage API for {project_id}")
                })?;

            if !usage.status().is_success() {
                let status = usage.status();
                let body = usage.text().await.unwrap_or_default();
                anyhow::bail!("Deepgram usage API error: {status} - {body}");
            }

            let json: serde_json::Value = usage
                .json()
                .await
                .context("Failed to parse Deepgram usage response")?;
            requests += sum_by_key(
                &json,
                &["requests", "request_count", "requestCount", "count"],
            );
            audio_hours += sum_by_key(&json, &["hours", "audio_hours", "audioHours"]);
            tokens += sum_by_key(&json, &["tokens", "token_count", "tokenCount"]);
        }

        let mut windows = vec![UsageWindow {
            label: "Requests 30d".to_string(),
            used_percent: 0.0,
            limit: None,
            used: Some(requests),
            unit: Some("requests".to_string()),
            resets_at: None,
        }];
        if audio_hours > 0.0 {
            windows.push(UsageWindow {
                label: "Audio 30d".to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(audio_hours),
                unit: Some("hours".to_string()),
                resets_at: None,
            });
        }
        if tokens > 0.0 {
            windows.push(UsageWindow {
                label: "Tokens 30d".to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(tokens),
                unit: Some("tokens".to_string()),
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

impl DeepgramProvider {
    async fn list_projects(&self, base_url: &str, headers: &HeaderMap) -> Result<Vec<String>> {
        let resp = self
            .client
            .get(format!("{base_url}/projects"))
            .headers(headers.clone())
            .send()
            .await
            .context("Failed to connect to Deepgram projects API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Deepgram projects API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Deepgram projects response")?;
        let Some(projects) = json.get("projects").and_then(|v| v.as_array()) else {
            return Ok(Vec::new());
        };

        Ok(projects
            .iter()
            .filter_map(|project| {
                project
                    .get("project_id")
                    .or_else(|| project.get("projectId"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect())
    }
}

fn sum_by_key(value: &serde_json::Value, keys: &[&str]) -> f64 {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(key, value)| {
                let own = if keys.iter().any(|needle| key.eq_ignore_ascii_case(needle)) {
                    value
                        .as_f64()
                        .or_else(|| value.as_str()?.parse().ok())
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                own + sum_by_key(value, keys)
            })
            .sum(),
        serde_json::Value::Array(values) => values.iter().map(|v| sum_by_key(v, keys)).sum(),
        _ => 0.0,
    }
}
