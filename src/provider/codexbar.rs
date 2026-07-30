use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, TimeZone, Utc};
use reqwest::{Method, RequestBuilder};
use serde_json::Value;

use super::{CreditsInfo, Provider, UsageData, UsageWindow, http_client};

pub(crate) struct CodexBarProvider {
    id: &'static str,
    api_key: String,
    base_url: String,
    extra: String,
    client: reqwest::Client,
}

impl CodexBarProvider {
    pub(crate) fn new(
        id: &'static str,
        api_key: String,
        base_url: String,
        extra: String,
        proxy: Option<&str>,
    ) -> Self {
        Self {
            id,
            api_key,
            base_url,
            extra,
            client: http_client(proxy),
        }
    }

    pub(crate) fn has_environment_credentials(id: &str) -> bool {
        credential_environment_keys(id).iter().any(|key| {
            std::env::var(key)
                .ok()
                .is_some_and(|value| !value.trim().is_empty())
        }) || (id == "wayfinder"
            && std::env::var("WAYFINDER_GATEWAY_URL")
                .ok()
                .is_some_and(|value| !value.trim().is_empty()))
    }

    fn credential(&self) -> Result<String> {
        if !self.api_key.trim().is_empty() {
            return Ok(self.api_key.trim().to_string());
        }
        for key in credential_environment_keys(self.id) {
            if let Ok(value) = std::env::var(key)
                && !value.trim().is_empty()
            {
                return Ok(value.trim().to_string());
            }
        }
        if self.id == "wayfinder" {
            return Ok(String::new());
        }
        anyhow::bail!(
            "{} credential not configured. Set it in Provider Settings or one of: {}",
            super::display_name(self.id),
            credential_environment_keys(self.id).join(", ")
        )
    }

