use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageHistory {
    pub entries: Vec<UsageHistoryEntry>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub budget_refresh_failures: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageHistoryEntry {
    pub fetched_at: DateTime<Utc>,
    pub providers: Vec<crate::provider::UsageData>,
}

#[derive(Debug, Clone)]
pub struct ProviderTrend {
    pub samples: usize,
    pub latest_percent: f64,
    pub previous_percent: Option<f64>,
    pub peak_percent: f64,
    pub average_percent: f64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub observations: Vec<ProviderTrendObservation>,
}

/// Stable, presentation-independent identity for one provider quota window.
///
/// Providers do not currently expose an explicit window id, so this key is
/// derived from semantic period aliases and falls back to a normalized
/// label/unit pair. It is intentionally not serialized: historical snapshots
/// already contain the complete `UsageWindow` values and can be reclassified
/// as aliases improve.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UsageWindowKey(String);

impl UsageWindowKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn budget_30d_usd() -> Self {
        Self("budget:30d:usd".to_string())
    }
}

#[derive(Debug, Clone)]
pub struct UsageWindowTrend {
    pub key: UsageWindowKey,
    pub label: String,
    pub trend: ProviderTrend,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderTrendObservation {
    pub fetched_at: DateTime<Utc>,
    pub used_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderTrendBucket {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub latest_percent: Option<f64>,
    pub average_percent: Option<f64>,
    pub peak_percent: Option<f64>,
    pub samples: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendHistogramState {
    Chart,
    AllRoundedToZero,
    LatestUnavailable,
}

pub fn classify_histogram_buckets(buckets: &[ProviderTrendBucket]) -> TrendHistogramState {
    let mut has_latest_value = false;
    for value in buckets
        .iter()
        .filter_map(|bucket| bucket.latest_percent)
        .filter(|value| value.is_finite())
        .map(|value| value.max(0.0))
    {
        has_latest_value = true;
        if value >= 0.05 {
            return TrendHistogramState::Chart;
        }
    }

    if has_latest_value {
        TrendHistogramState::AllRoundedToZero
    } else {
        TrendHistogramState::LatestUnavailable
    }
}

impl ProviderTrend {
    pub fn histogram_buckets(&self, bucket_count: usize) -> Vec<ProviderTrendBucket> {
        if bucket_count == 0 {
            return Vec::new();
        }

        let total_millis = (self.period_end - self.period_start).num_milliseconds();
        let mut buckets = (0..bucket_count)
            .map(|index| {
                let start_offset = total_millis.saturating_mul(index as i64) / bucket_count as i64;
                let end_offset =
                    total_millis.saturating_mul((index + 1) as i64) / bucket_count as i64;
                ProviderTrendBucket {
                    starts_at: self.period_start + chrono::Duration::milliseconds(start_offset),
                    ends_at: self.period_start + chrono::Duration::milliseconds(end_offset),
                    latest_percent: None,
                    average_percent: None,
                    peak_percent: None,
                    samples: 0,
                }
            })
            .collect::<Vec<_>>();
        if total_millis <= 0 {
            return buckets;
        }

        let mut sums = vec![0.0; bucket_count];
        for observation in &self.observations {
            if observation.fetched_at < self.period_start
                || observation.fetched_at > self.period_end
            {
                continue;
            }

            let elapsed_millis = (observation.fetched_at - self.period_start).num_milliseconds();
            let bucket_index =
                ((elapsed_millis as i128 * bucket_count as i128) / total_millis as i128)
                    .clamp(0, bucket_count.saturating_sub(1) as i128) as usize;
            let bucket = &mut buckets[bucket_index];
            bucket.latest_percent = observation.used_percent;
            if let Some(used_percent) = observation.used_percent.filter(|value| value.is_finite()) {
                sums[bucket_index] += used_percent;
                bucket.samples += 1;
                bucket.peak_percent = Some(
                    bucket
                        .peak_percent
                        .map_or(used_percent, |peak| peak.max(used_percent)),
                );
            }
        }

        for (index, bucket) in buckets.iter_mut().enumerate() {
            if bucket.samples > 0 {
                bucket.average_percent = Some(sums[index] / bucket.samples as f64);
            }
        }

        buckets
    }
}

impl UsageHistory {
    pub fn load() -> Self {
        let path = history_path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = history_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content).with_context(|| format!("Failed to write {:?}", path))?;
        Ok(())
    }

    pub fn append(&mut self, providers: Vec<crate::provider::UsageData>) {
        // A refresh can yield no provider results at all (for example, an
        // isolated configuration or a startup race). That is not evidence
        // that every previously tracked provider was removed, so it must not
        // create a global discontinuity in every trend.
        if providers.is_empty() {
            tracing::debug!("Skipping empty usage history snapshot");
            return;
        }

        self.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now(),
            providers,
        });
        // Keep enough raw snapshots for a complete seven-day trend at the
        // default five-minute refresh interval, with room for manual refreshes.
        self.prune(30, 2500);
    }

