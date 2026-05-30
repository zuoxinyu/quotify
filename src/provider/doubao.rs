use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde_json::json;

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str = "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions";
const PROBE_MODELS: [&str; 3] = [
    "doubao-seed-2.0-code",
    "doubao-1.5-pro-32k",
    "doubao-lite-32k",
];

pub struct DoubaoProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl DoubaoProvider {
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
        std::env::var("ARK_API_KEY")
            .or_else(|_| std::env::var("VOLCENGINE_API_KEY"))
            .or_else(|_| std::env::var("DOUBAO_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().to_string()
        } else {
            std::env::var("DOUBAO_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for DoubaoProvider {
    fn name(&self) -> &str {
        "doubao"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self.resolve_api_key().context(
            "Doubao API key not configured. Set api_key, ARK_API_KEY, or DOUBAO_API_KEY",
        )?;
        let url = self.url();

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let mut last_error = None;
        for model in PROBE_MODELS {
            let resp = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&json!({
                    "model": model,
                    "messages": [{"role": "user", "content": "ping"}],
                    "max_tokens": 1,
                    "stream": false
                }))
                .send()
                .await
                .context("Failed to connect to Doubao Ark API")?;

            let status = resp.status();
            let headers = resp.headers().clone();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                last_error = Some(format!("{status} - {body}"));
                continue;
            }

            let limit = header_number(&headers, "x-ratelimit-limit-requests");
            let remaining = header_number(&headers, "x-ratelimit-remaining-requests");
            let reset_seconds = header_number(&headers, "x-ratelimit-reset-requests");
            let mut windows = Vec::new();
            if let (Some(limit), Some(remaining)) = (limit, remaining) {
                let used = (limit - remaining).max(0.0);
                windows.push(UsageWindow {
                    label: "Requests".to_string(),
                    used_percent: if limit > 0.0 {
                        (used / limit * 100.0).clamp(0.0, 100.0)
                    } else {
                        0.0
                    },
                    limit: Some(limit),
                    used: Some(used),
                    unit: Some("requests".to_string()),
                    resets_at: reset_seconds
                        .map(|seconds| Utc::now() + Duration::seconds(seconds as i64)),
                });
            }

            if windows.is_empty() {
                windows.push(UsageWindow {
                    label: "API key active".to_string(),
                    used_percent: 0.0,
                    limit: None,
                    used: None,
                    unit: None,
                    resets_at: None,
                });
            }

            return Ok(UsageData {
                provider: self.name().to_string(),
                windows,
                credits: None,
                fetched_at: Utc::now(),
                error: None,
            });
        }

        anyhow::bail!(
            "Doubao Ark probe failed for all configured models: {}",
            last_error.unwrap_or_else(|| "unknown error".to_string())
        )
    }
}

fn header_number(headers: &reqwest::header::HeaderMap, name: &str) -> Option<f64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
}
