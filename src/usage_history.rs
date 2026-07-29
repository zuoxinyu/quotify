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
        self.prune(30, 1000);
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

    pub fn trend_for(&self, provider: &str, days: i64) -> Option<ProviderTrend> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let budgeted_provider = crate::provider::supports_api_budget(provider);
        let mut expected_budget_limit: Option<Option<f64>> = None;
        let mut samples = Vec::new();

        for entry in self.entries.iter().rev() {
            if entry.fetched_at < cutoff {
                break;
            }
            let Some(data) = entry
                .providers
                .iter()
                .find(|data| data.provider.eq_ignore_ascii_case(provider))
            else {
                if expected_budget_limit.is_some() || !samples.is_empty() {
                    break;
                }
                continue;
            };
            if data.error.is_some() {
                continue;
            }

            if budgeted_provider {
                let current_limit = budget_limit(data);
                if let Some(expected_limit) = expected_budget_limit {
                    if !same_optional_limit(expected_limit, current_limit) {
                        break;
                    }
                } else {
                    expected_budget_limit = Some(current_limit);
                }
            }
            samples.push(data.max_used_percent());
        }
        samples.reverse();

        if samples.is_empty() {
            return None;
        }

        let latest_percent = *samples.last().unwrap_or(&0.0);
        let previous_percent = samples
            .iter()
            .rev()
            .nth(1)
            .copied()
            .filter(|value| value.is_finite());
        let peak_percent = samples.iter().copied().fold(0.0, f64::max);
        let average_percent = samples.iter().sum::<f64>() / samples.len() as f64;

        Some(ProviderTrend {
            samples: samples.len(),
            latest_percent,
            previous_percent,
            peak_percent,
            average_percent,
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

fn budget_limit(data: &crate::provider::UsageData) -> Option<f64> {
    data.windows
        .iter()
        .find(|window| {
            crate::provider::is_budget_spend_window(
                &data.provider,
                &window.label,
                window.unit.as_deref(),
            )
        })
        .and_then(|window| window.limit)
        .filter(|limit| limit.is_finite() && *limit > 0.0)
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
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now() - chrono::Duration::days(1),
            providers: vec![usage("openai", 20.0, None)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now(),
            providers: vec![usage("openai", 50.0, None)],
        });

        let trend = history.trend_for("openai", 7).unwrap();
        assert_eq!(trend.samples, 2);
        assert_eq!(trend.latest_percent, 50.0);
        assert_eq!(trend.previous_percent, Some(20.0));
        assert_eq!(trend.peak_percent, 50.0);
        assert_eq!(trend.average_percent, 35.0);
    }

    #[test]
    fn trend_starts_a_new_generation_when_budget_changes() {
        let mut history = UsageHistory::default();
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now() - chrono::Duration::hours(2),
            providers: vec![budget_usage(20.0, 100.0)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now() - chrono::Duration::hours(1),
            providers: vec![budget_usage(30.0, 100.0)],
        });
        history.entries.push(UsageHistoryEntry {
            fetched_at: Utc::now(),
            providers: vec![budget_usage(60.0, 50.0)],
        });

        let trend = history.trend_for("openai", 7).unwrap();
        assert_eq!(trend.samples, 1);
        assert_eq!(trend.latest_percent, 60.0);
        assert_eq!(trend.previous_percent, None);
        assert_eq!(trend.average_percent, 60.0);
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