    fn base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            return self.base_url.trim().trim_end_matches('/').to_string();
        }
        if let Some(key) = base_url_environment_key(self.id)
            && let Ok(value) = std::env::var(key)
            && !value.trim().is_empty()
        {
            return value.trim().trim_end_matches('/').to_string();
        }
        default_base_url(self.id).trim_end_matches('/').to_string()
    }

    async fn fetch_value(&self) -> Result<Value> {
        match self.id {
            "deepinfra" => self.fetch_deepinfra().await,
            "qwencloud" => self.fetch_qwencloud().await,
            "wayfinder" => self.fetch_wayfinder().await,
            "xai" => self.fetch_xai().await,
            "chutes" => self.fetch_chutes().await,
            "zenmux" => self.fetch_zenmux().await,
            "zed" => self.fetch_zed().await,
            _ => {
                let credential = self.credential()?;
                let endpoint = endpoint(self.id, &self.base_url(), &self.extra)?;
                let method = request_method(self.id);
                let mut request = self.client.request(method, &endpoint);
                request = apply_auth(self.id, request, &credential);
                request = apply_headers(self.id, request);
                request = apply_body(self.id, request, &self.extra);
                send_value(request, self.id).await
            }
        }
    }

    async fn fetch_deepinfra(&self) -> Result<Value> {
        let credential = self.credential()?;
        let base = self.base_url();
        let checklist = if self.base_url.trim().is_empty() {
            "https://api.deepinfra.com/payment/checklist?compute_owed=true".to_string()
        } else {
            format!("{base}/payment/checklist?compute_owed=true")
        };
        let usage = if self.base_url.trim().is_empty() {
            "https://api.deepinfra.com/payment/usage?from=current".to_string()
        } else {
            format!("{base}/payment/usage?from=current")
        };
        let checklist = send_value(
            apply_auth(self.id, self.client.get(checklist), credential.as_str()),
            self.id,
        )
        .await?;
        let usage = send_value(
            apply_auth(self.id, self.client.get(usage), credential.as_str()),
            self.id,
        )
        .await
        .unwrap_or(Value::Null);
        Ok(serde_json::json!({ "checklist": checklist, "usage": usage }))
    }

    async fn fetch_chutes(&self) -> Result<Value> {
        let credential = self.credential()?;
        let base = self.base_url();
        let subscription = send_value(
            apply_auth(
                self.id,
                self.client
                    .get(format!("{base}/users/me/subscription_usage")),
                &credential,
            ),
            self.id,
        )
        .await?;
        let quotas = send_value(
            apply_auth(
                self.id,
                self.client.get(format!("{base}/users/me/quotas")),
                &credential,
            ),
            self.id,
        )
        .await
        .unwrap_or(Value::Null);
        Ok(serde_json::json!({ "subscription": subscription, "quotas": quotas }))
    }

    async fn fetch_zenmux(&self) -> Result<Value> {
        let credential = self.credential()?;
        let base = self.base_url();
        let subscription = send_value(
            self.client
                .get(format!("{base}/subscription/detail"))
                .bearer_auth(&credential)
                .header("Accept", "application/json"),
            self.id,
        )
        .await?;
        let balance = send_value(
            self.client
                .get(format!("{base}/payg/balance"))
                .bearer_auth(credential)
                .header("Accept", "application/json"),
            self.id,
        )
        .await
        .unwrap_or(Value::Null);
        Ok(serde_json::json!({ "subscription": subscription, "balance": balance }))
    }

    async fn fetch_zed(&self) -> Result<Value> {
        let token = self.credential()?;
        let authorization = if token.split_whitespace().count() >= 2 {
            token
        } else {
            let user_id = if !self.extra.trim().is_empty() {
                self.extra.trim().to_string()
            } else {
                std::env::var("ZED_USER_ID").context(
                    "Zed user ID not configured. Set Deployment / User ID or ZED_USER_ID",
                )?
            };
            format!("{user_id} {token}")
        };
        send_value(
            self.client
                .get(format!("{}/client/users/me", self.base_url()))
                .header("Authorization", authorization)
                .header("Accept", "application/json"),
            self.id,
        )
        .await
    }

    async fn fetch_wayfinder(&self) -> Result<Value> {
        let base = self.base_url();
        let savings = match send_value(
            self.client
                .get(format!("{base}/v1/savings"))
                .query(&[("period", "30d")]),
            self.id,
        )
        .await
        {
            Ok(value) => value,
            Err(_) => {
                send_value(
                    self.client
                        .get(format!("{base}/savings"))
                        .query(&[("period", "30d")]),
                    self.id,
                )
                .await?
            }
        };
        let health = send_value(self.client.get(format!("{base}/healthz")), self.id)
            .await
            .unwrap_or(Value::Null);
        Ok(serde_json::json!({ "savings": savings, "health": health }))
    }

    async fn fetch_qwencloud(&self) -> Result<Value> {
        let cookie = normalized_cookie(&self.credential()?);
        let dashboard_base = self.base_url();
        let dashboard_url = format!("{dashboard_base}/billing/subscription/token-plan-individual");
        let dashboard = self
            .client
            .get(&dashboard_url)
            .header("Accept", "text/html,application/xhtml+xml")
            .header("Cookie", &cookie)
            .send()
            .await
            .context("Failed to open the Qwen Cloud token-plan dashboard")?;
        let status = dashboard.status();
        let html = dashboard.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Qwen Cloud dashboard error {status}");
        }

        let sec_token =
            extract_qwen_sec_token(&html).or_else(|| cookie_named_value(&cookie, "sec_token"));
        let sec_token = match sec_token {
            Some(token) => token,
            None => {
                let user_info = send_value(
                    self.client
                        .get(format!("{dashboard_base}/tool/user/info.json"))
                        .header("Cookie", &cookie)
                        .header("Accept", "application/json, text/plain, */*"),
                    self.id,
                )
                .await
                .context("Qwen Cloud login expired; sign in again and refresh the cookie")?;
                find_string(&user_info, &["secToken", "sec_token", "csrfToken", "token"])
                    .context("Qwen Cloud login expired: sec_token was not found")?
            }
        };

        let usage = self
            .fetch_qwencloud_api(
                &cookie,
                &dashboard_url,
                &sec_token,
                "zeldaHttp.apikeyMgr./tokenplan/personal/api/v2/usage",
                serde_json::json!({}),
            )
            .await?;
        let subscription = self
            .fetch_qwencloud_api(
                &cookie,
                &dashboard_url,
                &sec_token,
                "zeldaHttp.apikeyMgr./tokenplan/personal/api/v2/subscription",
                serde_json::json!({ "commodityCode": "sfm_tokenplansolo_public_intl" }),
            )
            .await
            .unwrap_or(Value::Null);
        let quota_config = self
            .fetch_qwencloud_api(
                &cookie,
                &dashboard_url,
                &sec_token,
                "zeldaHttp.apikeyMgr./tokenplan/personal/api/v2/quota-config",
                serde_json::json!({}),
            )
            .await
            .unwrap_or(Value::Null);
        Ok(serde_json::json!({
            "usage": expand_embedded_json(usage),
            "subscription": expand_embedded_json(subscription),
            "quota_config": expand_embedded_json(quota_config),
        }))
    }

    async fn fetch_qwencloud_api(
        &self,
        cookie: &str,
        dashboard_url: &str,
        sec_token: &str,
        api: &str,
        mut data: Value,
    ) -> Result<Value> {
        let dashboard_base = self.base_url();
        let has_host_override = !self.base_url.trim().is_empty()
            || std::env::var("QWEN_CLOUD_HOST")
                .ok()
                .is_some_and(|value| !value.trim().is_empty());
        let data_base = if has_host_override {
            dashboard_base.clone()
        } else {
            "https://cs-data.qwencloud.com".to_string()
        };
        let url = format!(
            "{data_base}/data/api.json?action=IntlBroadScopeAspnGateway&product=sfm_bailian&api={api}&_v=undefined"
        );
        let anonymous_id = cookie_named_value(cookie, "cna");
        let cornerstone = serde_json::json!({
            "feTraceId": format!("quotify-{}", Utc::now().timestamp_millis()),
            "feURL": dashboard_url,
            "protocol": "V2",
            "console": "ONE_CONSOLE",
            "productCode": "p_efm",
            "domain": dashboard_base
                .trim_start_matches("https://")
                .trim_start_matches("http://"),
            "consoleSite": "QWENCLOUD",
            "userNickName": "",
            "userPrincipalName": "",
            "xsp_lang": "en-US",
            "X-Anonymous-Id": anonymous_id.unwrap_or_default(),
        });
        data.as_object_mut()
            .context("Qwen Cloud request data must be an object")?
            .insert("cornerstoneParam".to_string(), cornerstone);
        let params = serde_json::json!({
            "Api": api,
            "V": "1.0",
            "Data": data,
        })
        .to_string();
        let form = [
            ("product", "sfm_bailian".to_string()),
            ("action", "IntlBroadScopeAspnGateway".to_string()),
            ("sec_token", sec_token.to_string()),
            ("region", "ap-southeast-1".to_string()),
            ("language", "en-US".to_string()),
            ("params", params),
        ];
        let mut request = self
            .client
            .post(url)
            .header("Accept", "application/json, text/plain, */*")
            .header("Cookie", cookie)
            .header("Origin", &dashboard_base)
            .header("Referer", dashboard_url)
            .header("X-Requested-With", "XMLHttpRequest")
            .form(&form);
        if let Some(csrf) = cookie_named_value(cookie, "login_aliyunid_csrf")
            .or_else(|| cookie_named_value(cookie, "csrf"))
        {
            request = request
                .header("x-xsrf-token", &csrf)
                .header("x-csrf-token", csrf);
        }
        send_value(request, self.id).await
    }

    async fn fetch_xai(&self) -> Result<Value> {
        let credential = self.credential()?;
        let team = if !self.extra.trim().is_empty() {
            self.extra.trim().to_string()
        } else {
            std::env::var("XAI_TEAM_ID")
                .or_else(|_| std::env::var("XAI_MANAGEMENT_TEAM_ID"))
                .context("xAI Team ID not configured. Set Deployment / Team ID or XAI_TEAM_ID")?
        };
        let base = self.base_url();
        let url = format!("{}/v1/billing/teams/{team}/prepaid/balance", base);
        let balance_response = send_value(
            self.client
                .get(url)
                .bearer_auth(&credential)
                .header("Accept", "application/json"),
            self.id,
        )
        .await?;
        let ledger_cents = find_string(&balance_response, &["val"])
            .and_then(|value| value.parse::<f64>().ok())
            .context("xAI balance response did not contain total.val")?;
        let balance = -ledger_cents / 100.0;

        let now = Utc::now();
        let start = (now - Duration::days(29))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .context("Could not calculate xAI usage history start")?;
        let usage_body = serde_json::json!({
            "analyticsRequest": {
                "timeRange": {
                    "startTime": start.format("%Y-%m-%d %H:%M:%S").to_string(),
                    "endTime": now.format("%Y-%m-%d %H:%M:%S").to_string(),
                    "timezone": "Etc/GMT"
                },
                "timeUnit": "TIME_UNIT_DAY",
                "values": [{ "name": "usd", "aggregation": "AGGREGATION_SUM" }],
                "groupBy": [],
                "filters": []
            }
        });
        let history = send_value(
            self.client
                .post(format!("{base}/v1/billing/teams/{team}/usage"))
                .bearer_auth(credential)
                .header("Accept", "application/json")
                .json(&usage_body),
            self.id,
        )
        .await
        .unwrap_or(Value::Null);
        let spend = sum_xai_usage_values(&history);
        Ok(serde_json::json!({
            "balance": balance,
            "usage_30d": spend,
            "history": history,
        }))
    }
}

