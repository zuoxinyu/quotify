use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;

use super::{Provider, UsageData, UsageWindow, http_client};

pub struct CodexProvider {
    auth_file: Option<String>,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct CodexAuthFile {
    #[serde(default)]
    tokens: Option<CodexTokens>,
    #[serde(default)]
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexTokens {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    #[expect(dead_code)]
    refresh_token: Option<String>,
    #[serde(default)]
    #[expect(dead_code)]
    account_id: Option<String>,
}

#[expect(dead_code)]
#[derive(Debug, Deserialize)]
struct CodexUsageResponse {
    usage: Option<CodexUsageData>,
}

#[expect(dead_code)]
#[derive(Debug, Deserialize)]
struct CodexUsageData {
    rate_limits: Option<CodexRateLimits>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CodexRateLimits {
    #[serde(default)]
    primary: Option<CodexRateWindow>,
    #[serde(default)]
    secondary: Option<CodexRateWindow>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CodexRateWindow {
    used_percent: Option<f64>,
    resets_at: Option<String>,
    limit: Option<f64>,
    used: Option<f64>,
}

impl CodexProvider {
    pub fn new(auth_file: Option<String>, proxy: Option<&str>) -> Self {
        Self {
            auth_file,
            client: http_client(proxy),
        }
    }

    fn default_auth_path() -> Option<std::path::PathBuf> {
        let home = dirs::home_dir()?;
        Some(home.join(".codex").join("auth.json"))
    }

    fn resolve_auth_file(&self) -> Option<std::path::PathBuf> {
        self.auth_file
            .as_ref()
            .map(std::path::PathBuf::from)
            .or_else(Self::default_auth_path)
    }

    fn read_access_token(&self) -> Result<String> {
        let path = self
            .resolve_auth_file()
            .context("Cannot find Codex auth file")?;

        let content =
            std::fs::read_to_string(&path).with_context(|| format!("Failed to read {:?}", path))?;

        let auth: CodexAuthFile =
            serde_json::from_str(&content).context("Failed to parse Codex auth JSON")?;

        auth.tokens
            .as_ref()
            .and_then(|t| t.access_token.clone())
            .or(auth.access_token)
            .filter(|t| !t.is_empty())
            .context("No access_token found in Codex auth file")
    }

    #[allow(dead_code)]
    fn parse_session_files(&self) -> Result<Vec<UsageWindow>> {
        let home = dirs::home_dir().context("Cannot find home directory")?;
        let sessions_dir = home.join(".codex").join("sessions");

        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut windows = Vec::new();
        let mut latest_primary: Option<CodexRateWindow> = None;
        let mut latest_secondary: Option<CodexRateWindow> = None;
        let mut latest_ts: i64 = 0;

        let entries = match std::fs::read_dir(&sessions_dir) {
            Ok(e) => e,
            Err(_) => return Ok(windows),
        };
        for entry in entries.map_while(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            if let Ok(file) = std::fs::File::open(&path) {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(file);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line)
                        && json.get("type").and_then(|v| v.as_str()) == Some("token_count")
                        && let Some(ts) = json.get("timestamp").and_then(|v| v.as_i64())
                        && ts > latest_ts
                    {
                        latest_ts = ts;
                        if let Some(rl) = json.get("rate_limits")
                            && let Ok(parsed) =
                                serde_json::from_value::<CodexRateLimits>(rl.clone())
                        {
                            latest_primary = parsed.primary;
                            latest_secondary = parsed.secondary;
                        }
                    }
                }
            }
        }

        if let Some(p) = latest_primary {
            windows.push(UsageWindow {
                label: "Session".to_string(),
                used_percent: p.used_percent.unwrap_or(0.0),
                limit: p.limit,
                used: p.used,
                unit: None,
                resets_at: p.resets_at.and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .ok()
                        .map(|dt| dt.to_utc())
                }),
            });
        }

        if let Some(s) = latest_secondary {
            windows.push(UsageWindow {
                label: "Weekly".to_string(),
                used_percent: s.used_percent.unwrap_or(0.0),
                limit: s.limit,
                used: s.used,
                unit: None,
                resets_at: s.resets_at.and_then(|s_str| {
                    chrono::DateTime::parse_from_rfc3339(&s_str)
                        .ok()
                        .map(|dt| dt.to_utc())
                }),
            });
        }

        Ok(windows)
    }
}

