use anyhow::{Context, Result};
use chrono::Utc;

use super::{CreditsInfo, Provider, UsageData, UsageWindow};
use crate::cookies;

pub struct MimoProvider {
    #[allow(dead_code)]
    api_key: String,
    client: reqwest::Client,
}

impl MimoProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }

    async fn find_cookie_header(&self) -> Result<String> {
        if let Ok(token) = std::env::var("MIMO_SERVICE_TOKEN")
            && !token.is_empty()
        {
            return Ok(format!("serviceToken={token}"));
        }

        // First try exact domain match (covers most cases)
        let exact_domains = [
            "platform.xiaomimimo.com",
            ".platform.xiaomimimo.com",
            "xiaomimimo.com",
            ".xiaomimimo.com",
        ];
        match cookies::find_cookie_header(&exact_domains).await {
            Ok(header) => return Ok(header),
            Err(e) => {
                tracing::debug!("MiMo: exact domain search failed: {e}");
            }
        }

        // Fallback: search for any cookie with "serviceToken" name across xiaomimimo variants
        for domain in &[
            "platform.xiaomimimo.com",
            "xiaomimimo.com",
            "www.xiaomimimo.com",
        ] {
            if let Ok(token) = cookies::find_cookie(domain, "serviceToken").await {
                tracing::debug!("MiMo: found serviceToken via find_cookie for {domain}");
                let full_cookie =
                    cookies::find_cookie_header(&[domain, &format!(".{domain}")]).await;
                match full_cookie {
                    Ok(header) => return Ok(header),
                    Err(_) => return Ok(format!("serviceToken={token}")),
                }
            }
        }

        anyhow::bail!("No MiMo browser session found. Log in at platform.xiaomimimo.com first")
    }
}

#[derive(Debug, serde::Deserialize)]
struct TokenPlanUsageResponse {
    data: Option<TokenPlanUsageData>,
}

#[derive(Debug, serde::Deserialize)]
struct TokenPlanUsageData {
    #[serde(default)]
    plans: Vec<TokenPlan>,
    #[serde(default)]
    total_used_tokens: Option<f64>,
    #[serde(default)]
    total_remaining_tokens: Option<f64>,
}

#[derive(Debug, serde::Deserialize)]
struct TokenPlan {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    plan_name: Option<String>,
    #[serde(default)]
    used_tokens: Option<f64>,
    #[serde(default)]
    total_tokens: Option<f64>,
    #[serde(default)]
    remaining_tokens: Option<f64>,
    #[serde(default)]
    expire_time: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    status: Option<String>,
}

#[async_trait::async_trait]
impl Provider for MimoProvider {
    fn name(&self) -> &str {
        "mimo"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let cookie_header = self.find_cookie_header().await?;

        let url = "https://platform.xiaomimimo.com/api/v1/tokenPlan/usage";

        let resp = self
            .client
            .get(url)
            .header("Cookie", cookie_header)
            .header("Content-Type", "application/json")
            .header("Accept", "*/*")
            .header("Accept-Language", "zh")
            .header("X-Timezone", "Asia/Shanghai")
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36",
            )
            .header("Referer", "https://platform.xiaomimimo.com/console/plan-manage")
            .send()
            .await
            .context("Failed to connect to MiMo API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("MiMo API error: {status} - {body}");
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse MiMo tokenPlan/usage response")?;

        tracing::debug!("MiMo tokenPlan/usage response: {body:#?}");

        let mut windows = Vec::new();
        let mut credits = None;

        if let Ok(usage) = serde_json::from_value::<TokenPlanUsageResponse>(body.clone())
            && let Some(data) = usage.data
        {
            for plan in &data.plans {
                let label = plan
                    .plan_name
                    .as_deref()
                    .or(plan.name.as_deref())
                    .unwrap_or("Plan");

                let total = plan.total_tokens.unwrap_or(0.0);
                let used = plan.used_tokens.unwrap_or(0.0);
                let _remaining = plan.remaining_tokens.unwrap_or(total - used);

                let used_percent = if total > 0.0 {
                    (used / total * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };

                let resets_at = plan
                    .expire_time
                    .as_deref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.to_utc());

                windows.push(UsageWindow {
                    label: label.to_string(),
                    used_percent,
                    limit: Some(total),
                    used: Some(used),
                    unit: Some("tokens".to_string()),
                    resets_at,
                });
            }

            let total_used = data.total_used_tokens.unwrap_or(0.0);
            let total_remaining = data.total_remaining_tokens.unwrap_or(0.0);
            if total_used > 0.0 || total_remaining > 0.0 {
                credits = Some(CreditsInfo {
                    balance: total_remaining,
                    currency: "tokens".to_string(),
                    total_granted: Some(total_used + total_remaining),
                    topped_up: None,
                });
            }
        }

        if windows.is_empty() {
            windows.push(UsageWindow {
                label: "No data".to_string(),
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
            credits,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}