#[async_trait::async_trait]
impl Provider for CodexBarProvider {
    fn name(&self) -> &str {
        self.id
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let value = self.fetch_value().await?;
        let (mut windows, credits, subscription_tier) = parse_provider_value(self.id, &value);
        if windows.is_empty() && credits.is_none() {
            anyhow::bail!(
                "{} response did not contain recognizable usage or credit data",
                super::display_name(self.id)
            );
        }
        deduplicate_windows(&mut windows);
        Ok(UsageData {
            provider: self.id.to_string(),
            windows,
            credits,
            subscription_tier,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

async fn send_value(request: RequestBuilder, id: &str) -> Result<Value> {
    let response = request
        .send()
        .await
        .with_context(|| format!("Failed to connect to {}", super::display_name(id)))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "{} API error {status}: {}",
            super::display_name(id),
            text.chars().take(240).collect::<String>()
        );
    }
    if let Ok(value) = serde_json::from_str(&text) {
        return Ok(value);
    }
    Ok(Value::String(text))
}

fn endpoint(id: &str, base: &str, extra: &str) -> Result<String> {
    let endpoint = match id {
        "clinepass" => format!("{base}/api/v1/users/me/plan/usage-limits"),
        "alibaba" => {
            let (region, commodity) = if extra.eq_ignore_ascii_case("cn") {
                ("cn-beijing", "sfm_codingplan_public_cn")
            } else {
                ("ap-southeast-1", "sfm_codingplan_public_intl")
            };
            format!(
                "{base}/data/api.json?action=zeldaEasy.broadscope-bailian.codingPlan.queryCodingPlanInstanceInfoV2&product=broadscope-bailian&api=queryCodingPlanInstanceInfoV2&currentRegionId={region}&commodityCode={commodity}"
            )
        }
        "qwencloud" => format!("{base}/billing/subscription/token-plan-individual"),
        "devin" => {
            let org = if !extra.trim().is_empty() {
                extra.trim().to_string()
            } else {
                std::env::var("DEVIN_ORG_ID").context(
                    "Devin organization ID not configured. Set Deployment / Org ID or DEVIN_ORG_ID",
                )?
            };
            format!("{base}/api/{org}/billing/quota/usage")
        }
        "manus" => format!("{base}/user.v1.UserService/GetAvailableCredits"),
        "zed" => format!("{base}/client/users/me"),
        "perplexity" => format!("{base}/rest/billing/credits?version=2.18&source=default"),
        "sakana" => format!("{base}/billing"),
        "commandcode" => format!("{base}/internal/billing/credits"),
        "qoder" => format!("{base}/api/v2/me/usages/big_model_credits"),
        "litellm" => format!("{base}/key/info"),
        "poe" => format!("{base}/usage/current_balance"),
        "neuralwatt" => {
            if base.ends_with("/v1") {
                format!("{base}/quota")
            } else {
                format!("{base}/v1/quota")
            }
        }
        "clawrouter" => {
            if base.ends_with("/v1") {
                format!("{base}/usage")
            } else {
                format!("{base}/v1/usage")
            }
        }
        "longcat" => format!("{base}/api/lc-platform/v1/tokenUsage"),
        "sub2api" => {
            if base.ends_with("/v1") {
                format!("{base}/usage")
            } else {
                format!("{base}/v1/usage")
            }
        }
        "zenmux" => format!("{base}/subscription/detail"),
        "aiand" => format!("{base}/logs?range=30days&limit=100"),
        "zoommate" => format!("{base}/ai-computer/api/v1/credits/status"),
        _ => base.to_string(),
    };
    Ok(endpoint)
}

fn request_method(id: &str) -> Method {
    match id {
        "alibaba" | "manus" => Method::POST,
        _ => Method::GET,
    }
}

fn apply_auth(id: &str, request: RequestBuilder, credential: &str) -> RequestBuilder {
    match id {
        "alibaba" => request
            .bearer_auth(credential)
            .header("x-api-key", credential)
            .header("X-DashScope-API-Key", credential),
        "manus" => {
            let token = cookie_named_value(credential, "session_id")
                .or_else(|| cookie_named_value(credential, "session-token"))
                .unwrap_or_else(|| credential.to_string());
            request.bearer_auth(token)
        }
        "perplexity" => request.header("Cookie", cookie_value("pplx.session-token", credential)),
        "qwencloud" | "sakana" | "commandcode" | "qoder" | "longcat" => {
            request.header("Cookie", normalized_cookie(credential))
        }
        "zed" => request.bearer_auth(credential),
        "zoommate" => request
            .bearer_auth(credential)
            .header("Cookie", normalized_cookie(credential)),
        _ if credential.is_empty() => request,
        _ => request.bearer_auth(credential),
    }
}

fn apply_headers(id: &str, request: RequestBuilder) -> RequestBuilder {
    let request = request
        .header("Accept", "application/json, text/plain, */*")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Quotify/0.5",
        );
    match id {
        "alibaba" => request
            .header("Content-Type", "application/json")
            .header("Origin", "https://modelstudio.console.alibabacloud.com")
            .header(
                "Referer",
                "https://modelstudio.console.alibabacloud.com/ap-southeast-1/?tab=coding-plan",
            ),
        "perplexity" => request
            .header("Origin", "https://www.perplexity.ai")
            .header("Referer", "https://www.perplexity.ai/account/usage"),
        "manus" => request
            .header("Origin", "https://manus.im")
            .header("Referer", "https://manus.im/")
            .header("Connect-Protocol-Version", "1"),
        "commandcode" => request
            .header("Origin", "https://commandcode.ai")
            .header("Referer", "https://commandcode.ai/"),
        "sakana" => request.header("Accept", "text/html,application/xhtml+xml"),
        _ => request,
    }
}

fn apply_body(id: &str, request: RequestBuilder, extra: &str) -> RequestBuilder {
    match id {
        "alibaba" => request.json(&serde_json::json!({
            "queryCodingPlanInstanceInfoRequest": {
                "commodityCode": if extra.eq_ignore_ascii_case("cn") {
                    "sfm_codingplan_public_cn"
                } else {
                    "sfm_codingplan_public_intl"
                }
            }
        })),
        "manus" => request.json(&serde_json::json!({})),
        _ => request,
    }
}

fn parse_provider_value(
    id: &str,
    value: &Value,
) -> (Vec<UsageWindow>, Option<CreditsInfo>, Option<String>) {
    if let Value::String(text) = value {
        return parse_text_provider(id, text);
    }

    let mut windows = Vec::new();
    match id {
        "alibaba" => {
            push_ratio_window(
                &mut windows,
                "Session (5h)",
                find_number(
                    value,
                    &["per5HourUsedQuota", "perFiveHourUsedQuota", "fiveHourUsed"],
                ),
                find_number(
                    value,
                    &[
                        "per5HourTotalQuota",
                        "perFiveHourTotalQuota",
                        "fiveHourTotal",
                    ],
                ),
                find_date(value, &["per5HourQuotaNextRefreshTime", "fiveHourResetAt"]),
                None,
            );
            push_ratio_window(
                &mut windows,
                "Weekly",
                find_number(value, &["perWeekUsedQuota", "weeklyUsed"]),
                find_number(value, &["perWeekTotalQuota", "weeklyTotal"]),
                find_date(value, &["perWeekQuotaNextRefreshTime", "weeklyResetAt"]),
                None,
            );
            push_ratio_window(
                &mut windows,
                "Monthly",
                find_number(
                    value,
                    &["perBillMonthUsedQuota", "perMonthUsedQuota", "monthlyUsed"],
                ),
                find_number(
                    value,
                    &[
                        "perBillMonthTotalQuota",
                        "perMonthTotalQuota",
                        "monthlyTotal",
                    ],
                ),
                find_date(
                    value,
                    &[
                        "perBillMonthQuotaNextRefreshTime",
                        "perMonthQuotaNextRefreshTime",
                    ],
                ),
                None,
            );
        }
        "qwencloud" => {
            let tier = find_string(value, &["specCode", "spec_code", "planName", "plan_name"]);
            let five_hour_total = tier.as_deref().and_then(|tier| {
                find_object(value, &[tier])
                    .and_then(|object| direct_number(object, &["five_hour", "fiveHour"]))
            });
            let weekly_total = tier.as_deref().and_then(|tier| {
                find_object(value, &[tier]).and_then(|object| direct_number(object, &["weekly"]))
            });
            push_percent_window(
                &mut windows,
                "Session (5h)",
                find_number(value, &["per5HourPercentage"]).map(ratio_to_percent),
                five_hour_total,
                find_date(value, &["per5HourResetTime"]),
            );
            push_percent_window(
                &mut windows,
                "Weekly",
                find_number(value, &["per1WeekPercentage"]).map(ratio_to_percent),
                weekly_total,
                find_date(value, &["per1WeekResetTime"]),
            );
        }
        "devin" => {
            push_named_object_window(&mut windows, value, "Daily", &["daily", "day"]);
            push_named_object_window(&mut windows, value, "Weekly", &["weekly", "week"]);
        }
        "deepinfra" => {
            let used = find_number(value, &["total_cost", "month_cost", "current_month_cost"])
                .map(|amount| amount / 100.0);
            let limit = find_number(value, &["spending_limit", "monthly_limit", "limit"]);
            push_ratio_window(
                &mut windows,
                "Monthly Spend",
                used,
                limit,
                None,
                Some("USD"),
            );
        }
        "litellm" => {
            let used = find_number(value, &["spend", "key_spend", "user_spend", "team_spend"]);
            let limit = find_number(value, &["max_budget", "budget", "key_max_budget"]);
            push_ratio_window(&mut windows, "Budget", used, limit, None, Some("USD"));
        }
        "clawrouter" => {
            let used = find_number(value, &["spend", "spent", "usage_usd", "cost"]);
            let limit = find_number(value, &["budget", "monthly_budget", "limit"]);
            push_ratio_window(
                &mut windows,
                "Monthly Budget",
                used,
                limit,
                None,
                Some("USD"),
            );
        }
        "aiand" => {
            let spend = sum_numbers_for_key(value, &["cost", "spend", "amount"]);
            if spend > 0.0 {
                windows.push(metric_window("Spend (30d)", spend, "USD"));
            }
        }
        "wayfinder" => {
            if let Some(saved) =
                find_number(value, &["saved", "savings", "savings_usd", "amount_saved"])
            {
                windows.push(metric_window("Savings (30d)", saved, "USD"));
            }
            if let Some(requests) = find_number(value, &["requests", "total_requests", "decisions"])
            {
                windows.push(metric_window("Routed Requests", requests, "requests"));
            }
        }
        "xai" => {
            if let Some(spend) = find_number(value, &["usage_30d"]) {
                windows.push(metric_window("Spend (30d)", spend, "USD"));
            }
        }
        _ => {}
    }

    collect_quota_windows(value, &mut windows, "Usage");

    let credits = credits_from_value(id, value);
    let tier = find_string(
        value,
        &[
            "subscription_tier",
            "subscriptionType",
            "membershipType",
            "plan_name",
            "planName",
            "plan",
            "tier",
            "specCode",
            "spec_code",
            "plan_v3",
        ],
    )
    .filter(|value| !value.trim().is_empty());

    (windows, credits, tier)
}

fn parse_text_provider(
    id: &str,
    text: &str,
) -> (Vec<UsageWindow>, Option<CreditsInfo>, Option<String>) {
    let mut windows = Vec::new();
    if id == "sakana" {
        for (label, marker) in [("Session (5h)", "5[- ]?hour"), ("Weekly", "week(?:ly)?")] {
            let pattern = format!(r"(?is){marker}.{{0,240}}?(\d+(?:\.\d+)?)\s*%");
            if let Ok(regex) = regex::Regex::new(&pattern)
                && let Some(captures) = regex.captures(text)
                && let Ok(percent) = captures[1].parse::<f64>()
            {
                windows.push(percent_window(label, percent, None));
            }
        }
    }
    if let Ok(json) = serde_json::from_str(text) {
        return parse_provider_value(id, &json);
    }
    let credits = regex::Regex::new(
        r#"(?is)(?:balance|credits?|remaining).{0,80}?([$¥€]?\s*-?\d+(?:\.\d+)?)"#,
    )
    .ok()
    .and_then(|regex| regex.captures(text))
    .and_then(|captures| {
        captures[1]
            .replace(['$', '¥', '€', ' '], "")
            .parse::<f64>()
            .ok()
    })
    .map(|balance| CreditsInfo {
        balance,
        currency: "USD".to_string(),
        total_granted: None,
        topped_up: None,
    });
    (windows, credits, None)
}

fn collect_quota_windows(value: &Value, windows: &mut Vec<UsageWindow>, fallback: &str) {
    match value {
        Value::Object(map) => {
            if let Some(window) = window_from_object(map, fallback) {
                windows.push(window);
            }
            for (key, child) in map {
                let label = label_from_key(key).unwrap_or_else(|| fallback.to_string());
                match child {
                    Value::Object(object) => {
                        if let Some(window) = window_from_object(object, &label) {
                            windows.push(window);
                        } else {
                            collect_quota_windows(child, windows, &label);
                        }
                    }
                    Value::Array(_) => collect_quota_windows(child, windows, &label),
                    _ => {}
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_quota_windows(child, windows, fallback);
            }
        }
        _ => {}
    }
}

fn window_from_object(object: &serde_json::Map<String, Value>, label: &str) -> Option<UsageWindow> {
    let value = Value::Object(object.clone());
    let percent = direct_number(object, &["usage_percentage"])
        .map(ratio_to_percent)
        .or_else(|| {
            direct_number(
                object,
                &[
                    "used_percent",
                    "usedPercent",
                    "usage_percent",
                    "usagePercent",
                    "percent_used",
                    "percentUsed",
                    "totalPercentUsed",
                    "percentage",
                    "utilization",
                ],
            )
        });
    let used = direct_number(
        object,
        &[
            "used",
            "usage",
            "consumed",
            "spent",
            "current",
            "used_quota",
            "usedQuota",
            "credits_used",
            "used_flows",
        ],
    );
    let limit = direct_number(
        object,
        &[
            "limit",
            "total",
            "quota",
            "maximum",
            "max",
            "budget",
            "total_quota",
            "totalQuota",
            "credits_total",
            "max_flows",
        ],
    );
    let remaining = direct_number(
        object,
        &[
            "remaining",
            "remain",
            "left",
            "available",
            "credits_remaining",
            "remaining_flows",
        ],
    );
    let percent = percent.or_else(|| match (used, limit) {
        (Some(used), Some(limit)) if limit > 0.0 => Some(used / limit * 100.0),
        _ => match (remaining, limit) {
            (Some(remaining), Some(limit)) if limit > 0.0 => {
                Some((limit - remaining).max(0.0) / limit * 100.0)
            }
            _ => None,
        },
    })?;
    let reset = find_date(
        &value,
        &[
            "resets_at",
            "resetsAt",
            "reset_at",
            "resetAt",
            "next_reset",
            "nextReset",
            "renew_at",
            "renewsAt",
            "end_at",
            "endAt",
        ],
    );
    let label = direct_string(object, &["type", "kind", "period", "window", "name"])
        .and_then(|value| label_from_key(&value))
        .unwrap_or_else(|| label.to_string());
    Some(UsageWindow {
        label,
        used_percent: percent.clamp(0.0, 100.0),
        limit,
        used: used.or_else(|| match (remaining, limit) {
            (Some(remaining), Some(limit)) => Some((limit - remaining).max(0.0)),
            _ => None,
        }),
        unit: direct_string(object, &["unit", "currency"]),
        resets_at: reset,
    })
}

fn credits_from_value(id: &str, value: &Value) -> Option<CreditsInfo> {
    let balance_keys: &[&str] = match id {
        "manus" => &["available_credits", "availableCredits", "balance"],
        "perplexity" => &["remaining_credits", "remainingCredits", "balance"],
        "poe" => &[
            "balance",
            "points",
            "current_balance",
            "current_point_balance",
        ],
        "zenmux" => &["total_credits", "balance"],
        "xai" => &["balance", "amount", "credit_balance"],
        "deepinfra" => &["balance", "prepaid_balance", "credit_balance"],
        "zoommate" => &["remaining_credits", "available_credits", "balance"],
        _ => &[
            "balance",
            "credit_balance",
            "credits_remaining",
            "available_credits",
            "remaining_credits",
        ],
    };
    let mut balance = find_number(value, balance_keys)?;
    if id == "xai" && balance < 0.0 {
        balance = -balance / 100.0;
    }
    let total = find_number(
        value,
        &["total_credits", "credit_limit", "credits_total", "total"],
    );
    let currency = find_string(value, &["currency", "currency_code"])
        .unwrap_or_else(|| if id == "poe" { "points" } else { "USD" }.to_string());
    Some(CreditsInfo {
        balance,
        currency,
        total_granted: total,
        topped_up: None,
    })
}

fn push_named_object_window(
    windows: &mut Vec<UsageWindow>,
    value: &Value,
    label: &str,
    keys: &[&str],
) {
    if let Some(object) = find_object(value, keys)
        && let Some(window) = window_from_object(object, label)
    {
        windows.push(window);
    }
}

fn push_ratio_window(
    windows: &mut Vec<UsageWindow>,
    label: &str,
    used: Option<f64>,
    limit: Option<f64>,
    resets_at: Option<DateTime<Utc>>,
    unit: Option<&str>,
) {
    let Some(used) = used else { return };
    let Some(limit) = limit else { return };
    if limit <= 0.0 {
        return;
    }
    windows.push(UsageWindow {
        label: label.to_string(),
        used_percent: (used / limit * 100.0).clamp(0.0, 100.0),
        limit: Some(limit),
        used: Some(used),
        unit: unit.map(str::to_string),
        resets_at,
    });
}

fn push_percent_window(
    windows: &mut Vec<UsageWindow>,
    label: &str,
    percent: Option<f64>,
    limit: Option<f64>,
    resets_at: Option<DateTime<Utc>>,
) {
    let Some(percent) = percent else { return };
    let percent = percent.clamp(0.0, 100.0);
    windows.push(UsageWindow {
        label: label.to_string(),
        used_percent: percent,
        limit,
        used: limit.map(|limit| limit * percent / 100.0),
        unit: limit.map(|_| "tokens".to_string()),
        resets_at,
    });
}

fn ratio_to_percent(value: f64) -> f64 {
    if (0.0..=1.0).contains(&value) {
        value * 100.0
    } else {
        value
    }
}

fn percent_window(label: &str, percent: f64, resets_at: Option<DateTime<Utc>>) -> UsageWindow {
    UsageWindow {
        label: label.to_string(),
        used_percent: percent.clamp(0.0, 100.0),
        limit: Some(100.0),
        used: Some(percent.clamp(0.0, 100.0)),
        unit: Some("%".to_string()),
        resets_at,
    }
}

fn metric_window(label: &str, used: f64, unit: &str) -> UsageWindow {
    UsageWindow {
        label: label.to_string(),
        used_percent: 0.0,
        limit: None,
        used: Some(used),
        unit: Some(unit.to_string()),
        resets_at: None,
    }
}

fn deduplicate_windows(windows: &mut Vec<UsageWindow>) {
    let mut seen = HashSet::new();
    windows.retain(|window| {
        let key = format!(
            "{}:{:.4}:{:.4}:{:.4}",
            window.label.to_ascii_lowercase(),
            window.used_percent,
            window.used.unwrap_or_default(),
            window.limit.unwrap_or_default()
        );
        seen.insert(key)
    });
}

fn label_from_key(key: &str) -> Option<String> {
    let normalized = key
        .replace(['_', '-'], " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let lower = normalized.to_ascii_lowercase();
    if lower.contains("5h")
        || lower.contains("5 hour")
        || lower.contains("five hour")
        || lower.contains("rolling")
    {
        Some("Session (5h)".to_string())
    } else if lower.contains("week") || lower.contains("7 day") {
        Some("Weekly".to_string())
    } else if lower.contains("month") || lower.contains("billing") {
        Some("Monthly".to_string())
    } else if lower.contains("day") || lower.contains("daily") {
        Some("Daily".to_string())
    } else if lower.contains("edit prediction") {
        Some("Edit Predictions".to_string())
    } else if lower.contains("quota") || lower.contains("usage") || lower.contains("plan") {
        Some(normalized)
    } else {
        None
    }
}

fn find_object<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a serde_json::Map<String, Value>> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if keys
                    .iter()
                    .any(|candidate| key.eq_ignore_ascii_case(candidate))
                    && let Some(object) = child.as_object()
                {
                    return Some(object);
                }
            }
            map.values().find_map(|child| find_object(child, keys))
        }
        Value::Array(values) => values.iter().find_map(|child| find_object(child, keys)),
        _ => None,
    }
}

