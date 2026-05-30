use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate, Utc};
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration as StdDuration, Instant},
};

use super::{CreditsInfo, Provider, UsageData, UsageWindow};

pub struct BedrockProvider {
    budget: String,
}

impl BedrockProvider {
    pub fn new(budget: String) -> Self {
        Self { budget }
    }

    fn budget(&self) -> Option<f64> {
        if !self.budget.trim().is_empty() {
            return self.budget.trim().parse().ok();
        }
        std::env::var("CODEXBAR_BEDROCK_BUDGET")
            .ok()
            .and_then(|value| value.trim().parse().ok())
    }
}

#[async_trait::async_trait]
impl Provider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let budget = self.budget();
        let output = tokio::task::spawn_blocking(run_cost_explorer)
            .await
            .context("Failed to join AWS Bedrock Cost Explorer task")??;
        let json: serde_json::Value =
            serde_json::from_str(&output).context("Failed to parse AWS Cost Explorer response")?;
        let cost = sum_amounts(&json);
        let used_percent = budget
            .filter(|budget| *budget > 0.0)
            .map(|budget| (cost / budget * 100.0).clamp(0.0, 100.0))
            .unwrap_or(0.0);

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Cost 30d".to_string(),
                used_percent,
                limit: budget,
                used: Some(cost),
                unit: Some("USD".to_string()),
                resets_at: None,
            }],
            credits: budget.map(|budget| CreditsInfo {
                balance: (budget - cost).max(0.0),
                currency: "USD".to_string(),
                total_granted: Some(budget),
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn run_cost_explorer() -> Result<String> {
    let end = Utc::now().date_naive() + Duration::days(1);
    let start = end - Duration::days(30);
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
