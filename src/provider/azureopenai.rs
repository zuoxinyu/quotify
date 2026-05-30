use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::json;

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_API_VERSION: &str = "2024-10-21";

pub struct AzureOpenAiProvider {
    api_key: String,
    endpoint: String,
    deployment: String,
    client: reqwest::Client,
}

impl AzureOpenAiProvider {
    pub fn new(api_key: String, endpoint: String, deployment: String, proxy: Option<&str>) -> Self {
        Self {
            api_key,
            endpoint,
            deployment,
            client: http_client(proxy),
        }
    }

    fn resolve_api_key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.trim().to_string());
        }
        std::env::var("AZURE_OPENAI_API_KEY")
            .or_else(|_| std::env::var("AZURE_OPENAI_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn resolve_endpoint(&self) -> Option<String> {
        if !self.endpoint.trim().is_empty() {
            return Some(self.endpoint.trim().trim_end_matches('/').to_string());
        }
        std::env::var("AZURE_OPENAI_ENDPOINT")
            .ok()
            .filter(|endpoint| !endpoint.trim().is_empty())
            .map(|endpoint| endpoint.trim().trim_end_matches('/').to_string())
    }

    fn resolve_deployment(&self) -> Option<String> {
        if !self.deployment.trim().is_empty() {
            return Some(self.deployment.trim().to_string());
        }
        std::env::var("AZURE_OPENAI_DEPLOYMENT_NAME")
            .or_else(|_| std::env::var("AZURE_OPENAI_DEPLOYMENT"))
            .ok()
            .filter(|deployment| !deployment.trim().is_empty())
    }
}

#[async_trait::async_trait]
impl Provider for AzureOpenAiProvider {
    fn name(&self) -> &str {
        "azureopenai"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("Azure OpenAI API key not configured. Set api_key or AZURE_OPENAI_API_KEY")?;
        let endpoint = self.resolve_endpoint().context(
            "Azure OpenAI endpoint not configured. Set base_url or AZURE_OPENAI_ENDPOINT",
        )?;
        let deployment = self.resolve_deployment().context(
            "Azure OpenAI deployment not configured. Set deployment or AZURE_OPENAI_DEPLOYMENT_NAME",
        )?;
        let api_version = std::env::var("AZURE_OPENAI_API_VERSION")
            .unwrap_or_else(|_| DEFAULT_API_VERSION.to_string());
        let url = if api_version == "v1" {
            format!("{endpoint}/openai/v1/chat/completions")
        } else {
            format!(
                "{endpoint}/openai/deployments/{deployment}/chat/completions?api-version={api_version}"
            )
        };

        let mut headers = HeaderMap::new();
        headers.insert("api-key", HeaderValue::from_str(&api_key)?);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let body = if api_version == "v1" {
            json!({
                "model": deployment,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1
            })
        } else {
            json!({
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1
            })
        };

        let resp = self
            .client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .context("Failed to connect to Azure OpenAI deployment")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Azure OpenAI deployment probe error: {status} - {body}");
        }

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Deployment".to_string(),
                used_percent: 0.0,
                limit: None,
                used: None,
                unit: None,
                resets_at: None,
            }],
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}
