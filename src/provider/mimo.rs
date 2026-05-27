use anyhow::{Context, Result};
use chrono::Utc;

use super::{CreditsInfo, Provider, UsageData, UsageWindow};
use crate::cookies;

pub struct MimoProvider {
    #[allow(dead_code)]
    api_key: String,
    service_token: Option<String>,
    cookie_header: Option<String>,
    client: reqwest::Client,
}

impl MimoProvider {
    pub fn new(
        api_key: String,
        service_token: Option<String>,
        cookie_header: Option<String>,
    ) -> Self {
        Self {
            api_key,
            service_token,
            cookie_header,
            client: reqwest::Client::new(),
        }
    }

    async fn find_cookie_header(&self) -> Result<String> {
        // 1. Config: cookie_header (full header string)
        if let Some(header) = &self.cookie_header {
            return Ok(header.clone());
        }

        // 2. Config: service_token
        if let Some(token) = &self.service_token {
            return Ok(format!("serviceToken={token}"));
        }

        // 3. Env: MIMO_COOKIE_HEADER (full header string)
        if let Ok(header) = std::env::var("MIMO_COOKIE_HEADER")
            && !header.is_empty()
        {
            return Ok(header);
        }

        // 4. Env: MIMO_SERVICE_TOKEN
        if let Ok(token) = std::env::var("MIMO_SERVICE_TOKEN")
            && !token.is_empty()
        {
            return Ok(format!("serviceToken={token}"));
        }

        // 5. Browser cookies
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

        anyhow::bail!(
            "No MiMo browser session found. Set MIMO_SERVICE_TOKEN, configure service_token in config, or log in at platform.xiaomimimo.com first"
        )
    }
}

#[derive(Debug, serde::Deserialize)]
struct MimoApiResponse {
    data: Option<MimoApiData>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MimoApiData {
    #[serde(default)]
    usage: Option<MimoUsageGroup>,
    #[serde(default)]
    month_usage: Option<MimoUsageGroup>,
}

#[derive(Debug, serde::Deserialize)]
struct MimoUsageGroup {
    #[serde(default)]
    items: Vec<MimoUsageItem>,
    #[serde(default)]
    #[allow(dead_code)]
    percent: Option<f64>,
}

#[derive(Debug, serde::Deserialize)]
struct MimoUsageItem {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    limit: Option<f64>,
    #[serde(default)]
    used: Option<f64>,
    #[serde(default)]
    percent: Option<f64>,
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

        if let Ok(resp) = serde_json::from_value::<MimoApiResponse>(body.clone())
            && let Some(data) = resp.data
        {
            // Parse monthly usage
            if let Some(month) = &data.month_usage {
                for item in &month.items {
                    let label = item_display_name(item, "Monthly");
                    let limit = item.limit.unwrap_or(0.0);
                    let used = item.used.unwrap_or(0.0);
                    let pct = item
                        .percent
                        .map(|p| (p * 100.0).clamp(0.0, 100.0))
                        .unwrap_or_else(|| {
                            if limit > 0.0 {
                                (used / limit * 100.0).clamp(0.0, 100.0)
                            } else {
                                0.0
                            }
                        });

                    windows.push(UsageWindow {
                        label,
                        used_percent: pct,
                        limit: Some(limit),
                        used: Some(used),
                        unit: Some("tokens".to_string()),
                        resets_at: None,
                    });
                }
            }

            // Parse plan usage
            if let Some(usage) = &data.usage {
                for item in &usage.items {
                    let label = item_display_name(item, "Plan");
                    let limit = item.limit.unwrap_or(0.0);
                    let used = item.used.unwrap_or(0.0);
                    let pct = item
                        .percent
                        .map(|p| (p * 100.0).clamp(0.0, 100.0))
                        .unwrap_or_else(|| {
                            if limit > 0.0 {
                                (used / limit * 100.0).clamp(0.0, 100.0)
                            } else {
                                0.0
                            }
                        });

                    windows.push(UsageWindow {
                        label,
                        used_percent: pct,
                        limit: Some(limit),
                        used: Some(used),
                        unit: Some("tokens".to_string()),
                        resets_at: None,
                    });
                }
            }

            // Compute credits from the main plan usage group
            if let Some(usage) = &data.usage {
                let total_limit: f64 = usage.items.iter().filter_map(|i| i.limit).sum();
                let total_used: f64 = usage.items.iter().filter_map(|i| i.used).sum();
                if total_limit > 0.0 {
                    credits = Some(CreditsInfo {
                        balance: total_limit - total_used,
                        currency: "tokens".to_string(),
                        total_granted: Some(total_limit),
                        topped_up: None,
                    });
                }
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

fn item_display_name(item: &MimoUsageItem, fallback: &str) -> String {
    match item.name.as_deref() {
        Some("month_total_token") => "Monthly".to_string(),
        Some("plan_total_token") => "Plan".to_string(),
        Some("compensation_total_token") => "Compensation".to_string(),
        Some(other) => other.replace('_', " "),
        None => fallback.to_string(),
    }
}