    pub fn latest_successful(&self) -> Vec<crate::provider::UsageData> {
        let mut providers = self
            .entries
            .iter()
            .rev()
            .find(|entry| {
                entry
                    .providers
                    .iter()
                    .any(|provider| provider.error.is_none())
            })
            .map(|entry| entry.providers.clone())
            .unwrap_or_default();
        self.carry_forward_codex_reset_credits(&mut providers, Utc::now());
        providers
    }

    pub fn carry_forward_codex_reset_credits(
        &self,
        providers: &mut [crate::provider::UsageData],
        now: DateTime<Utc>,
    ) -> bool {
        let Some(codex) = providers.iter_mut().find(|data| {
            data.provider.eq_ignore_ascii_case("codex")
                && data.error.is_none()
                && crate::provider::codex::reset_credits(data).is_none()
        }) else {
            return false;
        };

        let Some(mut resets) = self.entries.iter().rev().find_map(|entry| {
            entry
                .providers
                .iter()
                .find(|data| data.provider.eq_ignore_ascii_case("codex") && data.error.is_none())
                .and_then(crate::provider::codex::reset_credits)
        }) else {
            return false;
        };

        resets.credits.retain(|credit| {
            credit.status.eq_ignore_ascii_case("available")
                && credit.expires_at.is_none_or(|expires_at| expires_at > now)
        });
        resets.available_count = i32::try_from(resets.credits.len()).unwrap_or(i32::MAX);
        if resets.available_count == 0 {
            return false;
        }

        crate::provider::codex::attach_reset_credits(codex, &resets)
    }

    /// Computes independent trends for every meaningful window in `current`.
    ///
    /// The history is traversed once. A successful provider snapshot that is
    /// missing one target window contributes an explicit gap to that window
    /// instead of substituting another window's percentage. Configured API
    /// budgets are tracked as an additional window and a budget-limit change
    /// terminates only that budget generation.
    pub fn trends_for_at_with_budget(
        &self,
        current: &crate::provider::UsageData,
        days: i64,
        now: DateTime<Utc>,
        configured_budget: Option<f64>,
    ) -> Vec<UsageWindowTrend> {
        if days <= 0 || current.provider.trim().is_empty() || current.error.is_some() {
            return Vec::new();
        }

        let configured_budget = valid_configured_budget(&current.provider, configured_budget);
        let mut targets = Vec::<WindowTrendAccumulator>::new();
        let mut target_indexes = HashMap::<UsageWindowKey, usize>::new();

        for window in &current.windows {
            let Some(key) = usage_window_key(&current.provider, window) else {
                continue;
            };
            let is_budget = key == UsageWindowKey::budget_30d_usd();
            if is_budget && configured_budget.is_none() {
                continue;
            }
            if !is_budget && is_raw_amount_window(window) {
                continue;
            }
            if target_indexes.contains_key(&key) {
                tracing::warn!(
                    provider = %current.provider,
                    window = %window.label,
                    key = key.as_str(),
                    "Ignoring duplicate usage trend window identity"
                );
                continue;
            }

            target_indexes.insert(key.clone(), targets.len());
            targets.push(WindowTrendAccumulator::new(
                key,
                window.label.clone(),
                is_budget,
            ));
        }

        // A partial cost refresh can omit the budget window entirely. Keep its
        // historical series visible and record the current snapshot as a gap.
        if configured_budget.is_some() {
            let budget_key = UsageWindowKey::budget_30d_usd();
            if !target_indexes.contains_key(&budget_key) {
                target_indexes.insert(budget_key.clone(), targets.len());
                targets.push(WindowTrendAccumulator::new(
                    budget_key,
                    "30d Budget".to_string(),
                    true,
                ));
            }
        }

        if targets.is_empty() {
            return Vec::new();
        }

        let cutoff = now - chrono::Duration::days(days);
        let mut provider_segment_started = false;
        for entry in self.entries.iter().rev() {
            if entry.fetched_at > now {
                continue;
            }
            if entry.fetched_at < cutoff {
                break;
            }
            // Older versions could persist a completely empty refresh. Treat
            // those entries as no observation rather than as every provider
            // being removed, so existing history remains continuous.
            if entry.providers.is_empty() {
                continue;
            }

            let Some(data) = entry
                .providers
                .iter()
                .find(|data| data.provider.eq_ignore_ascii_case(&current.provider))
            else {
                if provider_segment_started {
                    break;
                }
                continue;
            };
            provider_segment_started = true;

            // Preserve the previous behavior for whole-provider failures: they
            // are not valid quota observations. Partial successful responses,
            // however, do create per-window gaps below.
            if data.error.is_some() {
                continue;
            }

            let mut windows_by_key =
                HashMap::<UsageWindowKey, &crate::provider::UsageWindow>::new();
            for window in &data.windows {
                if let Some(key) = usage_window_key(&data.provider, window) {
                    windows_by_key.entry(key).or_insert(window);
                }
            }

            for target in &mut targets {
                if target.generation_ended {
                    continue;
                }

                let matched = windows_by_key.get(&target.key).copied();
                let used_percent = if target.is_budget {
                    let Some(expected_budget) = configured_budget else {
                        continue;
                    };
                    match matched {
                        Some(window) => match window
                            .limit
                            .filter(|limit| limit.is_finite() && *limit > 0.0)
                        {
                            Some(historical_limit)
                                if !same_optional_limit(
                                    Some(expected_budget),
                                    Some(historical_limit),
                                ) =>
                            {
                                target.generation_ended = true;
                                continue;
                            }
                            Some(_) => window
                                .used_percent
                                .is_finite()
                                .then_some(window.used_percent),
                            None => None,
                        },
                        None => None,
                    }
                } else {
                    matched
                        .map(|window| window.used_percent)
                        .filter(|percent| percent.is_finite())
                };
                target.observations.push(ProviderTrendObservation {
                    fetched_at: entry.fetched_at,
                    used_percent,
                });
            }
        }

        targets
            .into_iter()
            .filter_map(|mut target| {
                target.observations.reverse();
                build_trend(target.observations, cutoff, now).map(|trend| UsageWindowTrend {
                    key: target.key,
                    label: target.label,
                    trend,
                })
            })
            .collect()
    }

