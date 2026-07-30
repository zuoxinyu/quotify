use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue};

use super::{Provider, UsageData, UsageWindow, http_client};

const DEFAULT_URL: &str = "https://cursor.com";

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

    fn base_url(&self) -> String {
        self.endpoint()
            .trim_end_matches('/')
            .trim_end_matches("/settings")
            .trim_end_matches("/api/usage-summary")
            .to_string()
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

        let summary_response = self
            .client
            .get(format!("{}/api/usage-summary", self.base_url()))
            .headers(headers.clone())
            .send()
            .await;

        if let Ok(response) = summary_response
            && response.status().is_success()
        {
            let json: serde_json::Value = response
                .json()
                .await
                .context("Failed to parse Cursor usage summary")?;
            let (windows, subscription_tier) = parse_usage_summary(&json);
            if !windows.is_empty() {
                return Ok(UsageData {
                    provider: self.name().to_string(),
                    windows,
                    credits: None,
                    subscription_tier,
                    fetched_at: Utc::now(),
                    error: None,
                });
            }
        }

        let settings_url = if self.endpoint().contains("/settings") {
            self.endpoint()
        } else {
            format!("{}/settings", self.base_url())
        };
        let response = self
            .client
            .get(settings_url)
            .headers(headers)
            .send()
            .await
            .context("Failed to connect to Cursor settings page")?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Cursor settings page error: {status} - {body}");
        }
        let text = response
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
            subscription_tier: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn parse_usage_summary(json: &serde_json::Value) -> (Vec<UsageWindow>, Option<String>) {
    let reset = json
        .get("billingCycleEnd")
        .and_then(|value| value.as_str())
        .and_then(parse_date);
    let tier = json
        .get("membershipType")
        .and_then(|value| value.as_str())
        .map(format_membership_type);
    let plan = json.pointer("/individualUsage/plan");
    let usage_source = plan
        .or_else(|| json.pointer("/individualUsage/overall"))
        .or_else(|| json.pointer("/teamUsage/pooled"));
    let mut windows = Vec::new();

    if let Some(source) = usage_source {
        let used_cents = number(source.get("used"));
        let limit_cents = number(source.get("limit"));
        let auto_percent = plan.and_then(|plan| number(plan.get("autoPercentUsed")));
        let api_percent = plan.and_then(|plan| number(plan.get("apiPercentUsed")));
        let total_percent = plan
            .and_then(|plan| number(plan.get("totalPercentUsed")))
            .or_else(|| match (auto_percent, api_percent) {
                (Some(auto), Some(api)) => Some((auto + api) / 2.0),
                (Some(auto), None) => Some(auto),
                (None, Some(api)) => Some(api),
                _ => None,
            })
            .or_else(|| match (used_cents, limit_cents) {
                (Some(used), Some(limit)) if limit > 0.0 => Some(used / limit * 100.0),
                _ => None,
            });
        if let Some(percent) = total_percent {
            windows.push(UsageWindow {
                label: "Billing Cycle".to_string(),
                used_percent: percent.clamp(0.0, 100.0),
                limit: limit_cents.map(|value| value / 100.0),
                used: used_cents.map(|value| value / 100.0),
                unit: Some("USD".to_string()),
                resets_at: reset,
            });
        }

        for (percent, label) in [(auto_percent, "Auto"), (api_percent, "API Models")] {
            if let Some(percent) = percent {
                windows.push(UsageWindow {
                    label: label.to_string(),
                    used_percent: percent.clamp(0.0, 100.0),
                    limit: Some(100.0),
                    used: Some(percent),
                    unit: Some("%".to_string()),
                    resets_at: reset,
                });
            }
        }
    }

    if let Some(on_demand) = json
        .pointer("/individualUsage/onDemand")
        .or_else(|| json.pointer("/teamUsage/onDemand"))
        && on_demand
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    {
        let used = number(on_demand.get("used")).map(|value| value / 100.0);
        let limit = number(on_demand.get("limit")).map(|value| value / 100.0);
        if let Some(used) = used {
            windows.push(UsageWindow {
                label: "On-Demand".to_string(),
                used_percent: limit
                    .filter(|limit| *limit > 0.0)
                    .map(|limit| used / limit * 100.0)
                    .unwrap_or(0.0)
                    .clamp(0.0, 100.0),
                limit,
                used: Some(used),
                unit: Some("USD".to_string()),
                resets_at: reset,
            });
        }
    }

    (windows, tier)
}

fn number(value: Option<&serde_json::Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str()?.parse::<f64>().ok())
    })
}

fn parse_date(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn format_membership_type(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cursor_billing_lanes_and_tier() {
        let value = serde_json::json!({
            "billingCycleEnd": "2030-01-02T03:04:05Z",
            "membershipType": "pro_plus",
            "individualUsage": {
                "plan": {
                    "used": 2500,
                    "limit": 10000,
                    "totalPercentUsed": 25,
                    "autoPercentUsed": 12.5,
                    "apiPercentUsed": 37.5
                }
            }
        });
        let (windows, tier) = parse_usage_summary(&value);
        assert_eq!(tier.as_deref(), Some("Pro Plus"));
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].used, Some(25.0));
        assert_eq!(windows[0].limit, Some(100.0));
        assert!(windows.iter().all(|window| window.resets_at.is_some()));
    }

    #[test]
    fn uses_team_pool_and_team_on_demand_when_individual_usage_is_absent() {
        let value = serde_json::json!({
            "membershipType": "enterprise",
            "teamUsage": {
                "pooled": { "used": 4000, "limit": 20000 },
                "onDemand": { "enabled": true, "used": 750, "limit": 5000 }
            }
        });
        let (windows, tier) = parse_usage_summary(&value);
        assert_eq!(tier.as_deref(), Some("Enterprise"));
        assert!(windows.iter().any(|window| {
            window.label == "Billing Cycle"
                && window.used_percent == 20.0
                && window.used == Some(40.0)
        }));
        assert!(windows.iter().any(|window| {
            window.label == "On-Demand" && window.used == Some(7.5) && window.limit == Some(50.0)
        }));
    }
}