fn find_number(value: &Value, keys: &[&str]) -> Option<f64> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if keys
                    .iter()
                    .any(|candidate| key.eq_ignore_ascii_case(candidate))
                    && let Some(number) = number_value(child)
                {
                    return Some(number);
                }
            }
            map.values().find_map(|child| find_number(child, keys))
        }
        Value::Array(values) => values.iter().find_map(|child| find_number(child, keys)),
        _ => None,
    }
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if keys
                    .iter()
                    .any(|candidate| key.eq_ignore_ascii_case(candidate))
                    && let Some(text) = string_value(child)
                {
                    return Some(text);
                }
            }
            map.values().find_map(|child| find_string(child, keys))
        }
        Value::Array(values) => values.iter().find_map(|child| find_string(child, keys)),
        _ => None,
    }
}

fn find_date(value: &Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if keys
                    .iter()
                    .any(|candidate| key.eq_ignore_ascii_case(candidate))
                    && let Some(date) = date_value(child)
                {
                    return Some(date);
                }
            }
            map.values().find_map(|child| find_date(child, keys))
        }
        Value::Array(values) => values.iter().find_map(|child| find_date(child, keys)),
        _ => None,
    }
}

fn direct_number(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    map.iter()
        .find(|(key, _)| {
            keys.iter()
                .any(|candidate| key.eq_ignore_ascii_case(candidate))
        })
        .and_then(|(_, value)| number_value(value))
}

