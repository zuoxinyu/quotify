use anyhow::{Context, Result};
use chrono::Utc;

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

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
        proxy: Option<&str>,
    ) -> Self {
        Self {
            api_key,
            service_token,
            cookie_header,
            client: http_client(proxy),
        }
    }

    async fn resolve_cookie_header(&self) -> Result<String> {
        // 1. Config: cookie_header (full header string)
        if let Some(header) = &self.cookie_header
            && !header.is_empty()
        {
            return Ok(header.clone());
        }

        // 2. Config: service_token
        if let Some(token) = &self.service_token
            && !token.is_empty()
        {
            if token.contains('=') {
                return Ok(token.clone());
            }
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
            if token.contains('=') {
                return Ok(token);
            }
            return Ok(format!("serviceToken={token}"));
        }

        // 5. If everything fails, try to prompt webview login
        tracing::info!("No MiMo credentials found. Attempting WebView2 login...");
        let full_cookie =
            tokio::task::spawn_blocking(crate::webview_login::login_and_get_cookie).await??;

        if let Err(err) = crate::secrets::set("mimo", "cookie_header", &full_cookie) {
            tracing::error!("Failed to store MiMo cookie in Windows Credential Manager: {err}");
        }

        Ok(full_cookie)
    }
}

#[derive(Debug, serde::Deserialize)]
struct MimoApiResponse {
    data: Option<MimoApiData>,
}

#[derive(Debug, serde::Deserialize)]
struct MimoBalanceResponse {
    data: Option<MimoBalanceData>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MimoBalanceData {
    balance: Option<String>,
    #[allow(dead_code)]
    frozen_balance: Option<String>,
    currency: Option<String>,
    #[allow(dead_code)]
    overdraft_limit: Option<String>,
    #[allow(dead_code)]
    remaining_overdraft_limit: Option<String>,
    #[allow(dead_code)]
    gift_balance: Option<String>,
    #[allow(dead_code)]
    cash_balance: Option<String>,
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
        let cookie_header = self.resolve_cookie_header().await?;

        // 1. Fetch Plan Usage
        let usage_url = "https://platform.xiaomimimo.com/api/v1/tokenPlan/usage";
        let mut resp = self
            .client
            .get(usage_url)
            .header("Cookie", &cookie_header)
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
            .context("Failed to connect to MiMo Usage API")?;

        let mut current_cookie_header = cookie_header;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            tracing::info!("MiMo token expired. Attempting WebView2 login...");
            let full_cookie =
                tokio::task::spawn_blocking(crate::webview_login::login_and_get_cookie).await??;

            if let Err(err) = crate::secrets::set("mimo", "cookie_header", &full_cookie) {
                tracing::error!("Failed to store MiMo cookie in Windows Credential Manager: {err}");
            }

            current_cookie_header = full_cookie;

            // Retry request
            resp = self
                .client
                .get(usage_url)
                .header("Cookie", &current_cookie_header)
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
                .context("Failed to connect to MiMo API on retry")?;
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("MiMo Usage API error: {status} - {body}");
        }

        let usage_body: MimoApiResponse = resp
            .json()
            .await
            .context("Failed to parse MiMo tokenPlan/usage response")?;

        // 2. Fetch Balance
        let balance_url = "https://platform.xiaomimimo.com/api/v1/balance";
        let resp = self
            .client
            .get(balance_url)
            .header("Cookie", &current_cookie_header)
            .header("Content-Type", "application/json")
            .header("Accept", "*/*")
            .header("Accept-Language", "zh")
            .header("X-Timezone", "Asia/Shanghai")
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36",
            )
            .header("Referer", "https://platform.xiaomimimo.com/console/balance")
            .send()
            .await
            .context("Failed to connect to MiMo Balance API")?;

        let mut credits = None;
        if resp.status().is_success()
            && let Ok(balance_resp) = resp.json::<MimoBalanceResponse>().await
                && let Some(data) = balance_resp.data {
                    let total = data
                        .balance
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let currency = data.currency.unwrap_or_else(|| "CNY".to_string());
                    credits = Some(CreditsInfo {
                        balance: total,
                        currency,
                        total_granted: None,
                        topped_up: None,
                    });
                }

        let mut windows = Vec::new();

        if let Some(data) = usage_body.data {
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

            // If we didn't get credits from the balance API, try fallback to tokens remaining
            if credits.is_none()
                && let Some(usage) = &data.usage {
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
