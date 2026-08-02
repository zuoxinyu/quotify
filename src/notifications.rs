use chrono::{DateTime, Duration, Utc};

use crate::config::NotificationConfig;
use crate::provider::{UsageData, UsageWindow};
use crate::usage_history::UsageHistory;

const RESET_ADVANCE_TOLERANCE: Duration = Duration::seconds(60);
const MAX_NOTIFICATION_BODY_UTF16: usize = 255;
const MAX_NOTIFICATION_LINE_UTF16: usize = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaPeriod {
    FiveHour,
    Weekly,
    Monthly,
}

impl QuotaPeriod {
    fn label(self) -> &'static str {
        match self {
            Self::FiveHour => "5h",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NotificationEvent {
    QuotaReset {
        provider: String,
        window_label: String,
        period: QuotaPeriod,
        next_reset_at: DateTime<Utc>,
    },
    UsageThresholdExceeded {
        provider: String,
        window_label: String,
        used_percent: f64,
        threshold_percent: f64,
    },
    SilentRefreshFailed {
        provider: String,
    },
    BudgetRefreshFailed {
        provider: String,
    },
}

#[derive(Default)]
struct ProviderBaseline<'a> {
    last_observation: Option<&'a UsageData>,
    last_successful: Option<&'a UsageData>,
}

/// Evaluate one complete provider refresh before the new snapshot is appended
/// to `history`.
///
/// History is the persisted edge state: callers must append the current
/// snapshot after this function returns, even when notifications are disabled.
/// That prevents enabling notifications later from replaying old events.
pub fn evaluate(
    settings: &NotificationConfig,
    history: &UsageHistory,
    current: &[UsageData],
    now: DateTime<Utc>,
    silent_refresh: bool,
) -> Vec<NotificationEvent> {
    if !settings.enabled {
        return Vec::new();
    }

    let threshold = valid_threshold(settings);
    let mut events = Vec::new();

    for current_provider in current {
        let baseline = provider_baseline(history, &current_provider.provider);

        if current_provider.error.is_some() {
            if silent_refresh
                && settings.silent_refresh_failures
                && baseline
                    .last_observation
                    .is_none_or(|previous| previous.error.is_none())
            {
                events.push(NotificationEvent::SilentRefreshFailed {
                    provider: current_provider.provider.clone(),
                });
            }
            continue;
        }

        let Some(previous_provider) = baseline.last_successful else {
            continue;
        };

        for current_window in current_provider
            .windows
            .iter()
            .filter(|window| is_notifiable_window(window))
        {
            let Some(previous_window) =
                matching_previous_window(previous_provider, current_provider, current_window)
                    .or_else(|| {
                        latest_budget_spend_window(history, current_provider, current_window)
                    })
            else {
                continue;
            };

            let cadence_period = quota_period_from_reset_cadence(previous_window, current_window);
            if let Some(period) =
                quota_period_for_window(&current_provider.provider, current_window)
                    .or(cadence_period)
                && reset_notifications_enabled(settings, period)
                && quota_period_for_window(&previous_provider.provider, previous_window)
                    .or(cadence_period)
                    == Some(period)
                && reset_happened(previous_window, current_window, now)
            {
                events.push(NotificationEvent::QuotaReset {
                    provider: current_provider.provider.clone(),
                    window_label: current_window.label.clone(),
                    period,
                    next_reset_at: current_window
                        .resets_at
                        .expect("reset_happened requires a current reset timestamp"),
                });
            }

            if let Some(threshold_percent) = threshold
                && previous_window.used_percent.is_finite()
                && previous_window.used_percent < threshold_percent
                && current_window.used_percent >= threshold_percent
                && same_limit_generation(previous_window, current_window)
            {
                events.push(NotificationEvent::UsageThresholdExceeded {
                    provider: current_provider.provider.clone(),
                    window_label: current_window.label.clone(),
                    used_percent: current_window.used_percent,
                    threshold_percent,
                });
            }
        }
    }

    events
}

/// Evaluate the failure edge for configured API budgets separately from the
/// provider's primary quota response. Some providers can still return useful
/// subscription or credit data when their spend endpoint fails.
pub fn evaluate_budget_refresh_failures(
    settings: &NotificationConfig,
    previous_failures: &std::collections::BTreeSet<String>,
    current_failures: &std::collections::BTreeSet<String>,
    silent_refresh: bool,
) -> Vec<NotificationEvent> {
    if !settings.enabled || !settings.silent_refresh_failures || !silent_refresh {
        return Vec::new();
    }

    current_failures
        .iter()
        .filter(|provider| !previous_failures.contains(*provider))
        .map(|provider| NotificationEvent::BudgetRefreshFailed {
            provider: provider.clone(),
        })
        .collect()
}

/// Collapse all events from one refresh into the single title/body pair
/// accepted by the native tray notification backend.
pub fn aggregate(events: &[NotificationEvent]) -> Option<(String, String)> {
    if events.is_empty() {
        return None;
    }

    let has_reset = events
        .iter()
        .any(|event| matches!(event, NotificationEvent::QuotaReset { .. }));
    let has_threshold = events
        .iter()
        .any(|event| matches!(event, NotificationEvent::UsageThresholdExceeded { .. }));
    let has_failure = events
        .iter()
        .any(|event| matches!(event, NotificationEvent::SilentRefreshFailed { .. }));
    let has_budget_failure = events
        .iter()
        .any(|event| matches!(event, NotificationEvent::BudgetRefreshFailed { .. }));

    let title = match (has_reset, has_threshold, has_failure, has_budget_failure) {
        (true, false, false, false) => "Quota reset",
        (false, true, false, false) => "Quota threshold reached",
        (false, false, true, false) => "Refresh failed",
        (false, false, false, true) => "Budget refresh failed",
        _ => "Quotify notifications",
    }
    .to_string();

    let lines = events
        .iter()
        .map(|event| match event {
            NotificationEvent::QuotaReset {
                provider,
                window_label,
                period,
                ..
            } => format!(
                "{}: {} reset ({})",
                display_name(provider),
                single_line(window_label),
                period.label()
            ),
            NotificationEvent::UsageThresholdExceeded {
                provider,
                window_label,
                used_percent,
                threshold_percent,
            } => format!(
                "{}: {} {}% (threshold {}%)",
                display_name(provider),
                single_line(window_label),
                format_percent(*used_percent),
                format_percent(*threshold_percent)
            ),
            NotificationEvent::SilentRefreshFailed { provider } => {
                format!("{}: background refresh failed", display_name(provider))
            }
            NotificationEvent::BudgetRefreshFailed { provider } => {
                format!("{}: 30-day spend data unavailable", display_name(provider))
            }
        })
        .map(|line| truncate_utf16_with_ellipsis(&line, MAX_NOTIFICATION_LINE_UTF16))
        .collect::<Vec<_>>();
    let body = fit_notification_lines(&lines);

    Some((title, body))
}

fn fit_notification_lines(lines: &[String]) -> String {
    let mut included = Vec::new();
    for line in lines {
        let candidate = if included.is_empty() {
            line.clone()
        } else {
            format!("{}\n{line}", included.join("\n"))
        };
        if candidate.encode_utf16().count() > MAX_NOTIFICATION_BODY_UTF16 {
            break;
        }
        included.push(line.clone());
    }

    let mut remaining = lines.len().saturating_sub(included.len());
    while remaining > 0 {
        let summary = format!("… and {remaining} more");
        let candidate = if included.is_empty() {
            summary.clone()
        } else {
            format!("{}\n{summary}", included.join("\n"))
        };
        if candidate.encode_utf16().count() <= MAX_NOTIFICATION_BODY_UTF16 {
            return candidate;
        }
        if included.pop().is_some() {
            remaining += 1;
        } else {
            return truncate_utf16_with_ellipsis(&summary, MAX_NOTIFICATION_BODY_UTF16);
        }
    }

    included.join("\n")
}

fn truncate_utf16_with_ellipsis(value: &str, max_units: usize) -> String {
    if value.encode_utf16().count() <= max_units {
        return value.to_string();
    }
    if max_units == 0 {
        return String::new();
    }

    let content_limit = max_units - 1;
    let mut output = String::new();
    let mut units = 0;
    for character in value.chars() {
        let character_units = character.len_utf16();
        if units + character_units > content_limit {
            break;
        }
        output.push(character);
        units += character_units;
    }
    output.push('…');
    output
}

fn provider_baseline<'a>(history: &'a UsageHistory, provider: &str) -> ProviderBaseline<'a> {
    let Some((latest_entry, earlier_entries)) = history.entries.split_last() else {
        return ProviderBaseline::default();
    };
    let Some(last_observation) = find_provider(&latest_entry.providers, provider) else {
        // A missing provider marks a disabled/unconfigured gap. Do not reach
        // across it and compare against stale quota data.
        return ProviderBaseline::default();
    };

    if last_observation.error.is_none() {
        return ProviderBaseline {
            last_observation: Some(last_observation),
            last_successful: Some(last_observation),
        };
    }

    let mut last_successful = None;
    for entry in earlier_entries.iter().rev() {
        let Some(observation) = find_provider(&entry.providers, provider) else {
            break;
        };
        if observation.error.is_none() {
            last_successful = Some(observation);
            break;
        }
    }

    ProviderBaseline {
        last_observation: Some(last_observation),
        last_successful,
    }
}