    pub fn latest_successful_for(&self, provider: &str) -> Option<crate::provider::UsageData> {
        self.entries.iter().rev().find_map(|entry| {
            entry
                .providers
                .iter()
                .find(|data| data.provider.eq_ignore_ascii_case(provider) && data.error.is_none())
                .cloned()
        })
    }

    fn prune(&mut self, days: i64, max_entries: usize) {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        self.entries.retain(|entry| entry.fetched_at >= cutoff);
        if self.entries.len() > max_entries {
            let remove_count = self.entries.len() - max_entries;
            self.entries.drain(0..remove_count);
        }
    }
}

#[derive(Debug)]
struct WindowTrendAccumulator {
    key: UsageWindowKey,
    label: String,
    is_budget: bool,
    generation_ended: bool,
    observations: Vec<ProviderTrendObservation>,
}

impl WindowTrendAccumulator {
    fn new(key: UsageWindowKey, label: String, is_budget: bool) -> Self {
        Self {
            key,
            label,
            is_budget,
            generation_ended: false,
            observations: Vec::new(),
        }
    }
}

fn build_trend(
    observations: Vec<ProviderTrendObservation>,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Option<ProviderTrend> {
    let points = observations
        .iter()
        .filter_map(|observation| {
            observation
                .used_percent
                .filter(|percent| percent.is_finite())
        })
        .collect::<Vec<_>>();
    let latest_percent = *points.last()?;
    let previous_percent = points.iter().rev().nth(1).copied();
    let peak_percent = points.iter().copied().fold(0.0, f64::max);
    let average_percent = points.iter().sum::<f64>() / points.len() as f64;

    Some(ProviderTrend {
        samples: points.len(),
        latest_percent,
        previous_percent,
        peak_percent,
        average_percent,
        period_start,
        period_end,
        observations,
    })
}

fn valid_configured_budget(provider: &str, configured_budget: Option<f64>) -> Option<f64> {
    configured_budget.filter(|budget| {
        crate::provider::supports_api_budget(provider) && budget.is_finite() && *budget > 0.0
    })
}

/// Derives a stable runtime identity from provider aliases and the window's
/// semantic period. The source `UsageWindow` schema remains unchanged.
pub fn usage_window_key(
    provider: &str,
    window: &crate::provider::UsageWindow,
) -> Option<UsageWindowKey> {
    let compact_label = compact_normalized(&window.label);
    if matches!(
        compact_label.as_str(),
        "" | "error" | "nodata" | "resetcredits"
    ) {
        return None;
    }

    if crate::provider::is_budget_spend_window(provider, &window.label, window.unit.as_deref()) {
        return Some(UsageWindowKey::budget_30d_usd());
    }

    let tokens = normalized_tokens(&window.label);
    let provider = provider.trim().to_ascii_lowercase();
    if let Some((period, qualifier)) =
        semantic_period_and_qualifier(&provider, &compact_label, &tokens)
    {
        let mut key = format!("quota:{period}");
        if !qualifier.is_empty() {
            key.push(':');
            key.push_str(&qualifier);
        }
        return Some(UsageWindowKey(key));
    }

    let normalized_label = normalized_component(&window.label);
    if normalized_label.is_empty() {
        return None;
    }
    let normalized_unit = window
        .unit
        .as_deref()
        .map(normalized_component)
        .filter(|unit| !unit.is_empty());
    Some(UsageWindowKey(match normalized_unit {
        Some(unit) => format!("named:{normalized_label}:{unit}"),
        None => format!("named:{normalized_label}"),
    }))
}

fn semantic_period_and_qualifier(
    provider: &str,
    compact_label: &str,
    tokens: &[String],
) -> Option<(&'static str, String)> {
    let has_token = |expected: &str| tokens.iter().any(|token| token == expected);
    let has_pair = |left: &str, right: &str| {
        tokens
            .windows(2)
            .any(|pair| pair[0] == left && pair[1] == right)
    };

    let (period, period_noise): (&str, &[&str]) = if has_token("5h")
        || compact_label.contains("5hour")
        || compact_label.contains("fivehour")
        || (matches!(provider, "codex" | "claude") && compact_label.contains("session"))
        || (matches!(provider, "opencode" | "opencodego") && compact_label.contains("rollingusage"))
        || (provider == "codex" && matches!(compact_label, "primary" | "primarywindow"))
    {
        (
            "5h",
            &[
                "5h", "5hour", "five", "hour", "hours", "session", "rolling", "primary",
            ],
        )
    } else if has_token("7d")
        || has_token("weekly")
        || has_token("week")
        || compact_label.contains("sevenday")
        || has_pair("seven", "day")
        || (provider == "codex" && matches!(compact_label, "secondary" | "secondarywindow"))
    {
        (
            "weekly",
            &["7d", "weekly", "week", "seven", "day", "days", "secondary"],
        )
    } else if has_token("monthly")
        || has_token("month")
        || has_token("30d")
        || compact_label.contains("thirtyday")
        || has_pair("thirty", "day")
    {
        (
            "monthly",
            &["30d", "monthly", "month", "thirty", "day", "days"],
        )
    } else {
        return None;
    };

    let qualifier = tokens
        .iter()
        .filter(|token| {
            !matches!(
                token.as_str(),
                "usage" | "quota" | "window" | "limit" | "rate"
            ) && !period_noise.contains(&token.as_str())
        })
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("-");
    Some((period, qualifier))
}

fn normalized_tokens(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.chars().flat_map(char::to_lowercase).collect())
        .collect()
}

