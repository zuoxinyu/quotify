use anyhow::{Context, Result};
use chrono::Utc;
use regex::Regex;
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{Provider, UsageData, UsageWindow};

pub struct KiroProvider {
    api_key: String,
}

impl KiroProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    fn resolve_api_key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.trim().to_string());
        }
        std::env::var("KIRO_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
    }
}

#[async_trait::async_trait]
impl Provider for KiroProvider {
    fn name(&self) -> &str {
        "kiro"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let api_key = self
            .resolve_api_key()
            .context("Kiro API key not configured. Set api_key or KIRO_API_KEY")?;

        let output = tokio::task::spawn_blocking(move || run_kiro_usage(&api_key))
            .await
            .context("Failed to join Kiro usage task")??;

        let windows = parse_usage_windows(&output);
        if windows.is_empty() {
            anyhow::bail!("Kiro usage output did not contain parseable quota data: {output}");
        }

        Ok(UsageData {
            provider: self.name().to_string(),
            windows,
            credits: None,
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn run_kiro_usage(api_key: &str) -> Result<String> {
    let mut cmd = Command::new("kiro-cli");
    cmd.arg("chat")
        .arg("--no-interactive")
        .arg("/usage")
        .env("KIRO_API_KEY", api_key)
        .env("NO_COLOR", "1")
        .env("KIRO_NO_PROGRESS", "1")
        .env("KIRO_NO_HYPERLINKS", "1");

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("Failed to run kiro-cli")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(_status) = child.try_wait().context("Failed to poll kiro-cli")? {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("kiro-cli usage command timed out after 10s");
        }
        thread::sleep(Duration::from_millis(100));
    }

    let output = child
        .wait_with_output()
        .context("Failed to collect kiro-cli output")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("kiro-cli exited with {}: {}", output.status, stderr.trim());
    }

    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    Ok(strip_ansi(&text))
}

fn parse_usage_windows(output: &str) -> Vec<UsageWindow> {
    let percent_re = Regex::new(
        r"(?i)(?P<label>[A-Za-z][A-Za-z0-9 /_-]{0,40}).{0,40}?(?P<pct>\d+(?:\.\d+)?)\s*%",
    )
    .unwrap();
    let fraction_re = Regex::new(
        r"(?i)(?P<label>[A-Za-z][A-Za-z0-9 /_-]{0,40}).{0,40}?(?P<used>\d+(?:\.\d+)?)\s*/\s*(?P<limit>\d+(?:\.\d+)?)",
    )
    .unwrap();

    let mut windows = Vec::new();
    for line in output.lines() {
        if let Some(caps) = fraction_re.captures(line) {
            let used = caps["used"].parse::<f64>().unwrap_or(0.0);
            let limit = caps["limit"].parse::<f64>().unwrap_or(0.0);
            if limit > 0.0 {
                windows.push(UsageWindow {
                    label: clean_label(&caps["label"]),
                    used_percent: (used / limit * 100.0).clamp(0.0, 100.0),
                    limit: Some(limit),
                    used: Some(used),
                    unit: Some("credits".to_string()),
                    resets_at: None,
                });
                continue;
            }
        }

        if let Some(caps) = percent_re.captures(line) {
            let pct = caps["pct"].parse::<f64>().unwrap_or(0.0);
            windows.push(UsageWindow {
                label: clean_label(&caps["label"]),
                used_percent: pct.clamp(0.0, 100.0),
                limit: None,
                used: None,
                unit: None,
                resets_at: None,
            });
        }
    }

    windows
}

fn clean_label(label: &str) -> String {
    let label = label
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .trim();
    if label.is_empty() {
        "Usage".to_string()
    } else {
        label.to_string()
    }
}

fn strip_ansi(input: &str) -> String {
    let ansi = Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap();
    ansi.replace_all(input, "").to_string()
}
