use anyhow::{Context, Result};
use chrono::Utc;
use regex::Regex;
use reqwest::header::{AUTHORIZATION, COOKIE, HeaderMap, USER_AGENT};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_BASE_URL: &str = "https://ollama.com";

pub struct OllamaProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(api_key: String, base_url: String, proxy: Option<&str>) -> Self {
        Self {
            api_key,
            base_url,
            client: http_client(proxy),
        }
    }

    fn resolve_api_key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() && !self.api_key.contains('=') && !self.api_key.contains(';') {
            return Some(self.api_key.trim().to_string());
        }
        std::env::var("OLLAMA_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
    }

    fn resolve_cookie(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() && (self.api_key.contains('=') || self.api_key.contains(';')) {
            return Some(cookie_header(self.api_key.trim()));
        }
        std::env::var("OLLAMA_COOKIE")
            .or_else(|_| std::env::var("OLLAMA_SESSION_COOKIE"))
            .ok()
            .filter(|c| !c.trim().is_empty())
            .map(|c| cookie_header(c.trim()))
            .or_else(|| {
                crate::secrets::get("ollama", "auth_cookie")
                    .ok()
                    .flatten()
                    .filter(|c| !c.trim().is_empty())
                    .map(|c| cookie_header(c.trim()))
            })
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().trim_end_matches('/').to_string()
        } else {
            std::env::var("OLLAMA_API_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string()
        }
    }

    fn is_local(&self) -> bool {
        let base = self.base_url();
        base.contains("localhost") || base.contains("127.0.0.1")
    }

    async fn fetch_api_usage(&self) -> Result<UsageData> {
        let mut headers = HeaderMap::new();
        if let Some(api_key) = self.resolve_api_key() {
            headers.insert(AUTHORIZATION, format!("Bearer {api_key}").parse()?);
        } else if !self.is_local() {
            anyhow::bail!("Ollama API key not configured. Set api_key or OLLAMA_API_KEY");
        }

        let resp = self
            .client
            .get(format!("{}/api/tags", self.base_url()))
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to Ollama API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error: {status} - {body}");
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Ollama models response")?;
        let model_count = json
            .get("models")
            .or_else(|| json.get("tags"))
            .and_then(|v| v.as_array())
            .map(|models| models.len() as f64);

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Cloud API".to_string(),
                used_percent: 0.0,
                limit: None,
                used: model_count,
                unit: Some("models".to_string()),
                resets_at: None,
            }],
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }

    async fn fetch_settings_usage(&self, cookie: &str) -> Result<UsageData> {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, reqwest::header::HeaderValue::from_str(cookie)?);
        headers.insert(
            USER_AGENT,
            reqwest::header::HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
            )
        );

        let url = format!("{}/settings", self.base_url());
        let resp = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to Ollama settings page")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama settings page error: {status} - {body}");
        }

        let html = resp
            .text()
            .await
            .context("Failed to read Ollama settings HTML")?;

        let windows = parse_settings_html(&html);
        if windows.is_empty() {
            anyhow::bail!("Ollama settings page did not contain parseable usage data");
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

#[async_trait::async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        if self.is_local() {
            return self.fetch_api_usage().await;
        }

        let mut cookie = self.resolve_cookie();

        // If we don't have a cookie and also don't have a standard API key configured, trigger login
        if cookie.is_none() && self.resolve_api_key().is_none() {
            tracing::info!("Ollama: No credentials found, launching WebView login...");
            match tokio::task::spawn_blocking(crate::webview_login::ollama_login_and_get_cookie).await? {
                Ok(fresh_cookie) => {
                    if let Err(err) = crate::secrets::set("ollama", "auth_cookie", &fresh_cookie) {
                        tracing::error!("Failed to store Ollama cookie in Windows Credential Manager: {err}");
                    }
                    cookie = Some(fresh_cookie);
                }
                Err(err) => {
                    anyhow::bail!("Ollama WebView login failed: {err}");
                }
            }
        }

        if let Some(cookie_str) = cookie {
            match self.fetch_settings_usage(&cookie_str).await {
                Ok(data) => return Ok(data),
                Err(err) => {
                    tracing::warn!("Ollama settings page fetch failed: {err}. Retrying with WebView login...");
                    match tokio::task::spawn_blocking(crate::webview_login::ollama_login_and_get_cookie).await? {
                        Ok(fresh_cookie) => {
                            if let Err(err) = crate::secrets::set("ollama", "auth_cookie", &fresh_cookie) {
                                tracing::error!("Failed to store Ollama cookie in Windows Credential Manager: {err}");
                            }
                            match self.fetch_settings_usage(&fresh_cookie).await {
                                Ok(data) => return Ok(data),
                                Err(err2) => {
                                    tracing::warn!("Ollama settings page fetch failed again with fresh cookie: {err2}");
                                    if self.resolve_api_key().is_some() {
                                        return self.fetch_api_usage().await;
                                    } else {
                                        anyhow::bail!("Failed to fetch Ollama settings usage: {err2}");
                                    }
                                }
                            }
                        }
                        Err(login_err) => {
                            tracing::warn!("Ollama WebView login retry failed: {login_err}");
                            if self.resolve_api_key().is_some() {
                                return self.fetch_api_usage().await;
                            } else {
                                anyhow::bail!("Ollama settings page fetch failed: {err} and login failed: {login_err}");
                            }
                        }
                    }
                }
            }
        }

        self.fetch_api_usage().await
    }
}

