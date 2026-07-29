use crate::usage_history::{ProviderTrend, ProviderTrendBucket, UsageHistory};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug)]
pub struct CachedProviderTrend {
    pub trend: ProviderTrend,
    pub histogram_buckets: Vec<ProviderTrendBucket>,
}

impl Deref for CachedProviderTrend {
    type Target = ProviderTrend;

    fn deref(&self) -> &Self::Target {
        &self.trend
    }
}

/// Caches provider trends for one minute-wide presentation epoch.
///
/// `UsageHistory` is append-only at runtime. Its length plus first and last
/// timestamps therefore form a cheap fingerprint that still changes when the
/// history reaches its capped length and an append prunes its oldest entry.
#[derive(Debug, Default)]
pub struct TrendCache {
    epoch: Option<CacheEpoch>,
    entries: HashMap<TrendCacheKey, Option<Arc<CachedProviderTrend>>>,
}

#[derive(Debug, Clone, Copy)]
struct CacheEpoch {
    history: HistoryFingerprint,
    minute: i64,
    evaluated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HistoryFingerprint {
    len: usize,
    first_fetched_at: Option<DateTime<Utc>>,
    last_fetched_at: Option<DateTime<Utc>>,
}

impl HistoryFingerprint {
    fn of(history: &UsageHistory) -> Self {
        Self {
            len: history.entries.len(),
            first_fetched_at: history.entries.first().map(|entry| entry.fetched_at),
            last_fetched_at: history.entries.last().map(|entry| entry.fetched_at),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TrendCacheKey {
    provider: String,
    days: i64,
    configured_budget_bits: Option<u64>,
}

impl TrendCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached trend, computing it once when the history, minute,
    /// provider, budget, or requested time window changes.
    ///
    /// The returned `Arc` is cheap to clone during animation frames. Both the
    /// raw trend and its histogram buckets are computed once per epoch; `None`
    /// is cached as well.
    pub fn get_or_compute(
        &mut self,
        history: &UsageHistory,
        provider: &str,
        days: i64,
        now: DateTime<Utc>,
        configured_budget: Option<f64>,
    ) -> Option<Arc<CachedProviderTrend>> {
        let history_fingerprint = HistoryFingerprint::of(history);
        let minute = now.timestamp().div_euclid(60);
        if !self
            .epoch
            .is_some_and(|epoch| epoch.matches(history_fingerprint, minute))
        {
            self.entries.clear();
            self.epoch = Some(CacheEpoch {
                history: history_fingerprint,
                minute,
                evaluated_at: now,
            });
        }
        let evaluated_at = self
            .epoch
            .expect("the cache epoch is initialized above")
            .evaluated_at;

        let provider = provider.trim().to_ascii_lowercase();
        let key = TrendCacheKey {
            provider: provider.clone(),
            days,
            configured_budget_bits: configured_budget.map(f64::to_bits),
        };

        self.entries
            .entry(key)
            .or_insert_with(|| {
                history
                    .trend_for_at_with_budget(&provider, days, evaluated_at, configured_budget)
                    .map(|trend| {
                        let bucket_count = usize::try_from(days).unwrap_or_default();
                        let histogram_buckets = trend.histogram_buckets(bucket_count);
                        Arc::new(CachedProviderTrend {
                            trend,
                            histogram_buckets,
                        })
                    })
            })
            .clone()
    }

    #[cfg(test)]
    pub fn clear(&mut self) {
        self.epoch = None;
        self.entries.clear();
    }
}

impl CacheEpoch {
    fn matches(self, history: HistoryFingerprint, minute: i64) -> bool {
        self.history == history && self.minute == minute
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{UsageData, UsageWindow};
    use crate::usage_history::UsageHistoryEntry;
    use chrono::{Duration, TimeZone};

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 29, 12, 34, 45)
            .single()
            .unwrap()
    }

    fn quota_usage(provider: &str, fetched_at: DateTime<Utc>, percent: f64) -> UsageData {
        UsageData {
            provider: provider.to_string(),
            windows: vec![UsageWindow {
                label: "Weekly".to_string(),
                used_percent: percent,
                limit: None,
                used: None,
                unit: None,
                resets_at: None,
            }],
            credits: None,
            fetched_at,
            error: None,
        }
    }

    fn budget_usage(fetched_at: DateTime<Utc>, percent: f64, budget: f64) -> UsageData {
        UsageData {
            provider: "openai".to_string(),
            windows: vec![UsageWindow {
                label: "Cost (30d)".to_string(),
                used_percent: percent,
                limit: Some(budget),
                used: Some(percent / 100.0 * budget),
                unit: Some("USD".to_string()),
                resets_at: None,
            }],
            credits: None,
            fetched_at,
            error: None,
        }
    }

    fn entry(fetched_at: DateTime<Utc>, providers: Vec<UsageData>) -> UsageHistoryEntry {
        UsageHistoryEntry {
            fetched_at,
            providers,
        }
    }

    #[test]
    fn reuses_provider_entry_within_the_same_minute() {
        let now = fixed_now();
        let history = UsageHistory {
            entries: vec![entry(
                now - Duration::minutes(1),
                vec![quota_usage("gemini", now - Duration::minutes(1), 25.0)],
            )],
            ..UsageHistory::default()
        };
        let mut cache = TrendCache::new();

        let first = cache
            .get_or_compute(&history, " Gemini ", 7, now, None)
            .unwrap();
        let second = cache
            .get_or_compute(&history, "gemini", 7, now + Duration::seconds(14), None)
            .unwrap();

        assert_eq!(cache.entries.len(), 1);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.period_end, second.period_end);
        assert_eq!(first.period_end, now);
        assert_eq!(first.histogram_buckets.len(), 7);
    }

