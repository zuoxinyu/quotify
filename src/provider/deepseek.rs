use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde::Deserialize;

use super::{CreditsInfo, Provider, UsageData, http_client};

pub struct DeepSeekProvider {
    api_key: String,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct BalanceResponse {
    balance_infos: Vec<BalanceInfo>,
}

#[derive(Debug, Deserialize)]
struct BalanceInfo {
    currency: String,
    total_balance: String,
    granted_balance: String,
    topped_up_balance: String,
}

impl DeepSeekProvider {
    pub fn new(api_key: String, proxy: Option<&str>) -> Self {
        Self {
            api_key,
            client: http_client(proxy),
        }
    }

    fn resolve_api_key(api_key: &str) -> Option<String> {
        if !api_key.is_empty() {
            return Some(api_key.to_string());
        }
        std::env::var("DEEPSEEK_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
    }
}

#[async_trait::async_trait]
impl Provider for DeepSeekProvider {
    fn name(&self) -> &str {
        "deepseek"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = Self::resolve_api_key(&self.api_key).context(
            "DeepSeek API key not configured. Set api_key in config or DEEPSEEK_API_KEY env var",
        )?;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let resp = self
            .client
            .get("https://api.deepseek.com/user/balance")
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to DeepSeek API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("DeepSeek API error: {status} - {body}");
        }

        let balance: BalanceResponse = resp
            .json()
            .await
            .context("Failed to parse DeepSeek balance response")?;

        let info = balance
            .balance_infos
            .first()
            .context("DeepSeek balance response did not contain balance information")?;
        let total: f64 = info
            .total_balance
            .parse()
            .context("Failed to parse total_balance")?;
        let granted: f64 = info
            .granted_balance
            .parse()
            .context("Failed to parse granted_balance")?;
        let topped_up: f64 = info
            .topped_up_balance
            .parse()
            .context("Failed to parse topped_up_balance")?;

        // DeepSeek exposes current balances rather than a usage endpoint. The
        // natural-period spend windows are derived from persisted balance
        // samples by `usage_history` after this fetch completes.
        let credits = Some(CreditsInfo {
            balance: total,
            currency: info.currency.clone(),
            total_granted: Some(granted),
            topped_up: Some(topped_up),
        });

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: Vec::new(),
            credits,
            subscription_tier: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}