fn find_provider<'a>(providers: &'a [UsageData], provider: &str) -> Option<&'a UsageData> {
    providers
        .iter()
        .find(|data| data.provider.eq_ignore_ascii_case(provider))
}

fn matching_previous_window<'a>(
    previous: &'a UsageData,
    current: &UsageData,
    target: &UsageWindow,
) -> Option<&'a UsageWindow> {
    let target_key = normalized_label(&target.label);
    if let Some(exact) = previous
        .windows
        .iter()
        .filter(|window| is_notifiable_window(window))
        .find(|window| normalized_label(&window.label) == target_key)
    {
        return Some(exact);
    }

    let period = quota_period_for_window(&current.provider, target)?;
    let current_candidates = current
        .windows
        .iter()
        .filter(|window| is_notifiable_window(window))
        .filter(|window| quota_period_for_window(&current.provider, window) == Some(period))
        .count();
    if current_candidates != 1 {
        return None;
    }

    let mut previous_candidates = previous
        .windows
        .iter()
        .filter(|window| is_notifiable_window(window))
        .filter(|window| quota_period_for_window(&previous.provider, window) == Some(period));
    let candidate = previous_candidates.next()?;
    if previous_candidates.next().is_some() {
        return None;
    }
    Some(candidate)
}

fn latest_budget_spend_window<'a>(
    history: &'a UsageHistory,
    current_provider: &UsageData,
    current_window: &UsageWindow,
) -> Option<&'a UsageWindow> {
    if !crate::provider::is_budget_spend_window(
        &current_provider.provider,
        &current_window.label,
        current_window.unit.as_deref(),
    ) {
        return None;
    }

    for entry in history.entries.iter().rev() {
        let Some(previous_provider) = find_provider(&entry.providers, &current_provider.provider)
        else {
            break;
        };
        if previous_provider.error.is_some() {
            continue;
        }
        if let Some(window) = previous_provider.windows.iter().find(|window| {
            crate::provider::is_budget_spend_window(
                &previous_provider.provider,
                &window.label,
                window.unit.as_deref(),
            )
        }) {
            return Some(window);
        }
    }

    None
}

