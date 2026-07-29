use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration as StdDuration, Instant},
};

use super::{Provider, UsageData, UsageWindow, completed_utc_day_window};

#[derive(Default)]
pub struct BedrockProvider;

impl BedrockProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Provider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let output = tokio::task::spawn_blocking(run_cost_explorer)
            .await
            .context("Failed to join AWS Bedrock Cost Explorer task")??;
        let json: serde_json::Value =
            serde_json::from_str(&output).context("Failed to parse AWS Cost Explorer response")?;
        let cost = sum_amounts(&json);

        Ok(usage_data_from_cost(cost, Utc::now()))
    }
}

fn usage_data_from_cost(cost: f64, fetched_at: DateTime<Utc>) -> UsageData {
    UsageData {
        provider: "bedrock".to_string(),
        windows: vec![UsageWindow {
            label: "Cost 30d".to_string(),
            used_percent: 0.0,
            limit: None,
            used: Some(cost),
            unit: Some("USD".to_string()),
            resets_at: None,
        }],
        credits: None,
        fetched_at,
        error: None,
    }
}

fn run_cost_explorer() -> Result<String> {
    let (start, end) = completed_utc_day_window(Utc::now());
    let start = start.date_naive();
    let end = end.date_naive();
    let mut cmd = Command::new("aws");
    cmd.arg("ce")
        .arg("get-cost-and-usage")
        .arg("--time-period")
        .arg(format!("Start={},End={}", fmt_date(start), fmt_date(end)))
        .arg("--granularity")
        .arg("DAILY")
        .arg("--metrics")
        .arg("UnblendedCost")
        .arg("--filter")
        .arg(r#"{"Dimensions":{"Key":"SERVICE","Values":["Amazon Bedrock"]}}"#)
        .arg("--output")
        .arg("json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().context("Failed to run aws CLI")?;
    let deadline = Instant::now() + StdDuration::from_secs(20);
    loop {
        if child
            .try_wait()
            .context("Failed to poll aws CLI")?
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("aws Cost Explorer command timed out after 20s");
        }
        thread::sleep(StdDuration::from_millis(100));
    }
    let output = child
        .wait_with_output()
        .context("Failed to collect aws CLI output")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("aws CLI exited with {}: {}", output.status, stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn fmt_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn sum_amounts(value: &serde_json::Value) -> f64 {
    match value {
        serde_json::Value::Object(map) => {
            let own = if map.contains_key("Amount") {
                map.get("Amount")
                    .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            own + map.values().map(sum_amounts).sum::<f64>()
        }
        serde_json::Value::Array(values) => values.iter().map(sum_amounts).sum(),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn builds_raw_cost_usage_without_a_budget() {
        let fetched_at = Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 0).unwrap();
        let data = usage_data_from_cost(42.25, fetched_at);
        let window = &data.windows[0];

        assert_eq!(data.provider, "bedrock");
        assert_eq!(data.fetched_at, fetched_at);
        assert_eq!(window.label, "Cost 30d");
        assert_eq!(window.used, Some(42.25));
        assert_eq!(window.used_percent, 0.0);
        assert_eq!(window.limit, None);
        assert_eq!(window.unit.as_deref(), Some("USD"));
        assert!(data.credits.is_none());
    }

    #[test]
    fn sums_nested_cost_explorer_amounts() {
        let response = serde_json::json!({
            "ResultsByTime": [
                {"Total": {"UnblendedCost": {"Amount": "1.25", "Unit": "USD"}}},
                {"Total": {"UnblendedCost": {"Amount": 2.75, "Unit": "USD"}}}
            ]
        });

        assert_eq!(sum_amounts(&response), 4.0);
    }
}