#[async_trait::async_trait]
impl Provider for CodexProvider {
    fn name(&self) -> &str {
        "codex"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let access_token = match self.read_access_token() {
            Ok(token) => Some(token),
            Err(e) => {
                tracing::debug!("Could not read Codex auth token: {e}, trying local data only");
                None
            }
        };

        let mut windows = Vec::new();

        let mut credits = None;

        if let Some(token) = access_token {
            let resp = self
                .client
                .get("https://chatgpt.com/backend-api/wham/usage")
                .header("Authorization", format!("Bearer {token}"))
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
                )
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    if let Ok(usage) = r.json::<serde_json::Value>().await {
                        tracing::debug!("Codex API raw response: {usage:#?}");

                        // Parse rate_limit structure: primary_window / secondary_window
                        if let Some(rate_limit) = usage.get("rate_limit") {
                            let parse_window = |_key: &str,
                                                label: &str,
                                                obj: &serde_json::Value|
                             -> Option<UsageWindow> {
                                let pct = obj.get("used_percent").and_then(|v| v.as_f64());
                                let limit_seconds =
                                    obj.get("limit_window_seconds").and_then(|v| v.as_f64());
                                let reset_after =
                                    obj.get("reset_after_seconds").and_then(|v| v.as_f64());
                                let reset_at_epoch = obj.get("reset_at").and_then(|v| v.as_f64());

                                if pct.is_none() && limit_seconds.is_none() {
                                    return None;
                                }

                                let resets_at = reset_at_epoch.and_then(|ts| {
                                    chrono::DateTime::from_timestamp(ts as i64, 0)
                                        .map(|dt| dt.to_utc())
                                });

                                Some(UsageWindow {
                                    label: label.to_string(),
                                    used_percent: pct.unwrap_or(0.0),
                                    limit: limit_seconds,
                                    used: reset_after,
                                    unit: Some("seconds".to_string()),
                                    resets_at,
                                })
                            };

                            // primary_window (session/5h)
                            if let Some(pw) = rate_limit.get("primary_window")
                                && let Some(w) = parse_window("primary_window", "Session (5h)", pw)
                            {
                                windows.push(w);
                            }

                            // secondary_window (weekly)
                            if let Some(sw) = rate_limit.get("secondary_window")
                                && let Some(w) = parse_window("secondary_window", "Weekly", sw)
                            {
                                windows.push(w);
                            }
                        }

                        // Parse credits
                        if let Some(c) = usage.get("credits") {
                            let balance = c
                                .get("balance")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .or_else(|| c.get("balance").and_then(|v| v.as_f64()));
                            let has_credits = c.get("has_credits").and_then(|v| v.as_bool());
                            let unlimited = c.get("unlimited").and_then(|v| v.as_bool());

                            if let Some(bal) = balance {
                                credits = Some(super::CreditsInfo {
                                    balance: bal,
                                    currency: "USD".to_string(),
                                    total_granted: None,
                                    topped_up: None,
                                });
                            }

                            let _ = (has_credits, unlimited);
                        }

                        // Fallback: try generic flat structure
                        if windows.is_empty()
                            && let Some(obj) = usage.as_object()
                        {
                            for (key, value) in obj {
                                if key == "rate_limit"
                                    || key == "credits"
                                    || key == "account_id"
                                    || key == "email"
                                    || key == "plan_type"
                                    || key == "user_id"
                                {
                                    continue;
                                }
                                if let Some(inner) = value.as_object() {
                                    let pct = inner
                                        .get("used_percent")
                                        .or_else(|| inner.get("percentage"))
                                        .and_then(|v| v.as_f64());
                                    let limit = inner.get("limit").and_then(|v| v.as_f64());
                                    let used = inner.get("used").and_then(|v| v.as_f64());
                                    let resets_at = inner
                                        .get("resets_at")
                                        .or_else(|| inner.get("reset_at"))
                                        .and_then(|v| v.as_str())
                                        .and_then(|s| {
                                            chrono::DateTime::parse_from_rfc3339(s)
                                                .ok()
                                                .map(|dt| dt.to_utc())
                                        });

                                    if pct.is_some() || used.is_some() || limit.is_some() {
                                        windows.push(UsageWindow {
                                            label: key.clone(),
                                            used_percent: pct.unwrap_or(0.0),
                                            limit,
                                            used,
                                            unit: None,
                                            resets_at,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(r) => {
                    tracing::warn!("Codex API returned non-success status: {}", r.status());
                }
                Err(e) => {
                    tracing::warn!("Failed to call Codex API: {e}");
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