fn reset_happened(previous: &UsageWindow, current: &UsageWindow, now: DateTime<Utc>) -> bool {
    let (Some(previous_reset), Some(current_reset)) =
        (previous.resets_at.as_ref(), current.resets_at.as_ref())
    else {
        return false;
    };

    previous_reset <= &now
        && current_reset.signed_duration_since(*previous_reset) > RESET_ADVANCE_TOLERANCE
}

fn quota_period_from_reset_cadence(
    previous: &UsageWindow,
    current: &UsageWindow,
) -> Option<QuotaPeriod> {
    let (Some(previous_reset), Some(current_reset)) =
        (previous.resets_at.as_ref(), current.resets_at.as_ref())
    else {
        return None;
    };
    let cadence = current_reset.signed_duration_since(*previous_reset);

    // Generic labels such as "Subscription" are commonly monthly. Shorter
    // windows should identify themselves as 5h/session or weekly/7d; inferring
    // those from a long offline gap can misclassify multiple missed resets.
    if cadence >= Duration::days(25)
        && cadence <= Duration::days(35)
        && previous.used_percent.is_finite()
        && current.used_percent.is_finite()
        && current.used_percent + 1.0 < previous.used_percent
    {
        Some(QuotaPeriod::Monthly)
    } else {
        None
    }
}

fn valid_threshold(settings: &NotificationConfig) -> Option<f64> {
    settings
        .usage_threshold_enabled
        .then(|| settings.threshold_percent())
}