fn direct_string(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    map.iter()
        .find(|(key, _)| {
            keys.iter()
                .any(|candidate| key.eq_ignore_ascii_case(candidate))
        })
        .and_then(|(_, value)| string_value(value))
}

fn number_value(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| {
        value
            .as_str()?
            .trim()
            .trim_end_matches('%')
            .replace([',', '$'], "")
            .parse()
            .ok()
    })
}

fn string_value(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_i64().map(|number| number.to_string()))
}

fn date_value(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(raw) = value.as_str() {
        if let Ok(date) = DateTime::parse_from_rfc3339(raw) {
            return Some(date.with_timezone(&Utc));
        }
        if let Ok(timestamp) = raw.parse::<i64>() {
            return timestamp_date(timestamp);
        }
    }
    value.as_i64().and_then(timestamp_date)
}

fn timestamp_date(timestamp: i64) -> Option<DateTime<Utc>> {
    let seconds = if timestamp.abs() > 10_000_000_000 {
        timestamp / 1000
    } else {
        timestamp
    };
    Utc.timestamp_opt(seconds, 0).single()
}

fn sum_numbers_for_key(value: &Value, keys: &[&str]) -> f64 {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, child)| {
                if keys
                    .iter()
                    .any(|candidate| key.eq_ignore_ascii_case(candidate))
                {
                    number_value(child).unwrap_or_default()
                } else {
                    sum_numbers_for_key(child, keys)
                }
            })
            .sum(),
        Value::Array(values) => values
            .iter()
            .map(|child| sum_numbers_for_key(child, keys))
            .sum(),
        _ => 0.0,
    }
}

