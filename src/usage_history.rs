use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageHistory {
    pub entries: Vec<UsageHistoryEntry>,
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
        let samples: Vec<f64> = self
            .entries
            .iter()
            .filter(|entry| entry.fetched_at >= cutoff)
            .filter_map(|entry| {
                entry
                    .providers
                    .iter()
                    .find(|data| {
                        data.provider.eq_ignore_ascii_case(provider) && data.error.is_none()
                    })
                    .map(|data| data.max_used_percent())
            })
            .collect();

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
