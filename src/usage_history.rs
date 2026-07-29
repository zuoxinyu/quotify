use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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
        self.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now(),
            providers,
        });
        // Keep enough raw snapshots for a complete seven-day trend at the
        // default five-minute refresh interval, with room for manual refreshes.
        self.prune(30, 2500);
    }

    pub fn latest_successful(&self) -> Vec<crate::provider::UsageData> {
        self.entries
            .iter()
            .rev()
            .find(|entry| {
                entry
                    .providers
                    .iter()
                    .any(|provider| provider.error.is_none())
            })
            .map(|entry| entry.providers.clone())
            .unwrap_or_default()
    }

    pub fn trend_for_with_budget(
        &self,
        provider: &str,
        days: i64,
        configured_budget: Option<f64>,
    ) -> Option<ProviderTrend> {
        self.trend_for_at_with_budget(provider, days, Utc::now(), configured_budget)
    }

    pub fn trend_for_at_with_budget(
        &self,
        provider: &str,
        days: i64,
        now: DateTime<Utc>,
        configured_budget: Option<f64>,
    ) -> Option<ProviderTrend> {
        if days <= 0 {
            return None;
        }

        let cutoff = now - chrono::Duration::days(days);
        let configured_budget = configured_budget.filter(|budget| {
            crate::provider::supports_api_budget(provider) && budget.is_finite() && *budget > 0.0
        });
        let mut points = Vec::new();
        let mut observations = Vec::new();

        for entry in self.entries.iter().rev() {
            if entry.fetched_at > now {
                continue;
            }
            if entry.fetched_at < cutoff {
                break;
            }
            let Some(data) = entry
                .providers
                .iter()
                .find(|data| data.provider.eq_ignore_ascii_case(provider))
            else {
                if !observations.is_empty() {
                    break;
                }
                continue;
            };
            if data.error.is_some() {
                continue;
            }

            let observed_percent = if let Some(configured_budget) = configured_budget {
                let Some(window) = data.windows.iter().find(|window| {
                    crate::provider::is_budget_spend_window(
                        &data.provider,
                        &window.label,
                        window.unit.as_deref(),
                    )
                }) else {
                    // Core quota data can still be available when the cost API
                    // fails. Preserve that time gap instead of substituting an
                    // unrelated quota percentage for the configured budget.
                    observations.push(ProviderTrendObservation {
                        fetched_at: entry.fetched_at,
                        used_percent: None,
                    });
                    continue;
                };
                let Some(window_limit) = window
                    .limit
                    .filter(|limit| limit.is_finite() && *limit > 0.0)
                else {
                    observations.push(ProviderTrendObservation {
                        fetched_at: entry.fetched_at,
                        used_percent: None,
                    });
                    continue;
                };
                if !same_optional_limit(Some(configured_budget), Some(window_limit)) {
                    break;
                }
                window
                    .used_percent
                    .is_finite()
                    .then_some(window.used_percent)
            } else if crate::provider::supports_api_budget(provider) {
                let old_budget_still_present = data.windows.iter().any(|window| {
                    crate::provider::is_budget_spend_window(
                        &data.provider,
                        &window.label,
                        window.unit.as_deref(),
                    ) && window
                        .limit
                        .is_some_and(|limit| limit.is_finite() && limit > 0.0)
                });
                if old_budget_still_present {
                    break;
                }

                let Some(non_budget_percent) = data
                    .windows
                    .iter()
                    .filter(|window| {
                        !crate::provider::is_budget_spend_window(
                            &data.provider,
                            &window.label,
                            window.unit.as_deref(),
                        )
                    })
                    .map(|window| window.used_percent)
                    .filter(|percent| percent.is_finite())
                    .reduce(f64::max)
                else {
                    observations.push(ProviderTrendObservation {
                        fetched_at: entry.fetched_at,
                        used_percent: None,
                    });
                    continue;
                };
                Some(non_budget_percent)
            } else {
                let used_percent = data.max_used_percent();
                used_percent.is_finite().then_some(used_percent)
            };
            observations.push(ProviderTrendObservation {
                fetched_at: entry.fetched_at,
                used_percent: observed_percent,
            });
            if let Some(used_percent) = observed_percent {
                points.push((entry.fetched_at, used_percent));
            }
        }
        points.reverse();
        observations.reverse();

        if points.is_empty() {
            return None;
        }

        let latest_percent = points.last().map_or(0.0, |(_, percent)| *percent);
        let previous_percent = points.iter().rev().nth(1).map(|(_, percent)| *percent);
        let peak_percent = points
            .iter()
            .map(|(_, percent)| *percent)
            .fold(0.0, f64::max);
        let average_percent =
            points.iter().map(|(_, percent)| *percent).sum::<f64>() / points.len() as f64;

        Some(ProviderTrend {
            samples: points.len(),
            latest_percent,
            previous_percent,
            peak_percent,
            average_percent,
            period_start: cutoff,
            period_end: now,
            observations,
        })
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
            fetched_at: Utc::now(),
            error: None,
        }
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
    fn trend_for_computes_recent_samples() {
        let now = fixed_now();
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::days(1),
            providers: vec![usage("openai", 20.0, None)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![usage("openai", 50.0, None)],
        });

        let trend = history
            .trend_for_at_with_budget("openai", 7, now, None)
            .unwrap();
        assert_eq!(trend.samples, 2);
        assert_eq!(trend.latest_percent, 50.0);
        assert_eq!(trend.previous_percent, Some(20.0));
        assert_eq!(trend.peak_percent, 50.0);
        assert_eq!(trend.average_percent, 35.0);
        assert_eq!(
            trend.observations,
            vec![
                ProviderTrendObservation {
                    fetched_at: now - chrono::Duration::days(1),
                    used_percent: Some(20.0),
                },
                ProviderTrendObservation {
                    fetched_at: now,
                    used_percent: Some(50.0),
                },
            ]
        );
    }

    #[test]
    fn trend_starts_a_new_generation_when_budget_changes() {
        let now = fixed_now();
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::hours(2),
            providers: vec![budget_usage(20.0, 100.0)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::hours(1),
            providers: vec![budget_usage(30.0, 100.0)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![budget_usage(60.0, 50.0)],
        });

        let trend = history
            .trend_for_at_with_budget("openai", 7, now, Some(50.0))
            .unwrap();
        assert_eq!(trend.samples, 1);
        assert_eq!(trend.latest_percent, 60.0);
        assert_eq!(trend.previous_percent, None);
        assert_eq!(trend.average_percent, 60.0);
        assert_eq!(trend.observations[0].fetched_at, now);
    }

    #[test]
    fn budget_trend_preserves_partial_cost_failure_as_a_gap() {
        let now = fixed_now();
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::days(2),
            providers: vec![budget_usage(20.0, 50.0)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::days(1),
            providers: vec![usage("openai", 90.0, None)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![budget_usage(60.0, 50.0)],
        });

        let trend = history
            .trend_for_at_with_budget("openai", 7, now, Some(50.0))
            .unwrap();
        assert_eq!(trend.samples, 2);
        assert_eq!(
            trend
                .observations
                .iter()
                .map(|observation| observation.used_percent)
                .collect::<Vec<_>>(),
            vec![Some(20.0), None, Some(60.0)]
        );
    }

    #[test]
    fn budget_failure_as_latest_observation_empties_bucket() {
        let now = fixed_now();
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::hours(2),
            providers: vec![budget_usage(20.0, 50.0)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::hours(1),
            providers: vec![usage("openai", 90.0, None)],
        });

        let trend = history
            .trend_for_at_with_budget("openai", 7, now, Some(50.0))
            .unwrap();
        let buckets = trend.histogram_buckets(7);
        let latest_bucket = buckets.last().unwrap();

        assert_eq!(trend.samples, 1);
        assert_eq!(latest_bucket.latest_percent, None);
        assert_eq!(latest_bucket.average_percent, Some(20.0));
        assert_eq!(latest_bucket.samples, 1);
    }

    #[test]
    fn cleared_budget_does_not_reuse_old_budget_history() {
        let now = fixed_now();
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::days(1),
            providers: vec![budget_usage(80.0, 100.0)],
        });

        assert!(
            history
                .trend_for_at_with_budget("openai", 7, now, None)
                .is_none()
        );

        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![usage("openai", 30.0, None)],
        });
        let trend = history
            .trend_for_at_with_budget("openai", 7, now, None)
            .unwrap();
        assert_eq!(trend.samples, 1);
        assert_eq!(trend.latest_percent, 30.0);
        assert_eq!(trend.observations[0].fetched_at, now);
    }

    #[test]
    fn effective_budget_limit_selects_the_matching_bedrock_generation() {
        let now = fixed_now();
        let mut old_budget = budget_usage(20.0, 100.0);
        old_budget.provider = "bedrock".to_string();
        let mut current_budget = budget_usage(40.0, 75.0);
        current_budget.provider = "bedrock".to_string();
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: now - chrono::Duration::hours(1),
            providers: vec![old_budget],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![current_budget],
        });

        let trend = history
            .trend_for_at_with_budget("bedrock", 7, now, Some(75.0))
            .unwrap();
        assert_eq!(trend.samples, 1);
        assert_eq!(trend.latest_percent, 40.0);
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

        let trend = history
            .trend_for_at_with_budget("openai", 7, now, None)
            .unwrap();
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
        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![usage("openai", 30.0, None)],
        });

        let trend = history
            .trend_for_at_with_budget("openai", 7, now, None)
            .unwrap();
        assert_eq!(trend.samples, 1);
        assert_eq!(trend.latest_percent, 30.0);
        assert_eq!(trend.observations[0].fetched_at, now);
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
        history.entries.push(UsageHistoryEntry {
            fetched_at: now,
            providers: vec![usage("openai", 90.0, None)],
        });

        assert!(
            history
                .trend_for_at_with_budget("openai", 7, now, Some(50.0))
                .is_none()
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