fn cookie_header(raw: &str) -> String {
    let raw = raw
        .strip_prefix("Cookie:")
        .or_else(|| raw.strip_prefix("cookie:"))
        .unwrap_or(raw)
        .trim();
    if raw.contains('=') {
        raw.to_string()
    } else {
        format!("__Host-next-auth.session-token={}; __Secure-session={}; next-auth.session-token={}", raw, raw, raw)
    }
}

fn parse_settings_html(html: &str) -> Vec<UsageWindow> {
    let mut windows = Vec::new();
    let html_lower = html.to_lowercase();

    // 1. Session Limit
    if let Some(window) = find_window_for_label("Session Limit", "session", &html_lower, html) {
        windows.push(window);
    }

    // 2. Weekly Limit
    if let Some(window) = find_window_for_label("Weekly Limit", "weekly", &html_lower, html) {
        windows.push(window);
    }

    windows
}

fn find_window_for_label(label: &str, keyword: &str, html_lower: &str, html: &str) -> Option<UsageWindow> {
    let other_keyword = if keyword == "session" { "weekly" } else { "session" };

    let mut start = 0;
    while let Some(idx) = html_lower[start..].find(keyword) {
        let absolute_idx = start + idx;

        // Determine end of block: max 1000 chars, or truncated at other_keyword
        let mut end_idx = std::cmp::min(html.len(), absolute_idx + 1000);
        if let Some(other_idx) = html_lower[absolute_idx..end_idx].find(other_keyword) {
            end_idx = absolute_idx + other_idx;
        }

        let block = &html[absolute_idx..end_idx];
        if let Some(window) = parse_usage_block(label, block) {
            return Some(window);
        }

        start = absolute_idx + keyword.len();
    }
    None
}

fn parse_usage_block(label: &str, block: &str) -> Option<UsageWindow> {
    let re_style_width = Regex::new(r"width:\s*(\d+(?:\.\d+)?)%").unwrap();
    let re_text_pct = Regex::new(r"(\d+(?:\.\d+)?)%").unwrap();

    let used_percent = if let Some(caps) = re_style_width.captures(block) {
        caps[1].parse::<f64>().unwrap_or(0.0)
    } else if let Some(caps) = re_text_pct.captures(block) {
        caps[1].parse::<f64>().unwrap_or(0.0)
    } else {
        return None;
    };

    let re_iso_time = Regex::new(r#"(?:data-time|datetime)="([^"]+)""#).unwrap();
    let re_standalone_iso = Regex::new(r"\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})\b").unwrap();

    let resets_at = if let Some(caps) = re_iso_time.captures(block) {
        chrono::DateTime::parse_from_rfc3339(caps[1].trim())
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
    } else if let Some(caps) = re_standalone_iso.captures(block) {
        chrono::DateTime::parse_from_rfc3339(caps[0].trim())
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
    } else {
        None
    };

    Some(UsageWindow {
        label: label.to_string(),
        used_percent,
        limit: None,
        used: None,
        unit: Some("%".to_string()),
        resets_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cookie_header() {
        assert_eq!(cookie_header("foo=bar"), "foo=bar");
        assert_eq!(
            cookie_header("xyz"),
            "__Host-next-auth.session-token=xyz; __Secure-session=xyz; next-auth.session-token=xyz"
        );
    }

    #[test]
    fn test_parse_settings_html() {
        let html = r#"
            <div>
                <h2>Cloud Usage</h2>
                <div class="card">
                    <h3>Session usage</h3>
                    <div style="width: 25.5%"></div>
                    <span data-time="2026-07-07T22:00:00Z">Resets in 1 hour</span>
                </div>
                <div class="card">
                    <h3>Weekly usage</h3>
                    <div>Current used: 40%</div>
                    <span datetime="2026-07-14T12:00:00+08:00">Resets in 7 days</span>
                </div>
            </div>
        "#;

        let windows = parse_settings_html(html);
        assert_eq!(windows.len(), 2);

        assert_eq!(windows[0].label, "Session Limit");
        assert_eq!(windows[0].used_percent, 25.5);
        assert!(windows[0].resets_at.is_some());

        assert_eq!(windows[1].label, "Weekly Limit");
        assert_eq!(windows[1].used_percent, 40.0);
        assert!(windows[1].resets_at.is_some());
    }

    #[test]
    fn test_parse_settings_html_different_values() {
        let html = r#"
            <div>
                <h2>Cloud Usage</h2>
                <div class="card">
                    <h3>Session usage</h3>
                    <div style="width: 5.0%"></div>
                    <span data-time="2026-07-07T22:00:00Z">Resets in 1 hour</span>
                </div>
                <div class="card">
                    <h3>Weekly usage</h3>
                    <div style="width: 45.0%"></div>
                    <span data-time="2026-07-14T22:00:00Z">Resets in 7 days</span>
                </div>
            </div>
        "#;

        let windows = parse_settings_html(html);
        assert_eq!(windows.len(), 2);

        assert_eq!(windows[0].label, "Session Limit");
        assert_eq!(windows[0].used_percent, 5.0);

        assert_eq!(windows[1].label, "Weekly Limit");
        assert_eq!(windows[1].used_percent, 45.0);
    }
}
