use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use reqwest::header::{HeaderMap, HeaderValue};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io/v1";

pub struct ElevenLabsProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl ElevenLabsProvider {
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
        std::env::var("ELEVENLABS_API_KEY")
            .or_else(|_| std::env::var("XI_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("ELEVENLABS_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for ElevenLabsProvider {
    fn name(&self) -> &str {
        "elevenlabs"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("ElevenLabs API key not configured. Set api_key or ELEVENLABS_API_KEY")?;
        let base_url = self.base_url();

        let mut headers = HeaderMap::new();
        headers.insert("xi-api-key", HeaderValue::from_str(&api_key)?);

        let resp = self
            .client
            .get(format!("{base_url}/user/subscription"))
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to ElevenLabs subscription API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("ElevenLabs subscription API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse ElevenLabs subscription response")?;
        let used = number_field(&json, &["character_count", "characterCount"]).unwrap_or(0.0);
        let limit = number_field(&json, &["character_limit", "characterLimit"]).unwrap_or(0.0);
        let used_percent = if limit > 0.0 {
            (used / limit * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let resets_at = number_field(
            &json,
            &[
                "next_character_count_reset_unix",
                "nextCharacterCountResetUnix",
            ],
        )
        .and_then(|ts| Utc.timestamp_opt(ts as i64, 0).single());

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Characters".to_string(),
                used_percent,
                limit: (limit > 0.0).then_some(limit),
                used: Some(used),
                unit: Some("chars".to_string()),
                resets_at,
            }],
            credits: None,
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
