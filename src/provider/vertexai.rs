use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration as StdDuration, Instant},
};

use super::{Provider, UsageData, UsageWindow};

pub struct VertexAiProvider {
    project_id: String,
}

impl VertexAiProvider {
    pub fn new(project_id: String) -> Self {
        Self { project_id }
    }

    fn project_id(&self) -> Option<String> {
        if !self.project_id.trim().is_empty() {
            return Some(self.project_id.trim().to_string());
        }
        [
            "GOOGLE_CLOUD_PROJECT",
            "GCLOUD_PROJECT",
            "GOOGLE_PROJECT_ID",
        ]
        .into_iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
    }

    pub fn has_project(project_id: &str) -> bool {
        !project_id.trim().is_empty()
            || [
                "GOOGLE_CLOUD_PROJECT",
                "GCLOUD_PROJECT",
                "GOOGLE_PROJECT_ID",
            ]
            .into_iter()
            .any(|key| {
                std::env::var(key)
                    .ok()
                    .is_some_and(|value| !value.trim().is_empty())
            })
    }
}

#[async_trait::async_trait]
impl Provider for VertexAiProvider {
    fn name(&self) -> &str {
        "vertexai"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let project_id = self.project_id().context(
            "Vertex AI requires a Google Cloud project ID in [vertexai].api_key or GOOGLE_CLOUD_PROJECT",
        )?;
        let output = tokio::task::spawn_blocking(move || run_gcloud_monitoring(&project_id))
            .await
            .context("Failed to join Vertex AI monitoring task")??;
        let json: serde_json::Value =
            serde_json::from_str(&output).context("Failed to parse gcloud monitoring response")?;
        let samples = sum_quota_samples(&json);

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Quota Usage 24h".to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(samples),
                unit: Some("requests".to_string()),
                resets_at: None,
            }],
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn run_gcloud_monitoring(project_id: &str) -> Result<String> {
    let end = Utc::now();
    let start = end - ChronoDuration::hours(24);
    let interval = format!(
        "start={},end={}",
        start.to_rfc3339_opts(SecondsFormat::Secs, true),
        end.to_rfc3339_opts(SecondsFormat::Secs, true)
    );

    let mut cmd = Command::new("gcloud");
    cmd.arg("monitoring")
        .arg("time-series")
        .arg("list")
        .arg("--project")
        .arg(project_id)
        .arg("--filter")
        .arg(
            r#"metric.type="serviceruntime.googleapis.com/quota/rate/net_usage" AND resource.label.service="aiplatform.googleapis.com""#,
        )
        .arg("--interval")
        .arg(interval)
        .arg("--format")
        .arg("json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().context("Failed to run gcloud CLI")?;
    let deadline = Instant::now() + StdDuration::from_secs(20);
    loop {
        if child
            .try_wait()
            .context("Failed to poll gcloud CLI")?
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("gcloud monitoring command timed out after 20s");
        }
        thread::sleep(StdDuration::from_millis(100));
    }

    let output = child
        .wait_with_output()
        .context("Failed to collect gcloud output")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "gcloud CLI exited with {}: {}",
            output.status,
            stderr.trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn sum_quota_samples(value: &serde_json::Value) -> f64 {
    match value {
        serde_json::Value::Object(map) => {
            let own = ["doubleValue", "int64Value", "value"]
                .into_iter()
                .find_map(|key| map.get(key).and_then(json_number))
                .unwrap_or(0.0);
            own + map.values().map(sum_quota_samples).sum::<f64>()
        }
        serde_json::Value::Array(values) => values.iter().map(sum_quota_samples).sum(),
        _ => 0.0,
    }
}

fn json_number(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}
