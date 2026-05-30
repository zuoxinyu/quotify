use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{AUTHORIZATION, HeaderMap};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://api.groq.com/v1/metrics/prometheus";

pub struct GroqCloudProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl GroqCloudProvider {
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
        std::env::var("GROQ_API_KEY")
            .or_else(|_| std::env::var("GROQCLOUD_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("GROQ_METRICS_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl Provider for GroqCloudProvider {
    fn name(&self) -> &str {
        "groqcloud"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("GroqCloud API key not configured. Set api_key or GROQ_API_KEY")?;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);

        let requests = self
            .query_metric(
                &headers,
                "sum(model_project_id_status_code:requests:rate5m)",
            )
            .await?;
        let tokens_in = self
            .query_metric(&headers, "sum(model_project_id:tokens_in:rate5m)")
            .await
            .unwrap_or(0.0);
        let tokens_out = self
            .query_metric(&headers, "sum(model_project_id:tokens_out:rate5m)")
            .await
            .unwrap_or(0.0);

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![
                UsageWindow {
                    label: "Requests 5m".to_string(),
                    used_percent: 0.0,
                    limit: None,
                    used: Some(requests),
                    unit: Some("req/s".to_string()),
                    resets_at: None,
                },
                UsageWindow {
                    label: "Input Tokens".to_string(),
                    used_percent: 0.0,
                    limit: None,
                    used: Some(tokens_in),
                    unit: Some("tok/s".to_string()),
                    resets_at: None,
                },
                UsageWindow {
                    label: "Output Tokens".to_string(),
                    used_percent: 0.0,
                    limit: None,
                    used: Some(tokens_out),
                    unit: Some("tok/s".to_string()),
                    resets_at: None,
                },
            ],
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

impl GroqCloudProvider {
    async fn query_metric(&self, headers: &HeaderMap, query: &str) -> Result<f64> {
        let resp = self
            .client
            .get(format!("{}/api/v1/query", self.base_url()))
            .headers(headers.clone())
            .query(&[("query", query)])
            .send()
            .await
            .with_context(|| format!("Failed to query GroqCloud metric: {query}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("GroqCloud metrics API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse GroqCloud metrics response")?;
        Ok(sum_prometheus_values(&json))
    }
}

fn sum_prometheus_values(value: &serde_json::Value) -> f64 {
    match value {
        serde_json::Value::Array(items) => {
            if items.len() == 2
                && let Some(raw) = items.get(1).and_then(|v| v.as_str())
            {
                return raw.parse().unwrap_or(0.0);
            }
            items.iter().map(sum_prometheus_values).sum()
        }
        serde_json::Value::Object(map) => map.values().map(sum_prometheus_values).sum(),
        _ => 0.0,
    }
}