fn same_limit_generation(previous: &UsageWindow, current: &UsageWindow) -> bool {
    match (previous.limit, current.limit) {
        (Some(previous), Some(current)) => {
            (previous - current).abs() <= f64::EPSILON * previous.abs().max(current.abs()).max(1.0)
        }
        (None, None) => true,
        _ => false,
    }
}

fn reset_notifications_enabled(settings: &NotificationConfig, period: QuotaPeriod) -> bool {
    match period {
        QuotaPeriod::FiveHour => settings.five_hour_resets,
        QuotaPeriod::Weekly => settings.weekly_resets,
        QuotaPeriod::Monthly => settings.monthly_resets,
    }
}

fn quota_period(provider: &str, label: &str) -> Option<QuotaPeriod> {
    let normalized = normalized_label(label);
    let tokens = label
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let has_token = |expected: &str| tokens.iter().any(|token| token == expected);

    if has_token("5h") || normalized.contains("5hour") || normalized.contains("fivehour") {
        return Some(QuotaPeriod::FiveHour);
    }

    let provider = provider.trim().to_ascii_lowercase();
    if matches!(provider.as_str(), "codex" | "claude" | "ollama") && normalized.contains("session")
    {
        return Some(QuotaPeriod::FiveHour);
    }
    if provider == "opencode" && normalized.contains("rollingusage") {
        return Some(QuotaPeriod::FiveHour);
    }

    if has_token("7d")
        || normalized.contains("7day")
        || normalized.contains("weekly")
        || normalized.contains("sevenday")
        || has_token("week")
    {
        return Some(QuotaPeriod::Weekly);
    }

    if normalized.contains("monthly") || has_token("month") {
        return Some(QuotaPeriod::Monthly);
    }

    None
}

fn quota_period_for_window(provider: &str, window: &UsageWindow) -> Option<QuotaPeriod> {
    if provider.trim().eq_ignore_ascii_case("codex")
        && window
            .unit
            .as_deref()
            .is_some_and(|unit| unit.eq_ignore_ascii_case("seconds"))
    {
        match crate::provider::codex::rate_limit_period_key(window.limit) {
            Some("5h") => return Some(QuotaPeriod::FiveHour),
            Some("weekly") => return Some(QuotaPeriod::Weekly),
            Some("monthly") => return Some(QuotaPeriod::Monthly),
            _ => {}
        }
    }

    quota_period(provider, &window.label)
}

fn is_notifiable_window(window: &UsageWindow) -> bool {
    let label = normalized_label(&window.label);
    window.used_percent.is_finite()
        && !matches!(label.as_str(), "error" | "nodata" | "resetcredits")
}