fn normalized_cookie(raw: &str) -> String {
    raw.strip_prefix("Cookie:")
        .or_else(|| raw.strip_prefix("cookie:"))
        .unwrap_or(raw)
        .trim()
        .to_string()
}

fn cookie_named_value(raw: &str, name: &str) -> Option<String> {
    normalized_cookie(raw).split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

fn extract_qwen_sec_token(html: &str) -> Option<String> {
    [
        r#""secToken"\s*:\s*"([^"]+)""#,
        r#""sec_token"\s*:\s*"([^"]+)""#,
        r#"secToken['"]?\s*[:=]\s*['"]([^'"]+)['"]"#,
        r#"sec_token['"]?\s*[:=]\s*['"]([^'"]+)['"]"#,
        r#"csrfToken['"]?\s*[:=]\s*['"]([^'"]+)['"]"#,
    ]
    .iter()
    .find_map(|pattern| {
        regex::Regex::new(pattern)
            .ok()?
            .captures(html)?
            .get(1)
            .map(|value| value.as_str().trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn expand_embedded_json(value: Value) -> Value {
    match value {
        Value::String(text) => serde_json::from_str(&text)
            .map(expand_embedded_json)
            .unwrap_or(Value::String(text)),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(expand_embedded_json).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, expand_embedded_json(value)))
                .collect(),
        ),
        other => other,
    }
}

fn sum_xai_usage_values(value: &Value) -> f64 {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, child)| {
                if key.eq_ignore_ascii_case("values") {
                    child
                        .as_array()
                        .map(|values| values.iter().filter_map(number_value).sum())
                        .unwrap_or_default()
                } else {
                    sum_xai_usage_values(child)
                }
            })
            .sum(),
        Value::Array(values) => values.iter().map(sum_xai_usage_values).sum(),
        _ => 0.0,
    }
}

