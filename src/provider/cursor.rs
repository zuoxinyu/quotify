use anyhow::{Context, Result};
use chrono::Utc;
use regex::Regex;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str = "https://www.cursor.com/settings";

pub struct CursorProvider {
    cookie: String,
    base_url: String,
    client: reqwest::Client,
}

impl CursorProvider {
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
        std::env::var("CURSOR_COOKIE")
            .or_else(|_| std::env::var("CURSOR_SESSION_COOKIE"))
            .ok()
            .filter(|cookie| !cookie.trim().is_empty())
            .map(|cookie| cookie_header(cookie.trim()))
    }

    fn endpoint(&self) -> String {
        if !self.base_url.trim().is_empty() {
            self.base_url.trim().to_string()
        } else {
            std::env::var("CURSOR_SETTINGS_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_URL.to_string())
        }
    }
}

#[async_trait::async_trait]
impl Provider for CursorProvider {
    fn name(&self) -> &str {
        "cursor"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let cookie = self.resolve_cookie().context(
            "Cursor cookie not configured. Set api_key, CURSOR_COOKIE, or CURSOR_SESSION_COOKIE",
        )?;
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(&cookie)?);

        let resp = self
            .client
            .get(self.endpoint())
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to Cursor settings page")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Cursor settings page error: {status} - {body}");
        }
        let text = resp
            .text()
            .await
            .context("Failed to read Cursor settings page")?;
        let windows = parse_usage(&text);
        if windows.is_empty() {
            anyhow::bail!("Cursor settings page did not contain parseable usage data");
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

fn parse_usage(html: &str) -> Vec<UsageWindow> {
    let text = strip_html(html);
    let mut windows = Vec::new();
    for (label, pattern) in [
        (
            "Fast Requests",
            r"(?i)fast requests?.{0,80}?(\d+(?:\.\d+)?)\s*/\s*(\d+(?:\.\d+)?)",
        ),
        (
            "Usage",
            r"(?i)(?:usage|requests).{0,80}?(\d+(?:\.\d+)?)\s*/\s*(\d+(?:\.\d+)?)",
        ),
    ] {
        let re = Regex::new(pattern).unwrap();
        if let Some(caps) = re.captures(&text) {
            let used = caps[1].parse::<f64>().unwrap_or(0.0);
            let limit = caps[2].parse::<f64>().unwrap_or(0.0);
            windows.push(UsageWindow {
                label: label.to_string(),
                used_percent: if limit > 0.0 {
                    (used / limit * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                },
                limit: Some(limit),
                used: Some(used),
                unit: Some("requests".to_string()),
                resets_at: None,
            });
            break;
        }
    }
    windows
}

fn strip_html(html: &str) -> String {
    let tags = Regex::new(r"<[^>]+>").unwrap();
    tags.replace_all(html, " ")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn cookie_header(raw: &str) -> String {
    raw.strip_prefix("Cookie:")
        .or_else(|| raw.strip_prefix("cookie:"))
        .unwrap_or(raw)
        .trim()
        .to_string()
}
