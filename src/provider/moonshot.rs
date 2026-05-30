use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://api.moonshot.cn/v1";

pub struct MoonshotProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl MoonshotProvider {
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
        std::env::var("MOONSHOT_API_KEY")
            .or_else(|_| std::env::var("KIMI_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("MOONSHOT_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for MoonshotProvider {
    fn name(&self) -> &str {
        "moonshot"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("Moonshot/Kimi API key not configured. Set api_key or MOONSHOT_API_KEY")?;
        let base_url = self.base_url();

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .get(format!("{base_url}/users/me/balance"))
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to Moonshot balance API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Moonshot balance API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Moonshot balance response")?;
        let data = json
            .get("data")
            .or_else(|| json.get("result"))
            .unwrap_or(&json);
        let balance = number_by_keys(
            data,
            &[
                "available_balance",
                "availableBalance",
                "balance",
                "available",
                "remaining",
            ],
        )
        .unwrap_or(0.0);
        let granted = number_by_keys(
            data,
            &[
                "total_balance",
                "totalBalance",
                "grant_balance",
                "grantedBalance",
                "granted",
            ],
        );
        let currency = string_by_keys(data, &["currency", "currency_type", "currencyType"])
            .unwrap_or_else(|| "CNY".to_string());

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Balance".to_string(),
                used_percent: 0.0,
                limit: granted,
                used: granted.map(|limit| (limit - balance).max(0.0)),
                unit: Some(currency.clone()),
                resets_at: None,
            }],
            credits: Some(CreditsInfo {
                balance,
                currency,
                total_granted: granted,
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn number_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
    })
}

fn string_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    })
}