fn cookie_value(name: &str, raw: &str) -> String {
    let cookie = normalized_cookie(raw);
    if cookie.contains('=') {
        cookie
    } else {
        format!("{name}={cookie}")
    }
}

fn credential_environment_keys(id: &str) -> &'static [&'static str] {
    match id {
        "clinepass" => &["CLINE_API_KEY", "CLINEPASS_API_KEY"],
        "alibaba" => &[
            "ALIBABA_CODING_PLAN_API_KEY",
            "ALIBABA_QWEN_API_KEY",
            "DASHSCOPE_API_KEY",
        ],
        "qwencloud" => &["QWEN_CLOUD_COOKIE"],
        "devin" => &["DEVIN_BEARER_TOKEN"],
        "manus" => &["MANUS_SESSION_TOKEN", "MANUS_COOKIE"],
        "zed" => &["ZED_SESSION_TOKEN", "ZED_ACCESS_TOKEN"],
        "perplexity" => &["PERPLEXITY_SESSION_TOKEN", "PERPLEXITY_COOKIE"],
        "sakana" => &["SAKANA_COOKIE"],
        "deepinfra" => &["DEEPINFRA_API_KEY", "DEEPINFRA_TOKEN"],
        "commandcode" => &["COMMANDCODE_COOKIE", "COMMAND_CODE_COOKIE"],
        "qoder" => &["QODER_COOKIE"],
        "litellm" => &["LITELLM_API_KEY"],
        "poe" => &["POE_API_KEY"],
        "chutes" => &["CHUTES_API_KEY"],
        "neuralwatt" => &["NEURALWATT_API_KEY"],
        "clawrouter" => &["CLAWROUTER_API_KEY"],
        "longcat" => &["LONGCAT_MANUAL_COOKIE", "LONGCAT_COOKIE"],
        "sub2api" => &["SUB2API_API_KEY"],
        "zenmux" => &["ZENMUX_MANAGEMENT_API_KEY"],
        "aiand" => &["AIAND_API_KEY"],
        "zoommate" => &["ZOOMMATE_TOKEN", "ZOOMMATE_COOKIE"],
        "xai" => &["XAI_MANAGEMENT_API_KEY"],
        _ => &[],
    }
}

fn base_url_environment_key(id: &str) -> Option<&'static str> {
    match id {
        "alibaba" => Some("ALIBABA_CODING_PLAN_HOST"),
        "qwencloud" => Some("QWEN_CLOUD_HOST"),
        "litellm" => Some("LITELLM_BASE_URL"),
        "chutes" => Some("CHUTES_API_URL"),
        "neuralwatt" => Some("NEURALWATT_API_URL"),
        "clawrouter" => Some("CLAWROUTER_BASE_URL"),
        "sub2api" => Some("SUB2API_BASE_URL"),
        "wayfinder" => Some("WAYFINDER_GATEWAY_URL"),
        _ => None,
    }
}

