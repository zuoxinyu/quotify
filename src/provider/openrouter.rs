use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

pub struct OpenRouterProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenRouterProvider {
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
        std::env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("OPENROUTER_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for OpenRouterProvider {
    fn name(&self) -> &str {
        "openrouter"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("OpenRouter API key not configured. Set api_key or OPENROUTER_API_KEY")?;
        let base_url = self.base_url();

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);
        if let Ok(referer) = std::env::var("OPENROUTER_HTTP_REFERER")
            && !referer.trim().is_empty()
        {
            headers.insert("HTTP-Referer", HeaderValue::from_str(referer.trim())?);
        }
        headers.insert(
            "X-Title",
            HeaderValue::from_str(
                &std::env::var("OPENROUTER_X_TITLE").unwrap_or_else(|_| "Quotify".to_string()),
            )?,
        );

        let credits_json = self
            .client
            .get(format!("{base_url}/credits"))
            .headers(headers.clone())
            .send()
            .await
            .context("Failed to connect to OpenRouter credits API")?;
        if !credits_json.status().is_success() {
            let status = credits_json.status();
            let body = credits_json.text().await.unwrap_or_default();
            anyhow::bail!("OpenRouter credits API error: {status} - {body}");
        }
        let credits_json: serde_json::Value = credits_json
            .json()
            .await
            .context("Failed to parse OpenRouter credits response")?;

        let data = credits_json.get("data").unwrap_or(&credits_json);
        let total_credits = number_field(data, &["total_credits", "totalCredits"]).unwrap_or(0.0);
        let total_usage = number_field(data, &["total_usage", "totalUsage"]).unwrap_or(0.0);
        let balance = (total_credits - total_usage).max(0.0);
        let used_percent = if total_credits > 0.0 {
            (total_usage / total_credits * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        let mut windows = Vec::new();
        if total_credits > 0.0 || total_usage > 0.0 {
            windows.push(UsageWindow {
                label: "Credits".to_string(),
                used_percent,
                limit: Some(total_credits),
                used: Some(total_usage),
                unit: Some("USD".to_string()),
                resets_at: None,
            });
        }

        if windows.is_empty() {
            windows.push(UsageWindow {
                label: "Credits".to_string(),
                used_percent: 0.0,
                limit: None,
                used: None,
                unit: Some("USD".to_string()),
                resets_at: None,
            });
        }

        Ok(UsageData {
            provider: self.name().to_string(),
            windows,
            credits: Some(CreditsInfo {
                balance,
                currency: "USD".to_string(),
                total_granted: Some(total_credits),
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn number_field(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
    })
}
