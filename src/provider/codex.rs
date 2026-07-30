use anyhow::{Context, Result};
use base64::Engine as _;
use chrono::Utc;
use reqwest::header::HeaderValue;
use serde::Deserialize;

use super::{Provider, UsageData, UsageWindow, http_client};

pub const RESET_CREDITS_LABEL: &str = "Reset Credits";
const RESET_CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CHATGPT_ACCOUNT_ID_HEADER: &str = "ChatGPT-Account-ID";
const CODEX_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";
const OPENAI_AUTH_CLAIM: &str = "https://api.openai.com/auth";

pub fn is_reset_credits_window(window: &UsageWindow) -> bool {
    window.label == RESET_CREDITS_LABEL
}

pub fn reset_credits(data: &UsageData) -> Option<super::CodexResetCredits> {
    if !data.provider.eq_ignore_ascii_case("codex") {
        return None;
    }

    data.windows
        .iter()
        .find(|window| is_reset_credits_window(window))
        .and_then(|window| window.unit.as_deref())
        .and_then(|json| serde_json::from_str(json).ok())
}

pub struct CodexProvider {
    auth_file: Option<String>,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct CodexAuthFile {
    #[serde(default)]
    tokens: Option<CodexTokens>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Deserialize)]
struct CodexTokens {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    refresh_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

struct CodexAuthContext {
    access_token: String,
    id_token: Option<String>,
    account_id: Option<String>,
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn jwt_auth_claim(jwt: &str, key: &str) -> Option<String> {
    let mut parts = jwt.split('.');
    let header = parts.next()?;
    let payload = parts.next()?;
    let signature = parts.next()?;
    if header.is_empty() || payload.is_empty() || signature.is_empty() || parts.next().is_some() {
        return None;
    }

    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    claims
        .get(OPENAI_AUTH_CLAIM)?
        .get(key)?
        .as_str()
        .map(str::to_string)
        .and_then(non_empty)
}

fn normalize_subscription_tier(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("unknown") {
        return None;
    }

    let display = match value.to_ascii_lowercase().as_str() {
        "free" => "Free",
        "go" => "Go",
        "plus" => "Plus",
        "pro" => "Pro",
        "prolite" | "pro_lite" => "Pro Lite",
        "team" => "Team",
        "self_serve_business_prolite" | "self_serve_business_pro_lite" => {
            "Self Serve Business Pro Lite"
        }
        "self_serve_business_usage_based" => "Self Serve Business Usage Based",
        "business" => "Business",
        "ent26" => "Enterprise",
        "enterprise_cbp_usage_based" => "Enterprise CBP Usage Based",
        "enterprise" | "hc" => "Enterprise",
        "education" | "edu" => "Edu",
        "guest" => "Guest",
        "free_workspace" => "Free Workspace",
        "quorum" => "Quorum",
        "k12" => "K-12",
        _ => return Some(value.to_string()),
    };
    Some(display.to_string())
}

fn subscription_tier_from_jwt(jwt: &str) -> Option<String> {
    jwt_auth_claim(jwt, "chatgpt_plan_type").and_then(|tier| normalize_subscription_tier(&tier))
}

fn subscription_tier_from_usage(usage: &serde_json::Value) -> Option<String> {
    usage
        .get("plan_type")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_subscription_tier)
}

fn resolve_subscription_tier(
    usage: Option<&serde_json::Value>,
    id_token: Option<&str>,
    access_token: Option<&str>,
) -> Option<String> {
    usage
        .and_then(subscription_tier_from_usage)
        .or_else(|| id_token.and_then(subscription_tier_from_jwt))
        .or_else(|| access_token.and_then(subscription_tier_from_jwt))
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct CodexUsageResponse {
    usage: Option<CodexUsageData>,
}

#[allow(dead_code)]
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

    fn read_auth_context(&self) -> Result<CodexAuthContext> {
        let path = self
            .resolve_auth_file()
            .context("Cannot find Codex auth file")?;

        let content =
            std::fs::read_to_string(&path).with_context(|| format!("Failed to read {:?}", path))?;

        let auth: CodexAuthFile =
            serde_json::from_str(&content).context("Failed to parse Codex auth JSON")?;

        let access_token = auth
            .tokens
            .as_ref()
            .and_then(|t| t.access_token.clone())
            .or(auth.access_token)
            .and_then(non_empty)
            .context("No access_token found in Codex auth file")?;
        let id_token = auth
            .tokens
            .as_ref()
            .and_then(|tokens| tokens.id_token.clone())
            .or(auth.id_token)
            .and_then(non_empty);
        let account_id = auth
            .tokens
            .as_ref()
            .and_then(|tokens| tokens.account_id.clone())
            .or(auth.account_id)
            .and_then(non_empty)
            .or_else(|| {
                id_token
                    .as_deref()
                    .and_then(|token| jwt_auth_claim(token, "chatgpt_account_id"))
            })
            .or_else(|| jwt_auth_claim(&access_token, "chatgpt_account_id"));

        Ok(CodexAuthContext {
            access_token,
            id_token,
            account_id,
        })
    }

    fn authenticated_get(&self, url: &str, auth: &CodexAuthContext) -> reqwest::RequestBuilder {
        let request = self
            .client
            .get(url)
            .bearer_auth(&auth.access_token)
            .header("User-Agent", CODEX_USER_AGENT);

        match auth.account_id.as_deref() {
            Some(account_id) if HeaderValue::from_str(account_id).is_ok() => {
                request.header(CHATGPT_ACCOUNT_ID_HEADER, account_id)
            }
            Some(_) => {
                tracing::warn!("Ignoring invalid Codex account id from auth file");
                request
            }
            None => request,
        }
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
        let auth = match self.read_auth_context() {
            Ok(auth) => Some(auth),
            Err(e) => {
                tracing::debug!("Could not read Codex auth token: {e}, trying local data only");
                None
            }
        };

        let mut windows = Vec::new();
        let mut credits = None;
        let mut codex_reset_credits = None;
        let mut subscription_tier = auth.as_ref().and_then(|auth| {
            resolve_subscription_tier(
                None,
                auth.id_token.as_deref(),
                Some(auth.access_token.as_str()),
            )
        });

        if let Some(auth) = auth.as_ref() {
            let reset_resp = self.authenticated_get(RESET_CREDITS_URL, auth).send().await;

            if let Ok(r) = reset_resp
                && r.status().is_success()
                && let Ok(parsed) = r.json::<super::CodexResetCredits>().await
            {
                codex_reset_credits = Some(parsed);
            }

            let resp = self.authenticated_get(USAGE_URL, auth).send().await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    if let Ok(usage) = r.json::<serde_json::Value>().await {
                        subscription_tier = resolve_subscription_tier(
                            Some(&usage),
                            auth.id_token.as_deref(),
                            Some(auth.access_token.as_str()),
                        );

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

                        tracing::debug!(
                            window_count = windows.len(),
                            has_subscription_tier = subscription_tier.is_some(),
                            has_credits = credits.is_some(),
                            "Parsed Codex API usage response"
                        );
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

        if let Some(resets) = codex_reset_credits
            && let Ok(json_str) = serde_json::to_string(&resets)
        {
            windows.push(UsageWindow {
                label: RESET_CREDITS_LABEL.to_string(),
                used_percent: 0.0,
                limit: None,
                used: None,
                unit: Some(json_str),
                resets_at: None,
            });
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
            subscription_tier,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use reqwest::header::AUTHORIZATION;
    use serde_json::json;

    use super::{
        CHATGPT_ACCOUNT_ID_HEADER, CODEX_USER_AGENT, CodexAuthContext, CodexProvider,
        OPENAI_AUTH_CLAIM, RESET_CREDITS_URL, USAGE_URL, jwt_auth_claim, resolve_subscription_tier,
    };

    fn fake_jwt(plan_type: Option<&str>, account_id: Option<&str>) -> String {
        let mut auth_claims = serde_json::Map::new();
        if let Some(plan_type) = plan_type {
            auth_claims.insert(
                "chatgpt_plan_type".to_string(),
                serde_json::Value::String(plan_type.to_string()),
            );
        }
        if let Some(account_id) = account_id {
            auth_claims.insert(
                "chatgpt_account_id".to_string(),
                serde_json::Value::String(account_id.to_string()),
            );
        }

        let mut payload = serde_json::Map::new();
        payload.insert(
            OPENAI_AUTH_CLAIM.to_string(),
            serde_json::Value::Object(auth_claims),
        );

        let encoder = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = encoder.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = encoder.encode(serde_json::to_vec(&payload).unwrap());
        let signature = encoder.encode(b"synthetic-signature");
        format!("{header}.{payload}.{signature}")
    }

    #[test]
    fn usage_plan_type_has_priority_over_jwt_claims() {
        let usage = json!({ "plan_type": "pro" });
        let id_token = fake_jwt(Some("plus"), None);
        let access_token = fake_jwt(Some("free"), None);

        assert_eq!(
            resolve_subscription_tier(Some(&usage), Some(&id_token), Some(&access_token)),
            Some("Pro".to_string())
        );
    }

    #[test]
    fn id_token_plan_type_is_the_first_jwt_fallback() {
        let id_token = fake_jwt(Some("business"), None);
        let access_token = fake_jwt(Some("plus"), None);

        assert_eq!(
            resolve_subscription_tier(None, Some(&id_token), Some(&access_token)),
            Some("Business".to_string())
        );
    }

    #[test]
    fn access_token_plan_type_is_used_when_id_token_has_no_plan() {
        let id_token = fake_jwt(None, Some("workspace-test"));
        let access_token = fake_jwt(Some("go"), None);

        assert_eq!(
            resolve_subscription_tier(None, Some(&id_token), Some(&access_token)),
            Some("Go".to_string())
        );
    }

    #[test]
    fn malformed_jwt_is_ignored_without_panicking() {
        for token in [
            "",
            "header.payload",
            "header.payload.signature.extra",
            "header.not-base64.signature",
        ] {
            assert_eq!(resolve_subscription_tier(None, Some(token), None), None);
        }
    }

    #[test]
    fn future_plan_type_is_preserved_and_unknown_uses_jwt_fallback() {
        let future = json!({ "plan_type": "future_ultra" });
        assert_eq!(
            resolve_subscription_tier(Some(&future), None, None),
            Some("future_ultra".to_string())
        );

        let unknown = json!({ "plan_type": "unknown" });
        let id_token = fake_jwt(Some("plus"), None);
        assert_eq!(
            resolve_subscription_tier(Some(&unknown), Some(&id_token), None),
            Some("Plus".to_string())
        );
    }

    #[test]
    fn known_plan_types_are_normalized_for_display() {
        for (raw, display) in [
            ("free", "Free"),
            ("go", "Go"),
            ("plus", "Plus"),
            ("pro", "Pro"),
            ("prolite", "Pro Lite"),
            ("team", "Team"),
            ("business", "Business"),
            ("enterprise", "Enterprise"),
            ("edu", "Edu"),
        ] {
            let usage = json!({ "plan_type": raw });
            assert_eq!(
                resolve_subscription_tier(Some(&usage), None, None).as_deref(),
                Some(display)
            );
        }
    }

    #[test]
    fn jwt_account_id_claim_is_decoded_without_exposing_the_token() {
        let token = fake_jwt(None, Some("workspace-test"));
        assert_eq!(
            jwt_auth_claim(&token, "chatgpt_account_id"),
            Some("workspace-test".to_string())
        );
    }

    #[test]
    fn both_codex_requests_include_workspace_auth_headers() {
        let provider = CodexProvider::new(None, None);
        let auth = CodexAuthContext {
            access_token: "synthetic-access-token".to_string(),
            id_token: None,
            account_id: Some("workspace-test".to_string()),
        };

        for url in [RESET_CREDITS_URL, USAGE_URL] {
            let request = provider
                .authenticated_get(url, &auth)
                .build()
                .expect("synthetic request should build");
            assert_eq!(request.url().as_str(), url);
            assert_eq!(
                request
                    .headers()
                    .get(AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer synthetic-access-token")
            );
            assert_eq!(
                request
                    .headers()
                    .get(CHATGPT_ACCOUNT_ID_HEADER)
                    .and_then(|value| value.to_str().ok()),
                Some("workspace-test")
            );
            assert_eq!(
                request
                    .headers()
                    .get("User-Agent")
                    .and_then(|value| value.to_str().ok()),
                Some(CODEX_USER_AGENT)
            );
        }
    }

    #[test]
    fn test_parse_codex_reset_credits() {
        let raw_json = json!({
            "available_count": 2,
            "credits": [
                {
                    "status": "available",
                    "granted_at": "2026-06-17T17:38:38Z",
                    "expires_at": "2026-07-17T17:38:38Z"
                },
                {
                    "status": "redeemed",
                    "granted_at": "2026-06-18T17:38:38Z",
                    "expires_at": null
                }
            ]
        });

        let parsed: crate::provider::CodexResetCredits = serde_json::from_value(raw_json).unwrap();
        assert_eq!(parsed.available_count, 2);
        assert_eq!(parsed.credits.len(), 2);
        assert_eq!(parsed.credits[0].status, "available");
        assert!(parsed.credits[0].expires_at.is_some());
        assert_eq!(parsed.credits[1].status, "redeemed");
        assert!(parsed.credits[1].expires_at.is_none());
    }

    #[test]
    fn reset_credits_are_decoded_from_codex_usage_metadata() {
        let resets = crate::provider::CodexResetCredits {
            available_count: 1,
            credits: vec![crate::provider::CodexResetCredit {
                status: "available".to_string(),
                granted_at: None,
                expires_at: chrono::DateTime::parse_from_rfc3339("2026-07-17T17:38:38Z")
                    .ok()
                    .map(|date| date.to_utc()),
            }],
        };
        let data = crate::provider::UsageData {
            provider: "codex".to_string(),
            windows: vec![crate::provider::UsageWindow {
                label: super::RESET_CREDITS_LABEL.to_string(),
                used_percent: 0.0,
                limit: None,
                used: None,
                unit: Some(serde_json::to_string(&resets).unwrap()),
                resets_at: None,
            }],
            credits: None,
            subscription_tier: None,
            fetched_at: chrono::Utc::now(),
            error: None,
        };

        let decoded = super::reset_credits(&data).unwrap();
        assert_eq!(decoded.available_count, 1);
        assert_eq!(decoded.credits.len(), 1);
        assert_eq!(decoded.credits[0].status, "available");
    }
}