fn normalized_component(value: &str) -> String {
    normalized_tokens(value).join("-")
}

fn compact_normalized(value: &str) -> String {
    normalized_tokens(value).concat()
}

fn is_raw_amount_window(window: &crate::provider::UsageWindow) -> bool {
    let compact_label = compact_normalized(&window.label);
    compact_label.contains("localstats")
        || (window.limit.is_none()
            && window.used.is_some()
            && window.used_percent.abs() < 0.05
            && window.unit.as_deref().is_some_and(|unit| {
                matches!(
                    unit.trim().to_ascii_lowercase().as_str(),
                    "usd" | "eur" | "gbp" | "cny" | "jpy" | "token" | "tokens"
                )
            }))
}

fn same_optional_limit(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            (left - right).abs() <= f64::EPSILON * left.abs().max(right.abs()).max(1.0)
        }
        (None, None) => true,
        _ => false,
    }
}

pub fn history_path() -> PathBuf {
    crate::diagnostics::app_dir().join("usage-history.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{UsageData, UsageWindow};
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 0)
            .single()
            .unwrap()
    }

    fn usage(provider: &str, percent: f64, error: Option<&str>) -> UsageData {
        UsageData {
            provider: provider.to_string(),
            windows: vec![UsageWindow {
                label: "Monthly".to_string(),
                used_percent: percent,
                limit: None,
                used: None,
                unit: None,
                resets_at: None,
            }],
            credits: None,
            subscription_tier: None,
            fetched_at: Utc::now(),
            error: error.map(str::to_string),
        }
    }

    fn budget_usage(percent: f64, limit: f64) -> UsageData {
        UsageData {
            provider: "openai".to_string(),
            windows: vec![UsageWindow {
                label: "Cost 30d".to_string(),
                used_percent: percent,
                limit: Some(limit),
                used: Some(percent / 100.0 * limit),
                unit: Some("USD".to_string()),
                resets_at: None,
            }],
            credits: None,
            subscription_tier: None,
            fetched_at: Utc::now(),
            error: None,
        }
    }

    fn window(label: &str, percent: f64) -> UsageWindow {
        UsageWindow {
            label: label.to_string(),
            used_percent: percent,
            limit: None,
            used: None,
            unit: None,
            resets_at: None,
        }
    }

    fn usage_with_windows(
        provider: &str,
        fetched_at: DateTime<Utc>,
        windows: Vec<UsageWindow>,
    ) -> UsageData {
        UsageData {
            provider: provider.to_string(),
            windows,
            credits: None,
            subscription_tier: None,
            fetched_at,
            error: None,
        }
    }

    fn budget_window(label: &str, percent: f64, limit: f64) -> UsageWindow {
        UsageWindow {
            label: label.to_string(),
            used_percent: percent,
            limit: Some(limit),
            used: Some(percent / 100.0 * limit),
            unit: Some("USD".to_string()),
            resets_at: None,
        }
    }

    fn reset_credit(
        status: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> crate::provider::CodexResetCredit {
        crate::provider::CodexResetCredit {
            status: status.to_string(),
            granted_at: None,
            expires_at,
        }
    }

    #[test]
    fn missing_codex_reset_response_carries_only_unexpired_available_credits() {
        let now = fixed_now();
        let resets = crate::provider::CodexResetCredits {
            available_count: 3,
            credits: vec![
                reset_credit("available", Some(now + chrono::Duration::days(2))),
                reset_credit("available", Some(now - chrono::Duration::minutes(1))),
                reset_credit("redeemed", Some(now + chrono::Duration::days(3))),
            ],
        };
        let mut historical = usage("codex", 10.0, None);
        assert!(crate::provider::codex::attach_reset_credits(
            &mut historical,
            &resets
        ));
        let history = UsageHistory {
            entries: vec![UsageHistoryEntry {
                fetched_at: now - chrono::Duration::minutes(5),
                providers: vec![historical],
            }],
            ..UsageHistory::default()
        };
        let mut current = vec![usage("codex", 20.0, None)];

        assert!(history.carry_forward_codex_reset_credits(&mut current, now));
        let carried = crate::provider::codex::reset_credits(&current[0]).unwrap();
        assert_eq!(carried.available_count, 1);
        assert_eq!(carried.credits.len(), 1);
        assert_eq!(
            carried.credits[0].expires_at,
            Some(now + chrono::Duration::days(2))
        );
    }

    #[test]
    fn current_empty_codex_reset_response_is_not_replaced_with_history() {
        let now = fixed_now();
        let historical_resets = crate::provider::CodexResetCredits {
            available_count: 1,
            credits: vec![reset_credit(
                "available",
                Some(now + chrono::Duration::days(2)),
            )],
        };
        let mut historical = usage("codex", 10.0, None);
        crate::provider::codex::attach_reset_credits(&mut historical, &historical_resets);
        let history = UsageHistory {
            entries: vec![UsageHistoryEntry {
                fetched_at: now - chrono::Duration::minutes(5),
                providers: vec![historical],
            }],
            ..UsageHistory::default()
        };
        let mut current = usage("codex", 20.0, None);
        crate::provider::codex::attach_reset_credits(
            &mut current,
            &crate::provider::CodexResetCredits {
                available_count: 0,
                credits: Vec::new(),
            },
        );
        let mut current = vec![current];

        assert!(!history.carry_forward_codex_reset_credits(&mut current, now));
        assert_eq!(
            crate::provider::codex::reset_credits(&current[0])
                .unwrap()
                .available_count,
            0
        );
    }

    #[test]
    fn semantic_window_keys_join_aliases_and_keep_weekly_variants_separate() {
        let codex_session = usage_window_key("codex", &window("Session", 10.0)).unwrap();
        let codex_five_hour = usage_window_key("codex", &window("Session (5h)", 20.0)).unwrap();
        let claude_weekly = usage_window_key("claude", &window("Weekly", 10.0)).unwrap();
        let claude_seven_day = usage_window_key("claude", &window("seven_day", 20.0)).unwrap();
        let sonnet = usage_window_key("claude", &window("Weekly (Sonnet)", 30.0)).unwrap();
        let opus = usage_window_key("claude", &window("seven_day_opus", 40.0)).unwrap();
        let opencodego_rolling =
            usage_window_key("opencodego", &window("Rolling Usage", 50.0)).unwrap();

        assert_eq!(codex_session.as_str(), "quota:5h");
        assert_eq!(codex_session, codex_five_hour);
        assert_eq!(claude_weekly.as_str(), "quota:weekly");
        assert_eq!(claude_weekly, claude_seven_day);
        assert_eq!(sonnet.as_str(), "quota:weekly:sonnet");
        assert_eq!(opus.as_str(), "quota:weekly:opus");
        assert_ne!(sonnet, opus);
        assert_eq!(opencodego_rolling.as_str(), "quota:5h");
    }

    #[test]
    fn auxiliary_amount_windows_are_not_trended_as_zero_percent_quotas() {
        let now = fixed_now();
        let current = usage_with_windows(
            "claude",
            now,
            vec![
                window("Weekly", 25.0),
                UsageWindow {
                    label: "Token Usage (local stats)".to_string(),
                    used_percent: 0.0,
                    limit: None,
                    used: Some(42_000.0),
                    unit: Some("tokens".to_string()),
                    resets_at: None,
                },
                UsageWindow {
                    label: "Last Full Day Spend".to_string(),
                    used_percent: 0.0,
                    limit: None,
                    used: Some(1.25),
                    unit: Some("USD".to_string()),
                    resets_at: None,
                },
            ],
        );
        let history = UsageHistory {
            entries: vec![UsageHistoryEntry {
                fetched_at: now,
                providers: vec![current.clone()],
            }],
            ..UsageHistory::default()
        };

        let trends = history.trends_for_at_with_budget(&current, 7, now, None);

        assert_eq!(trends.len(), 1);
        assert_eq!(trends[0].key.as_str(), "quota:weekly");
    }

    #[test]
    fn per_window_trends_preserve_current_order_and_missing_window_gaps() {
        let now = fixed_now();
        let oldest_at = now - chrono::Duration::days(2);
        let middle_at = now - chrono::Duration::days(1);
        let current = usage_with_windows(
            "opencode",
            now,
            vec![
                window("Monthly Usage", 90.0),
                window("Rolling Usage", 70.0),
                window("Weekly Usage", 80.0),
            ],
        );
        let history = UsageHistory {
            entries: vec![
                UsageHistoryEntry {
                    fetched_at: oldest_at,
                    providers: vec![usage_with_windows(
                        "opencode",
                        oldest_at,
                        vec![
                            window("Rolling Usage", 10.0),
                            window("Weekly Usage", 20.0),
                            window("Monthly Usage", 30.0),
                        ],
                    )],
                },
                UsageHistoryEntry {
                    fetched_at: middle_at,
                    providers: vec![usage_with_windows(
                        "opencode",
                        middle_at,
                        vec![window("Rolling Usage", 40.0), window("Monthly Usage", 50.0)],
                    )],
                },
                UsageHistoryEntry {
                    fetched_at: now,
                    providers: vec![current.clone()],
                },
            ],
            ..UsageHistory::default()
        };

        let trends = history.trends_for_at_with_budget(&current, 7, now, None);

        assert_eq!(
            trends
                .iter()
                .map(|window| window.key.as_str())
                .collect::<Vec<_>>(),
            vec!["quota:monthly", "quota:5h", "quota:weekly"]
        );
        assert_eq!(trends[0].trend.average_percent, (30.0 + 50.0 + 90.0) / 3.0);
        assert_eq!(trends[1].trend.average_percent, 40.0);
        assert_eq!(
            trends[2]
                .trend
                .observations
                .iter()
                .map(|observation| observation.used_percent)
                .collect::<Vec<_>>(),
            vec![Some(20.0), None, Some(80.0)]
        );
        assert_eq!(trends[2].trend.samples, 2);
        assert_eq!(trends[2].trend.peak_percent, 80.0);
    }

    #[test]
    fn budget_generation_changes_do_not_truncate_quota_windows() {
        let now = fixed_now();
        let oldest_at = now - chrono::Duration::hours(2);
        let middle_at = now - chrono::Duration::hours(1);
        let current = usage_with_windows(
            "claude",
            now,
            vec![
                window("Session (5h)", 30.0),
                budget_window("30d Spend", 40.0, 50.0),
            ],
        );
        let history = UsageHistory {
            entries: vec![
                UsageHistoryEntry {
                    fetched_at: oldest_at,
                    providers: vec![usage_with_windows(
                        "claude",
                        oldest_at,
                        vec![
                            window("five_hour", 10.0),
                            budget_window("30d Spend", 20.0, 100.0),
                        ],
                    )],
                },
                UsageHistoryEntry {
                    fetched_at: middle_at,
                    providers: vec![usage_with_windows(
                        "claude",
                        middle_at,
                        vec![
                            window("Session", 20.0),
                            budget_window("30d Spend", 30.0, 50.0),
                        ],
                    )],
                },
                UsageHistoryEntry {
                    fetched_at: now,
                    providers: vec![current.clone()],
                },
            ],
            ..UsageHistory::default()
        };

        let trends = history.trends_for_at_with_budget(&current, 7, now, Some(50.0));

        assert_eq!(trends.len(), 2);
        assert_eq!(trends[0].key.as_str(), "quota:5h");
        assert_eq!(trends[0].trend.samples, 3);
        assert_eq!(trends[0].trend.average_percent, 20.0);
        assert_eq!(trends[1].key.as_str(), "budget:30d:usd");
        assert_eq!(trends[1].trend.samples, 2);
        assert_eq!(trends[1].trend.average_percent, 35.0);
    }

    #[test]
    fn invalid_budget_snapshot_is_a_gap_instead_of_zero_percent_usage() {
        let now = fixed_now();
        let oldest_at = now - chrono::Duration::hours(2);
        let middle_at = now - chrono::Duration::hours(1);
        let current =
            usage_with_windows("claude", now, vec![budget_window("30d Spend", 60.0, 50.0)]);
        let mut unavailable_budget = budget_window("30d Spend", 0.0, 50.0);
        unavailable_budget.limit = None;
        let history = UsageHistory {
            entries: vec![
                UsageHistoryEntry {
                    fetched_at: oldest_at,
                    providers: vec![usage_with_windows(
                        "claude",
                        oldest_at,
                        vec![budget_window("30d Spend", 20.0, 50.0)],
                    )],
                },
                UsageHistoryEntry {
                    fetched_at: middle_at,
                    providers: vec![usage_with_windows(
                        "claude",
                        middle_at,
                        vec![unavailable_budget],
                    )],
                },
                UsageHistoryEntry {
                    fetched_at: now,
                    providers: vec![current.clone()],
                },
            ],
            ..UsageHistory::default()
        };

        let trends = history.trends_for_at_with_budget(&current, 7, now, Some(50.0));

        assert_eq!(trends.len(), 1);
        assert_eq!(
            trends[0]
                .trend
                .observations
                .iter()
                .map(|observation| observation.used_percent)
                .collect::<Vec<_>>(),
            vec![Some(20.0), None, Some(60.0)]
        );
    }

    #[test]
    fn old_history_json_without_subscription_tier_still_builds_window_trends() {
        let now = fixed_now();
        let json = format!(
            r#"{{
                "entries": [{{
                    "fetched_at": "{now}",
                    "providers": [{{
                        "provider": "codex",
                        "windows": [{{
                            "label": "Session",
                            "used_percent": 25.0,
                            "limit": null,
                            "used": null,
                            "unit": null,
                            "resets_at": null
                        }}],
                        "credits": null,
                        "fetched_at": "{now}",
                        "error": null
                    }}]
                }}]
            }}"#
        );
        let history: UsageHistory = serde_json::from_str(&json).unwrap();
        let current = &history.entries[0].providers[0];

        let trends = history.trends_for_at_with_budget(current, 7, now, None);

        assert_eq!(current.subscription_tier, None);
        assert_eq!(trends.len(), 1);
        assert_eq!(trends[0].key.as_str(), "quota:5h");
        assert_eq!(trends[0].trend.latest_percent, 25.0);
    }

    #[test]
    fn latest_successful_skips_error_only_snapshots() {
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now(),
            providers: vec![usage("openai", 20.0, None)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now(),
            providers: vec![usage("openai", 0.0, Some("failed"))],
        });

        let latest = history.latest_successful();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].max_used_percent(), 20.0);
    }

    #[test]
    fn append_ignores_empty_provider_snapshots() {
        let mut history = UsageHistory::default();

        history.append(Vec::new());

        assert!(history.entries.is_empty());
    }

    #[test]
    fn histogram_uses_fixed_time_buckets_and_preserves_gaps() {
        let now = fixed_now();
        let cutoff = now - chrono::Duration::days(7);
        let mut history = UsageHistory::default();
        for (fetched_at, percent) in [
            (cutoff - chrono::Duration::seconds(1), 99.0),
            (cutoff, 10.0),
            (cutoff + chrono::Duration::days(1), 20.0),
            (
                cutoff + chrono::Duration::days(1) + chrono::Duration::hours(1),
                40.0,
            ),
            (now, 70.0),
            (now + chrono::Duration::seconds(1), 100.0),
        ] {
            history.entries.push(UsageHistoryEntry {
                fetched_at,
                providers: vec![usage("openai", percent, None)],
            });
        }

        let current = history
            .entries
            .iter()
            .find(|entry| entry.fetched_at == now)
            .and_then(|entry| entry.providers.first())
            .unwrap();
        let trends = history.trends_for_at_with_budget(current, 7, now, None);
        let trend = &trends[0].trend;
        let buckets = trend.histogram_buckets(7);

        assert_eq!(trend.samples, 4);
        assert_eq!(buckets.len(), 7);
        assert_eq!(buckets[0].average_percent, Some(10.0));
        assert_eq!(buckets[0].samples, 1);
        assert_eq!(buckets[1].average_percent, Some(30.0));
        assert_eq!(buckets[1].latest_percent, Some(40.0));
        assert_eq!(buckets[1].peak_percent, Some(40.0));
        assert_eq!(buckets[1].samples, 2);
        assert!(
            buckets[2..6]
                .iter()
                .all(|bucket| bucket.latest_percent.is_none()
                    && bucket.average_percent.is_none()
                    && bucket.samples == 0)
        );
        assert_eq!(buckets[6].average_percent, Some(70.0));
        assert_eq!(buckets[6].samples, 1);
        assert_eq!(buckets[0].starts_at, cutoff);
        assert_eq!(buckets[6].ends_at, now);
    }

    #[test]
    fn histogram_zero_bucket_count_is_empty() {
        let now = fixed_now();
        let trend = ProviderTrend {
            samples: 1,
            latest_percent: 25.0,
            previous_percent: None,
            peak_percent: 25.0,
            average_percent: 25.0,
            period_start: now - chrono::Duration::days(7),
            period_end: now,
            observations: vec![ProviderTrendObservation {
                fetched_at: now,
                used_percent: Some(25.0),
            }],
        };

        assert!(trend.histogram_buckets(0).is_empty());
    }

    #[test]
    fn histogram_latest_gap_is_not_reported_as_zero_usage() {
        let bucket = ProviderTrendBucket {
            starts_at: fixed_now() - chrono::Duration::days(1),
            ends_at: fixed_now(),
            latest_percent: None,
            average_percent: Some(20.0),
            peak_percent: Some(20.0),
            samples: 1,
        };

        assert_eq!(
            classify_histogram_buckets(&[bucket]),
            TrendHistogramState::LatestUnavailable
        );
    }

    #[test]
    fn histogram_populated_zero_buckets_have_a_distinct_empty_state() {
        let now = fixed_now();
        let buckets = [0.0, 0.04].map(|latest_percent| ProviderTrendBucket {
            starts_at: now - chrono::Duration::days(1),
            ends_at: now,
            latest_percent: Some(latest_percent),
            average_percent: Some(latest_percent),
            peak_percent: Some(latest_percent),
            samples: 1,
        });

        assert_eq!(
            classify_histogram_buckets(&buckets),
            TrendHistogramState::AllRoundedToZero
        );
    }

    #[test]
    fn histogram_nonzero_latest_sample_renders_the_chart() {
        let now = fixed_now();
        let bucket = ProviderTrendBucket {
            starts_at: now - chrono::Duration::days(1),
            ends_at: now,
            latest_percent: Some(20.0),
            average_percent: Some(20.0),
            peak_percent: Some(20.0),
            samples: 1,
        };

        assert_eq!(
            classify_histogram_buckets(&[bucket]),
            TrendHistogramState::Chart
        );
    }

    #[test]
    fn trend_does_not_cross_a_missing_provider_snapshot() {
        let now = fixed_now();
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::days(3),
            providers: vec![usage("openai", 10.0, None)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::days(2),
            providers: vec![usage("gemini", 70.0, None)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::days(1),
            providers: vec![usage("openai", 0.0, Some("failed"))],
        });
        let current = usage("openai", 30.0, None);
        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![current.clone()],
        });

        let trends = history.trends_for_at_with_budget(&current, 7, now, None);
        let trend = &trends[0].trend;
        assert_eq!(trend.samples, 1);
        assert_eq!(trend.latest_percent, 30.0);
        assert_eq!(trend.observations[0].fetched_at, now);
    }

    #[test]
    fn trend_crosses_a_global_empty_snapshot() {
        let now = fixed_now();
        let oldest_at = now - chrono::Duration::days(2);
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: oldest_at,
            providers: vec![usage("openai", 10.0, None)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::days(1),
            providers: Vec::new(),
        });
        let current = usage("openai", 30.0, None);
        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![current.clone()],
        });

        let trends = history.trends_for_at_with_budget(&current, 7, now, None);
        let trend = &trends[0].trend;

        assert_eq!(trend.samples, 2);
        assert_eq!(trend.average_percent, 20.0);
        assert_eq!(trend.observations[0].fetched_at, oldest_at);
        assert_eq!(trend.observations[1].fetched_at, now);
    }

    #[test]
    fn partial_budget_failure_does_not_cross_a_missing_provider_snapshot() {
        let now = fixed_now();
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::hours(2),
            providers: vec![budget_usage(20.0, 50.0)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::hours(1),
            providers: vec![usage("gemini", 70.0, None)],
        });
        let current = usage("openai", 90.0, None);
        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![current.clone()],
        });

        let trends = history.trends_for_at_with_budget(&current, 7, now, Some(50.0));
        assert!(
            trends
                .iter()
                .all(|trend| trend.key.as_str() != "budget:30d:usd")
        );
    }

    #[test]
    fn prune_keeps_the_latest_2500_entries() {
        let now = Utc::now();
        let mut history = UsageHistory::default();
        for index in 0..2601 {
            history.entries.push(UsageHistoryEntry {
                fetched_at: now - chrono::Duration::seconds((2600 - index) as i64),
                providers: Vec::new(),
            });
        }

        history.prune(30, 2500);

        assert_eq!(history.entries.len(), 2500);
        assert_eq!(
            history.entries.first().unwrap().fetched_at,
            now - chrono::Duration::seconds(2499)
        );
        assert_eq!(history.entries.last().unwrap().fetched_at, now);
    }

    #[test]
    fn latest_successful_for_finds_correct_provider() {
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now() - chrono::Duration::days(2),
            providers: vec![usage("openai", 20.0, None)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now() - chrono::Duration::days(1),
            providers: vec![usage("openai", 0.0, Some("failed"))],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now(),
            providers: vec![usage("gemini", 10.0, None)],
        });

        let openai_cached = history.latest_successful_for("openai").unwrap();
        assert_eq!(openai_cached.max_used_percent(), 20.0);

        let gemini_cached = history.latest_successful_for("gemini").unwrap();
        assert_eq!(gemini_cached.max_used_percent(), 10.0);

        let non_existent = history.latest_successful_for("claude");
        assert!(non_existent.is_none());
    }
}