fn default_base_url(id: &str) -> &'static str {
    match id {
        "clinepass" => "https://api.cline.bot",
        "alibaba" => "https://modelstudio.console.alibabacloud.com",
        "qwencloud" => "https://home.qwencloud.com",
        "devin" => "https://app.devin.ai",
        "manus" => "https://api.manus.im",
        "zed" => "https://cloud.zed.dev",
        "perplexity" => "https://www.perplexity.ai",
        "sakana" => "https://console.sakana.ai",
        "deepinfra" => "https://api.deepinfra.com",
        "commandcode" => "https://api.commandcode.ai",
        "qoder" => "https://qoder.com",
        "litellm" => "",
        "poe" => "https://api.poe.com",
        "chutes" => "https://api.chutes.ai",
        "neuralwatt" => "https://api.neuralwatt.com",
        "clawrouter" => "https://clawrouter.openclaw.ai",
        "longcat" => "https://longcat.chat",
        "sub2api" => "",
        "wayfinder" => "http://127.0.0.1:8088",
        "zenmux" => "https://zenmux.ai/api/v1/management",
        "aiand" => "https://api.aiand.com",
        "zoommate" => "https://ai.zoom.us",
        "xai" => "https://management-api.x.ai",
        _ => "",
    }
}

macro_rules! define_codexbar_provider {
    ($name:ident, $id:literal) => {
        pub struct $name(super::codexbar::CodexBarProvider);

        impl $name {
            pub fn new(
                api_key: String,
                base_url: String,
                extra: String,
                proxy: Option<&str>,
            ) -> Self {
                Self(super::codexbar::CodexBarProvider::new(
                    $id, api_key, base_url, extra, proxy,
                ))
            }
        }

        #[async_trait::async_trait]
        impl super::Provider for $name {
            fn name(&self) -> &str {
                self.0.name()
            }

            async fn fetch_usage(&self) -> anyhow::Result<super::UsageData> {
                self.0.fetch_usage().await
            }
        }
    };
}

pub(crate) use define_codexbar_provider;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_independent_quota_windows_and_tier() {
        let value = serde_json::json!({
            "planName": "Pro",
            "limits": {
                "five_hour": { "used": 25, "limit": 100, "resetAt": "2030-01-01T00:00:00Z" },
                "weekly": { "usagePercent": 63, "resetAt": 1893456000 }
            }
        });
        let (windows, _, tier) = parse_provider_value("clinepass", &value);
        assert_eq!(tier.as_deref(), Some("Pro"));
        assert!(
            windows
                .iter()
                .any(|window| window.label == "Session (5h)" && window.used_percent == 25.0)
        );
        assert!(
            windows
                .iter()
                .any(|window| window.label == "Weekly" && window.used_percent == 63.0)
        );
    }

    #[test]
    fn parses_remaining_credit_balance() {
        let value = serde_json::json!({ "data": { "remainingCredits": 42.5, "currency": "USD" } });
        let (_, credits, _) = parse_provider_value("perplexity", &value);
        assert_eq!(credits.unwrap().balance, 42.5);
    }

    #[test]
    fn xai_negative_cent_balance_is_normalized() {
        let value = serde_json::json!({ "balance": -12345 });
        let (_, credits, _) = parse_provider_value("xai", &value);
        assert_eq!(credits.unwrap().balance, 123.45);
    }

    #[test]
    fn qwencloud_parses_ratio_windows_and_plan_quota() {
        let value = serde_json::json!({
            "usage": {
                "per5HourPercentage": 0.25,
                "per5HourResetTime": "2030-01-01T00:00:00Z",
                "per1WeekPercentage": 0.75,
                "per1WeekResetTime": "2030-01-07T00:00:00Z"
            },
            "subscription": { "specCode": "pro" },
            "quota_config": {
                "pro": { "five_hour": 1000, "weekly": 10000 }
            }
        });
        let (windows, _, tier) = parse_provider_value("qwencloud", &value);
        assert_eq!(tier.as_deref(), Some("pro"));
        assert!(windows.iter().any(|window| {
            window.label == "Session (5h)"
                && window.used_percent == 25.0
                && window.used == Some(250.0)
        }));
        assert!(windows.iter().any(|window| {
            window.label == "Weekly" && window.used_percent == 75.0 && window.limit == Some(10000.0)
        }));
    }

    #[test]
    fn expands_json_strings_in_gateway_envelopes() {
        let value = serde_json::json!({
            "data": "{\"per5HourPercentage\":0.5}"
        });
        let expanded = expand_embedded_json(value);
        assert_eq!(find_number(&expanded, &["per5HourPercentage"]), Some(0.5));
    }

    #[test]
    fn sums_xai_daily_usage_series() {
        let value = serde_json::json!({
            "timeSeries": [
                { "dataPoints": [{ "values": [1.25] }, { "values": [2.75] }] },
                { "dataPoints": [{ "values": [0.5] }] }
            ]
        });
        assert_eq!(sum_xai_usage_values(&value), 4.5);
    }

    #[test]
    fn parses_zenmux_fractional_quota_windows_and_balance() {
        let value = serde_json::json!({
            "subscription": {
                "data": {
                    "plan": { "tier": "pro" },
                    "quota_5_hour": {
                        "usage_percentage": 0.4,
                        "max_flows": 100,
                        "used_flows": 40,
                        "resets_at": "2030-01-01T00:00:00Z"
                    },
                    "quota_7_day": {
                        "usage_percentage": 0.7,
                        "max_flows": 1000,
                        "used_flows": 700
                    }
                }
            },
            "balance": {
                "data": { "currency": "USD", "total_credits": 12.5 }
            }
        });
        let (windows, credits, tier) = parse_provider_value("zenmux", &value);
        assert_eq!(tier.as_deref(), Some("pro"));
        assert!(windows.iter().any(|window| {
            window.label == "Session (5h)"
                && window.used_percent == 40.0
                && window.used == Some(40.0)
        }));
        assert!(
            windows
                .iter()
                .any(|window| { window.label == "Weekly" && window.used_percent == 70.0 })
        );
        assert_eq!(credits.unwrap().balance, 12.5);
    }
}
