use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use reqwest::header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue};

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str = "https://bailian.console.aliyun.com/data/api.json?action=GetSubscriptionSummary&product=BssOpenAPI-V3&_tag=";

pub struct AlibabaTokenProvider {
    cookie: String,
    base_url: String,
    client: reqwest::Client,
}

impl AlibabaTokenProvider {
    pub fn new(cookie: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            cookie,
            base_url,
            client: http_client(proxy),
        }
    }

    fn resolve_cookie(&self) -> Option<String> {
        if !self.cookie.trim().is_empty() {
            return Some(cookie_header(self.cookie.trim()));
        }
        std::env::var("ALIBABA_TOKEN_PLAN_COOKIE")
            .or_else(|_| std::env::var("ALIBABA_TOKEN_COOKIE"))
            .ok()
            .filter(|cookie| !cookie.trim().is_empty())
            .map(|cookie| cookie_header(cookie.trim()))
    }

    fn endpoint(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().to_string()
        } else {
            std::env::var("ALIBABA_TOKEN_PLAN_QUOTA_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for AlibabaTokenProvider {
    fn name(&self) -> &str {
        "alibabatoken"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let cookie = self.resolve_cookie().context(
            "Alibaba Token Plan cookie not configured. Set api_key or ALIBABA_TOKEN_PLAN_COOKIE",
        )?;
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(&cookie)?);
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );

        let params = serde_json::json!({"ProductCode":"sfm_tokenplanteams_dp_cn"}).to_string();
        let form = [
            ("product", "BssOpenAPI-V3"),
            ("action", "GetSubscriptionSummary"),
            ("region", "cn-beijing"),
            ("params", params.as_str()),
        ];

        let resp = self
            .client
            .post(self.endpoint())
            .headers(headers)
            .form(&form)
            .send()
            .await
            .context("Failed to connect to Alibaba Token Plan API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Alibaba Token Plan API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Alibaba Token Plan response")?;
        let total = find_number(&json, &["TotalValue", "totalValue", "total_value"]).unwrap_or(0.0);
        let remaining = find_number(
            &json,
            &[
                "TotalSurplusValue",
                "totalSurplusValue",
                "total_surplus_value",
                "SurplusValue",
                "surplusValue",
            ],
        )
        .unwrap_or(0.0);
        let used = (total - remaining).max(0.0);
        let used_percent = if total > 0.0 {
            (used / total * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let resets_at = find_string(
            &json,
            &[
                "NearestExpireDate",
                "nearestExpireDate",
                "nearest_expire_date",
                "ExpireDate",
                "expireDate",
            ],
        )
        .and_then(|raw| parse_reset(&raw));

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Token Plan".to_string(),
                used_percent,
                limit: (total > 0.0).then_some(total),
                used: Some(used),
                unit: Some("tokens".to_string()),
                resets_at,
            }],
            credits: Some(CreditsInfo {
                balance: remaining.max(0.0),
                currency: "tokens".to_string(),
                total_granted: (total > 0.0).then_some(total),
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn cookie_header(raw: &str) -> String {
    raw.strip_prefix("Cookie:")
        .or_else(|| raw.strip_prefix("cookie:"))
        .unwrap_or(raw)
        .trim()
        .to_string()
}

fn find_number(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    match value {
        serde_json::Value::Object(map) => keys
            .iter()
            .find_map(|key| map.get(*key).and_then(json_number))
            .or_else(|| map.values().find_map(|value| find_number(value, keys))),
        serde_json::Value::Array(values) => {
            values.iter().find_map(|value| find_number(value, keys))
        }
        _ => None,
    }
}

fn find_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => keys
            .iter()
            .find_map(|key| {
                map.get(*key)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .or_else(|| map.values().find_map(|value| find_string(value, keys))),
        serde_json::Value::Array(values) => {
            values.iter().find_map(|value| find_string(value, keys))
        }
        _ => None,
    }
}

fn json_number(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

fn parse_reset(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|dt| Utc.from_utc_datetime(&dt))
        })
}
