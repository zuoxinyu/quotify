use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str = "https://crof.ai/usage_api/";

pub struct CrofProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl CrofProvider {
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
        std::env::var("CROF_API_KEY")
            .or_else(|_| std::env::var("CROFAI_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().to_string()
        } else {
            std::env::var("CROF_USAGE_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for CrofProvider {
    fn name(&self) -> &str {
        "crof"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("Crof API key not configured. Set api_key or CROF_API_KEY")?;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .get(self.url())
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to Crof usage API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Crof usage API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Crof usage response")?;
        let requests_plan = number_field(&json, &["requests_plan", "requestsPlan", "requestLimit"]);
        let usable_requests = number_field(
            &json,
            &["usable_requests", "usableRequests", "remainingRequests"],
        );
        let credits = number_field(&json, &["credits", "credit_balance", "creditBalance"]);

        let mut windows = Vec::new();
        if let (Some(limit), Some(remaining)) = (requests_plan, usable_requests) {
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
                resets_at: next_utc_midnight(),
            });
        }

        if let Some(balance) = credits {
            windows.push(UsageWindow {
                label: "Credits".to_string(),
                used_percent: 0.0,
                limit: None,
                used: None,
                unit: Some("USD".to_string()),
                resets_at: None,
            });

            return Ok(UsageData {
                provider: self.name().to_string(),
                windows,
                credits: Some(CreditsInfo {
                    balance,
                    currency: "USD".to_string(),
                    total_granted: None,
                    topped_up: None,
                }),
                fetched_at: Utc::now(),
                error: None,
            });
        }

        if windows.is_empty() {
            windows.push(UsageWindow {
                label: "Usage".to_string(),
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

fn next_utc_midnight() -> Option<chrono::DateTime<Utc>> {
    let now = Utc::now();
    let tomorrow = NaiveDate::from_ymd_opt(now.year(), now.month(), now.day())?
        .checked_add_signed(Duration::days(1))?;
    Utc.from_local_datetime(&tomorrow.and_hms_opt(0, 0, 0)?)
        .single()
}

fn number_field(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
    })
}
