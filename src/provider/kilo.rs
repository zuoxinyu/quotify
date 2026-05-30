use anyhow::{Context, Result};
use chrono::Utc;
use regex::Regex;
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{Provider, UsageData, UsageWindow};

pub struct KiloProvider {
    enabled_token: String,
}

impl KiloProvider {
    pub fn new(enabled_token: String) -> Self {
        Self { enabled_token }
    }

    pub fn has_cli_or_token(enabled_token: &str) -> bool {
        !enabled_token.trim().is_empty()
            || std::env::var("KILO_API_KEY")
                .ok()
                .is_some_and(|key| !key.trim().is_empty())
            || command_exists("kilo")
    }
}

#[async_trait::async_trait]
impl Provider for KiloProvider {
    fn name(&self) -> &str {
        "kilo"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let configured_token = if !self.enabled_token.trim().is_empty() {
            Some(self.enabled_token.trim().to_string())
        } else {
            std::env::var("KILO_API_KEY")
                .ok()
                .filter(|key| !key.trim().is_empty())
        };
        let output =
            tokio::task::spawn_blocking(move || run_kilo_stats(configured_token.as_deref()))
                .await
                .context("Failed to join Kilo stats task")??;
        let windows = parse_usage_windows(&output);
        if windows.is_empty() {
            anyhow::bail!("Kilo stats output did not contain parseable quota data: {output}");
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

fn run_kilo_stats(api_key: Option<&str>) -> Result<String> {
    let mut cmd = Command::new("kilo");
    cmd.arg("stats")
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(api_key) = api_key {
        cmd.env("KILO_API_KEY", api_key);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().context("Failed to run kilo stats")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child
            .try_wait()
            .context("Failed to poll kilo stats")?
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("kilo stats timed out after 10s");
        }
        thread::sleep(Duration::from_millis(100));
    }

    let output = child
        .wait_with_output()
        .context("Failed to collect kilo stats output")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "kilo stats exited with {}: {}",
            output.status,
            stderr.trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(strip_ansi(if stdout.trim().is_empty() {
        &stderr
    } else {
        &stdout
    }))
}

fn parse_usage_windows(output: &str) -> Vec<UsageWindow> {
    let fraction_re = Regex::new(
        r"(?i)(?P<label>[A-Za-z][A-Za-z0-9 /_-]{0,40}).{0,40}?(?P<used>\d+(?:\.\d+)?)\s*/\s*(?P<limit>\d+(?:\.\d+)?)",
    )
    .unwrap();
    let percent_re = Regex::new(
        r"(?i)(?P<label>[A-Za-z][A-Za-z0-9 /_-]{0,40}).{0,40}?(?P<pct>\d+(?:\.\d+)?)\s*%",
    )
    .unwrap();

    output
        .lines()
        .filter_map(|line| {
            if let Some(caps) = fraction_re.captures(line) {
                let used = caps["used"].parse::<f64>().ok()?;
                let limit = caps["limit"].parse::<f64>().ok()?;
                return Some(UsageWindow {
                    label: clean_label(&caps["label"]),
                    used_percent: if limit > 0.0 {
                        (used / limit * 100.0).clamp(0.0, 100.0)
                    } else {
                        0.0
                    },
                    limit: Some(limit),
                    used: Some(used),
                    unit: Some("credits".to_string()),
                    resets_at: None,
                });
            }
            percent_re.captures(line).map(|caps| UsageWindow {
                label: clean_label(&caps["label"]),
                used_percent: caps["pct"].parse::<f64>().unwrap_or(0.0).clamp(0.0, 100.0),
                limit: None,
                used: None,
                unit: None,
                resets_at: None,
            })
        })
        .collect()
}

fn command_exists(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| child.wait())
        .is_ok()
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