    #[test]
    fn caches_a_missing_trend() {
        let history = UsageHistory::default();
        let now = fixed_now();
        let mut cache = TrendCache::new();

        assert!(
            cache
                .get_or_compute(&history, "gemini", 7, now, None)
                .is_none()
        );
        assert!(
            cache
                .get_or_compute(&history, "GEMINI", 7, now, None)
                .is_none()
        );
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn history_append_invalidates_within_the_same_minute() {
        let now = fixed_now();
        let first_at = now - Duration::minutes(2);
        // This snapshot is newer than the start of the current minute. A
        // cache epoch evaluated at the minute boundary would incorrectly
        // classify it as a future entry.
        let second_at = now - Duration::seconds(15);
        let mut history = UsageHistory {
            entries: vec![entry(first_at, vec![quota_usage("gemini", first_at, 20.0)])],
            ..UsageHistory::default()
        };
        let mut cache = TrendCache::new();

        assert_eq!(
            cache
                .get_or_compute(&history, "gemini", 7, now, None)
                .as_ref()
                .unwrap()
                .samples,
            1
        );

        history.entries.push(entry(
            second_at,
            vec![quota_usage("gemini", second_at, 40.0)],
        ));

        let refreshed = cache
            .get_or_compute(&history, "gemini", 7, now, None)
            .unwrap();
        assert_eq!(refreshed.samples, 2);
        assert_eq!(refreshed.latest_percent, 40.0);
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn changed_tail_invalidates_even_when_history_length_is_unchanged() {
        let now = fixed_now();
        let first_at = now - Duration::minutes(3);
        let old_tail_at = now - Duration::minutes(2);
        let new_tail_at = now - Duration::minutes(1);
        let mut history = UsageHistory {
            entries: vec![
                entry(first_at, vec![quota_usage("gemini", first_at, 10.0)]),
                entry(old_tail_at, vec![quota_usage("gemini", old_tail_at, 20.0)]),
            ],
            ..UsageHistory::default()
        };
        let mut cache = TrendCache::new();

        assert_eq!(
            cache
                .get_or_compute(&history, "gemini", 7, now, None)
                .as_ref()
                .unwrap()
                .latest_percent,
            20.0
        );

        history.entries[1] = entry(new_tail_at, vec![quota_usage("gemini", new_tail_at, 60.0)]);

        assert_eq!(
            cache
                .get_or_compute(&history, "gemini", 7, now, None)
                .as_ref()
                .unwrap()
                .latest_percent,
            60.0
        );
    }

    #[test]
    fn provider_budget_and_window_are_independent_keys() {
        let now = fixed_now();
        let sample_at = now - Duration::minutes(1);
        let history = UsageHistory {
            entries: vec![entry(
                sample_at,
                vec![
                    quota_usage("gemini", sample_at, 30.0),
                    budget_usage(sample_at, 40.0, 50.0),
                ],
            )],
            ..UsageHistory::default()
        };
        let mut cache = TrendCache::new();

        assert!(
            cache
                .get_or_compute(&history, "openai", 7, now, Some(50.0))
                .is_some()
        );
        assert!(
            cache
                .get_or_compute(&history, "openai", 7, now, Some(100.0))
                .is_none()
        );
        assert!(
            cache
                .get_or_compute(&history, "gemini", 7, now, None)
                .is_some()
        );
        assert!(
            cache
                .get_or_compute(&history, "gemini", 30, now, None)
                .is_some()
        );
        assert_eq!(cache.entries.len(), 4);
    }

    #[test]
    fn crossing_a_minute_boundary_starts_a_new_epoch() {
        let now = fixed_now();
        let sample_at = now - Duration::minutes(1);
        let history = UsageHistory {
            entries: vec![entry(
                sample_at,
                vec![quota_usage("gemini", sample_at, 25.0)],
            )],
            ..UsageHistory::default()
        };
        let mut cache = TrendCache::new();

        let old_period_end = cache
            .get_or_compute(&history, "gemini", 7, now, None)
            .as_ref()
            .unwrap()
            .period_end;
        let next_minute = now + Duration::seconds(15);
        let new_period_end = cache
            .get_or_compute(&history, "gemini", 7, next_minute, None)
            .as_ref()
            .unwrap()
            .period_end;

        assert_eq!(old_period_end, now);
        assert_eq!(new_period_end, next_minute);
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn clear_discards_the_epoch_and_all_entries() {
        let now = fixed_now();
        let history = UsageHistory::default();
        let mut cache = TrendCache::new();
        cache.get_or_compute(&history, "gemini", 7, now, None);

        cache.clear();

        assert!(cache.epoch.is_none());
        assert!(cache.entries.is_empty());
    }
}