fn normalized_label(label: &str) -> String {
    label
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn display_name(provider: &str) -> String {
    single_line(crate::provider::display_name(provider))
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_percent(value: f64) -> String {
    if (value - value.round()).abs() < 0.05 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_history::UsageHistoryEntry;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-07-29T{hour:02}:{minute:02}:00Z"))
            .unwrap()
            .to_utc()
    }

    fn settings() -> NotificationConfig {
        NotificationConfig {
            enabled: true,
            monthly_resets: true,
            weekly_resets: true,
            five_hour_resets: true,
            usage_threshold_enabled: true,
            usage_threshold_percent: 80.0,
            silent_refresh_failures: true,
        }
    }

    fn window(label: &str, used_percent: f64, resets_at: Option<DateTime<Utc>>) -> UsageWindow {
        UsageWindow {
            label: label.to_string(),
            used_percent,
            limit: Some(100.0),
            used: Some(used_percent),
            unit: Some("%".to_string()),
            resets_at,
        }
    }

    fn usage(
        provider: &str,
        windows: Vec<UsageWindow>,
        error: Option<&str>,
        fetched_at: DateTime<Utc>,
    ) -> UsageData {
        UsageData {
            provider: provider.to_string(),
            windows,
            credits: None,
            subscription_tier: None,
            fetched_at,
            error: error.map(str::to_string),
        }
    }

    fn entry(fetched_at: DateTime<Utc>, providers: Vec<UsageData>) -> UsageHistoryEntry {
        UsageHistoryEntry {
            fetched_at,
            providers,
        }
    }

    fn history(entries: Vec<UsageHistoryEntry>) -> UsageHistory {
        UsageHistory {
            entries,
            ..UsageHistory::default()
        }
    }

    #[test]
    fn classifies_only_supported_quota_periods() {
        assert_eq!(
            quota_period("codex", "Session"),
            Some(QuotaPeriod::FiveHour)
        );
        assert_eq!(
            quota_period("claude", "Session (5h)"),
            Some(QuotaPeriod::FiveHour)
        );
        assert_eq!(
            quota_period("stepfun", "5-hour Pro"),
            Some(QuotaPeriod::FiveHour)
        );
        assert_eq!(
            quota_period("opencode", "Rolling Usage"),
            Some(QuotaPeriod::FiveHour)
        );
        assert_eq!(
            quota_period("claude", "Weekly (Sonnet)"),
            Some(QuotaPeriod::Weekly)
        );
        assert_eq!(
            quota_period("provider", "seven_day"),
            Some(QuotaPeriod::Weekly)
        );
        assert_eq!(
            quota_period("provider", "Monthly Usage"),
            Some(QuotaPeriod::Monthly)
        );
        assert_eq!(quota_period("openai", "Cost 30d"), None);
        assert_eq!(quota_period("other", "Session"), None);
        assert_eq!(quota_period("other", "Rolling Usage"), None);
    }

    #[test]
    fn disabled_notifications_emit_nothing() {
        let mut config = settings();
        config.enabled = false;
        let current = vec![usage(
            "codex",
            vec![window("Weekly", 90.0, Some(at(18, 0)))],
            None,
            at(12, 0),
        )];

        assert!(evaluate(&config, &UsageHistory::default(), &current, at(12, 0), true).is_empty());
    }

    #[test]
    fn first_successful_observation_only_establishes_a_baseline() {
        let current = vec![usage(
            "codex",
            vec![window("Weekly", 90.0, Some(at(18, 0)))],
            None,
            at(12, 0),
        )];

        assert!(
            evaluate(
                &settings(),
                &UsageHistory::default(),
                &current,
                at(12, 0),
                true
            )
            .is_empty()
        );
    }

    #[test]
    fn reset_requires_expired_and_advanced_reset_timestamps() {
        let previous = usage(
            "codex",
            vec![window("Weekly", 70.0, Some(at(11, 0)))],
            None,
            at(10, 0),
        );
        let history = history(vec![entry(at(10, 0), vec![previous])]);
        let current = vec![usage(
            "codex",
            vec![window("Weekly", 2.0, Some(at(18, 0)))],
            None,
            at(12, 0),
        )];

        let events = evaluate(&settings(), &history, &current, at(12, 0), true);
        assert_eq!(
            events,
            vec![NotificationEvent::QuotaReset {
                provider: "codex".to_string(),
                window_label: "Weekly".to_string(),
                period: QuotaPeriod::Weekly,
                next_reset_at: at(18, 0),
            }]
        );

        let current_before_expiry = vec![usage(
            "codex",
            vec![window("Weekly", 2.0, Some(at(18, 0)))],
            None,
            at(10, 30),
        )];
        assert!(
            evaluate(
                &settings(),
                &history,
                &current_before_expiry,
                at(10, 30),
                true
            )
            .is_empty()
        );
    }

    #[test]
    fn reset_requires_both_reset_timestamps() {
        let previous = usage("codex", vec![window("Weekly", 90.0, None)], None, at(10, 0));
        let history = history(vec![entry(at(10, 0), vec![previous])]);
        let current = vec![usage(
            "codex",
            vec![window("Weekly", 1.0, Some(at(18, 0)))],
            None,
            at(12, 0),
        )];

        assert!(evaluate(&settings(), &history, &current, at(12, 0), true).is_empty());
    }

    #[test]
    fn generic_window_uses_reset_cadence_to_identify_monthly_quota() {
        let previous_reset = at(11, 0);
        let previous = usage(
            "synthetic",
            vec![window("Subscription", 95.0, Some(previous_reset))],
            None,
            at(10, 0),
        );
        let history = history(vec![entry(at(10, 0), vec![previous])]);
        let current = vec![usage(
            "synthetic",
            vec![window(
                "Subscription",
                1.0,
                Some(previous_reset + Duration::days(30)),
            )],
            None,
            at(12, 0),
        )];

        assert!(matches!(
            evaluate(&settings(), &history, &current, at(12, 0), true).as_slice(),
            [NotificationEvent::QuotaReset {
                period: QuotaPeriod::Monthly,
                ..
            }]
        ));
    }

    #[test]
    fn reset_does_not_repeat_after_new_generation_is_in_history() {
        let previous = usage(
            "codex",
            vec![window("Weekly", 1.0, Some(at(18, 0)))],
            None,
            at(12, 0),
        );
        let history = history(vec![entry(at(12, 0), vec![previous])]);
        let current = vec![usage(
            "codex",
            vec![window("Weekly", 2.0, Some(at(18, 0)))],
            None,
            at(12, 5),
        )];

        assert!(evaluate(&settings(), &history, &current, at(12, 5), true).is_empty());
    }

    #[test]
    fn unique_period_fallback_matches_renamed_window() {
        let previous = usage(
            "codex",
            vec![window("Session", 60.0, Some(at(11, 0)))],
            None,
            at(10, 0),
        );
        let history = history(vec![entry(at(10, 0), vec![previous])]);
        let current = vec![usage(
            "codex",
            vec![window("Session (5h)", 0.0, Some(at(16, 0)))],
            None,
            at(12, 0),
        )];

        let events = evaluate(&settings(), &history, &current, at(12, 0), true);
        assert!(matches!(
            events.as_slice(),
            [NotificationEvent::QuotaReset {
                period: QuotaPeriod::FiveHour,
                ..
            }]
        ));
    }

    #[test]
    fn codex_duration_matches_mislabelled_primary_window_to_weekly_window() {
        let mut previous_window = window("Session (5h)", 79.0, None);
        previous_window.limit = Some(604_800.0);
        previous_window.used = Some(522_049.0);
        previous_window.unit = Some("seconds".to_string());
        let previous = usage("codex", vec![previous_window], None, at(10, 0));
        let baseline_history = history(vec![entry(at(10, 0), vec![previous])]);

        let mut current_window = window("Weekly", 80.0, None);
        current_window.limit = Some(604_800.0);
        current_window.used = Some(518_400.0);
        current_window.unit = Some("seconds".to_string());
        let current = vec![usage("codex", vec![current_window], None, at(10, 5))];

        assert_eq!(
            evaluate(&settings(), &baseline_history, &current, at(10, 5), true),
            vec![NotificationEvent::UsageThresholdExceeded {
                provider: "codex".to_string(),
                window_label: "Weekly".to_string(),
                used_percent: 80.0,
                threshold_percent: 80.0,
            }]
        );
    }

    #[test]
    fn ambiguous_period_fallback_does_not_cross_match_windows() {
        let previous = usage(
            "antigravity",
            vec![
                window("Gemini Weekly", 79.0, Some(at(11, 0))),
                window("Claude Weekly", 79.0, Some(at(11, 0))),
            ],
            None,
            at(10, 0),
        );
        let history = history(vec![entry(at(10, 0), vec![previous])]);
        let current = vec![usage(
            "antigravity",
            vec![window("Weekly", 90.0, Some(at(18, 0)))],
            None,
            at(12, 0),
        )];

        assert!(evaluate(&settings(), &history, &current, at(12, 0), true).is_empty());
    }

    #[test]
    fn threshold_fires_only_on_an_upward_crossing() {
        let previous = usage("codex", vec![window("Weekly", 79.0, None)], None, at(10, 0));
        let baseline_history = history(vec![entry(at(10, 0), vec![previous])]);
        let current = vec![usage(
            "codex",
            vec![window("Weekly", 80.0, None)],
            None,
            at(10, 5),
        )];

        assert_eq!(
            evaluate(&settings(), &baseline_history, &current, at(10, 5), true),
            vec![NotificationEvent::UsageThresholdExceeded {
                provider: "codex".to_string(),
                window_label: "Weekly".to_string(),
                used_percent: 80.0,
                threshold_percent: 80.0,
            }]
        );

        let already_above = history(vec![entry(
            at(10, 5),
            vec![usage(
                "codex",
                vec![window("Weekly", 80.0, None)],
                None,
                at(10, 5),
            )],
        )]);
        let still_above = vec![usage(
            "codex",
            vec![window("Weekly", 90.0, None)],
            None,
            at(10, 10),
        )];
        assert!(evaluate(&settings(), &already_above, &still_above, at(10, 10), true).is_empty());
    }

    #[test]
    fn threshold_uses_last_success_through_contiguous_errors() {
        let successful = usage("codex", vec![window("Weekly", 79.0, None)], None, at(10, 0));
        let failed = usage("codex", Vec::new(), Some("secret error"), at(10, 5));
        let history = history(vec![
            entry(at(10, 0), vec![successful]),
            entry(at(10, 5), vec![failed]),
        ]);
        let current = vec![usage(
            "codex",
            vec![window("Weekly", 81.0, None)],
            None,
            at(10, 10),
        )];

        assert!(matches!(
            evaluate(&settings(), &history, &current, at(10, 10), true).as_slice(),
            [NotificationEvent::UsageThresholdExceeded { .. }]
        ));
    }

    #[test]
    fn budget_threshold_uses_last_spend_window_through_partial_failures() {
        let mut previous_spend = window("Cost 30d", 79.0, None);
        previous_spend.unit = Some("USD".to_string());
        let spend_before_failure = usage("openai", vec![previous_spend], None, at(10, 0));
        let credits_only = usage(
            "openai",
            vec![window("Credits", 25.0, None)],
            None,
            at(10, 5),
        );
        let history = history(vec![
            entry(at(10, 0), vec![spend_before_failure]),
            entry(at(10, 5), vec![credits_only.clone()]),
            entry(at(10, 10), vec![credits_only]),
        ]);
        let mut current_spend = window("Cost 30d", 90.0, None);
        current_spend.unit = Some("USD".to_string());
        let current = vec![usage("openai", vec![current_spend], None, at(10, 15))];

        assert!(matches!(
            evaluate(&settings(), &history, &current, at(10, 15), true).as_slice(),
            [NotificationEvent::UsageThresholdExceeded {
                provider,
                window_label,
                ..
            }] if provider == "openai" && window_label == "Cost 30d"
        ));
    }

    #[test]
    fn threshold_does_not_cross_when_the_budget_or_quota_limit_changes() {
        let previous = usage(
            "openai",
            vec![window("Cost 30d", 79.0, None)],
            None,
            at(10, 0),
        );
        let history = history(vec![entry(at(10, 0), vec![previous])]);
        let mut current_window = window("Cost 30d", 90.0, None);
        current_window.limit = Some(50.0);
        let current = vec![usage("openai", vec![current_window], None, at(10, 5))];

        assert!(evaluate(&settings(), &history, &current, at(10, 5), true).is_empty());
    }

    #[test]
    fn provider_absence_breaks_the_success_baseline() {
        let old = usage("codex", vec![window("Weekly", 79.0, None)], None, at(10, 0));
        let history = history(vec![
            entry(at(10, 0), vec![old]),
            entry(
                at(10, 5),
                vec![usage("claude", Vec::new(), None, at(10, 5))],
            ),
        ]);
        let current = vec![usage(
            "codex",
            vec![window("Weekly", 90.0, None)],
            None,
            at(10, 10),
        )];

        assert!(evaluate(&settings(), &history, &current, at(10, 10), true).is_empty());
    }

    #[test]
    fn invalid_percentages_are_ignored_and_thresholds_are_clamped() {
        let previous = usage("codex", vec![window("Weekly", 79.0, None)], None, at(10, 0));
        let history = history(vec![entry(at(10, 0), vec![previous])]);
        let current = vec![usage(
            "codex",
            vec![window("Weekly", f64::NAN, None)],
            None,
            at(10, 5),
        )];
        assert!(evaluate(&settings(), &history, &current, at(10, 5), true).is_empty());

        let mut invalid_settings = settings();
        invalid_settings.usage_threshold_percent = 101.0;
        assert_eq!(valid_threshold(&invalid_settings), Some(100.0));
        let current = vec![usage(
            "codex",
            vec![window("Weekly", 90.0, None)],
            None,
            at(10, 5),
        )];
        assert!(evaluate(&invalid_settings, &history, &current, at(10, 5), true).is_empty());
    }

    #[test]
    fn silent_failure_fires_on_the_error_edge_only() {
        let successful = usage("opencode", Vec::new(), None, at(10, 0));
        let successful_history = history(vec![entry(at(10, 0), vec![successful])]);
        let failed = vec![usage(
            "opencode",
            Vec::new(),
            Some("token=must-not-leak"),
            at(10, 5),
        )];

        assert_eq!(
            evaluate(&settings(), &successful_history, &failed, at(10, 5), true),
            vec![NotificationEvent::SilentRefreshFailed {
                provider: "opencode".to_string(),
            }]
        );
        assert!(evaluate(&settings(), &successful_history, &failed, at(10, 5), false).is_empty());

        let failed_history = history(vec![entry(at(10, 0), failed.clone())]);
        assert!(evaluate(&settings(), &failed_history, &failed, at(10, 10), true).is_empty());
    }

    #[test]
    fn first_failed_observation_notifies_once() {
        let failed = vec![usage(
            "opencode",
            Vec::new(),
            Some("unavailable"),
            at(10, 0),
        )];

        assert!(matches!(
            evaluate(
                &settings(),
                &UsageHistory::default(),
                &failed,
                at(10, 0),
                true
            )
            .as_slice(),
            [NotificationEvent::SilentRefreshFailed { .. }]
        ));
    }

    #[test]
    fn budget_refresh_failure_fires_on_the_silent_failure_edge_only() {
        let previous = std::collections::BTreeSet::new();
        let current = std::collections::BTreeSet::from(["openai".to_string()]);

        assert_eq!(
            evaluate_budget_refresh_failures(&settings(), &previous, &current, true),
            vec![NotificationEvent::BudgetRefreshFailed {
                provider: "openai".to_string(),
            }]
        );
        assert!(evaluate_budget_refresh_failures(&settings(), &current, &current, true).is_empty());
        assert!(
            evaluate_budget_refresh_failures(&settings(), &previous, &current, false).is_empty()
        );
    }

    #[test]
    fn disabled_notifications_still_suppress_budget_refresh_failures() {
        let mut config = settings();
        config.enabled = false;
        let current = std::collections::BTreeSet::from(["openai".to_string()]);

        assert!(
            evaluate_budget_refresh_failures(
                &config,
                &std::collections::BTreeSet::new(),
                &current,
                true,
            )
            .is_empty()
        );
    }

    #[test]
    fn auxiliary_windows_do_not_generate_threshold_events() {
        let previous = usage(
            "codex",
            vec![
                window("No data", 79.0, None),
                window("Reset Credits", 79.0, None),
            ],
            None,
            at(10, 0),
        );
        let history = history(vec![entry(at(10, 0), vec![previous])]);
        let current = vec![usage(
            "codex",
            vec![
                window("No data", 90.0, None),
                window("Reset Credits", 90.0, None),
            ],
            None,
            at(10, 5),
        )];

        assert!(evaluate(&settings(), &history, &current, at(10, 5), true).is_empty());
    }

    #[test]
    fn aggregate_combines_events_without_provider_error_details() {
        let events = vec![
            NotificationEvent::QuotaReset {
                provider: "codex".to_string(),
                window_label: "Weekly\nLimit".to_string(),
                period: QuotaPeriod::Weekly,
                next_reset_at: at(18, 0),
            },
            NotificationEvent::UsageThresholdExceeded {
                provider: "claude".to_string(),
                window_label: "Session (5h)".to_string(),
                used_percent: 82.25,
                threshold_percent: 80.0,
            },
            NotificationEvent::SilentRefreshFailed {
                provider: "opencode".to_string(),
            },
            NotificationEvent::BudgetRefreshFailed {
                provider: "openai".to_string(),
            },
        ];

        let (title, body) = aggregate(&events).unwrap();
        assert_eq!(title, "Quotify notifications");
        assert!(body.contains("Codex: Weekly Limit reset (weekly)"));
        assert!(body.contains("Claude: Session (5h) 82.2% (threshold 80%)"));
        assert!(body.contains("OpenCode: background refresh failed"));
        assert!(body.contains("OpenAI: 30-day spend data unavailable"));
        assert!(!body.contains("secret"));
    }

    #[test]
    fn aggregate_fits_the_native_body_and_summarizes_omitted_events() {
        let events = (0..20)
            .map(|index| NotificationEvent::SilentRefreshFailed {
                provider: format!("provider-{index}-with-a-readable-name"),
            })
            .collect::<Vec<_>>();

        let (_, body) = aggregate(&events).unwrap();

        assert!(body.encode_utf16().count() <= MAX_NOTIFICATION_BODY_UTF16);
        assert!(body.contains(" more"));
    }

    #[test]
    fn utf16_truncation_keeps_surrogate_pairs_intact() {
        let value = format!("{}end", "😀".repeat(200));
        let truncated = truncate_utf16_with_ellipsis(&value, 17);

        assert!(truncated.encode_utf16().count() <= 17);
        assert!(truncated.ends_with('…'));
        assert!(String::from_utf16(&truncated.encode_utf16().collect::<Vec<_>>()).is_ok());
    }
}
