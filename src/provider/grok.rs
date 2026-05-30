use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://api.x.ai";

pub struct GrokProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl GrokProvider {
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
        std::env::var("XAI_API_KEY")
            .or_else(|_| std::env::var("GROK_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("XAI_API_BASE")
                .or_else(|_| std::env::var("GROK_API_BASE"))
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for GrokProvider {
    fn name(&self) -> &str {
        "grok"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self.resolve_api_key().context(
            "Grok/xAI API key not configured. Set api_key, XAI_API_KEY, or GROK_API_KEY",
        )?;
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .get(format!("{}/v1/models", self.base_url()))
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to xAI models API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("xAI models API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse xAI models response")?;
        let model_count = json
            .get("data")
            .or_else(|| json.get("models"))
            .and_then(|v| v.as_array())
            .map(|models| models.len() as f64);

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "API Access".to_string(),
                used_percent: 0.0,
                limit: None,
                used: model_count,
                unit: Some("models".to_string()),
                resets_at: None,
            }],
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}
