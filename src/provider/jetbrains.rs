use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use std::{fs, path::PathBuf};

use super::{Provider, UsageData, UsageWindow};

pub struct JetBrainsProvider {
    config_path: String,
}

impl JetBrainsProvider {
    pub fn new(config_path: String) -> Self {
        Self { config_path }
    }

    pub fn quota_file_exists(config_path: &str) -> bool {
        find_quota_file(config_path)
            .and_then(|path| fs::read_to_string(path).ok())
            .is_some_and(|content| parse_quota(&content).is_some())
    }
}

#[async_trait::async_trait]
impl Provider for JetBrainsProvider {
    fn name(&self) -> &str {
        "jetbrains"
    }

    async fn fetch_usage(&self) -> Result<UsageData> {
        let path = find_quota_file(&self.config_path)
            .context("JetBrains AI quota cache not found. Enable provider only after an IDE has written AIAssistantQuotaManager2.xml")?;
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read JetBrains quota file {}", path.display()))?;

        let parsed = parse_quota(&content)
            .context("JetBrains quota cache did not contain usable quotaInfo")?;
        let current = parsed.current;
        let maximum = parsed.maximum;
        let available = parsed.available;
        let reset = parsed.reset;
        let used_percent = maximum
            .filter(|maximum| *maximum > 0.0)
            .map(|maximum| (current / maximum * 100.0).clamp(0.0, 100.0))
            .unwrap_or(0.0);

        Ok(UsageData {
            provider: self.name().to_string(),
            windows: vec![UsageWindow {
                label: "Monthly".to_string(),
                used_percent,
                limit: maximum,
                used: Some(current),
                unit: Some("credits".to_string()),
                resets_at: reset,
            }],
            credits: available.map(|balance| super::CreditsInfo {
                balance,
                currency: "credits".to_string(),
                total_granted: maximum,
                topped_up: None,
            }),
            fetched_at: Utc::now(),
            error: None,
        })
    }
}

fn find_quota_file(config_path: &str) -> Option<PathBuf> {
    if !config_path.trim().is_empty() {
        let path = PathBuf::from(config_path.trim());
        return path.exists().then_some(path);
    }

    let base = dirs::config_dir()?.join("JetBrains");
    let mut candidates = Vec::new();
    collect_quota_files(&base, &mut candidates);
    candidates
        .into_iter()
        .filter_map(|path| {
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

fn collect_quota_files(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_quota_files(&path, out);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "AIAssistantQuotaManager2.xml")
        {
            out.push(path);
        }
    }
}

struct ParsedQuota {
    current: f64,
    maximum: Option<f64>,
    available: Option<f64>,
    reset: Option<DateTime<Utc>>,
}

fn parse_quota(content: &str) -> Option<ParsedQuota> {
    let quota_json = option_json(content, "quotaInfo")?;
    let current = json_number(&quota_json, &["current"]).unwrap_or(0.0);
    let maximum = json_number(&quota_json, &["maximum"]);
    let available = json_number(&quota_json, &["available"]).or_else(|| {
        let tariff = json_number(&quota_json, &["tariffQuota", "available"]).unwrap_or(0.0);
        let top_up = json_number(&quota_json, &["topUpQuota", "available"]).unwrap_or(0.0);
        (tariff > 0.0 || top_up > 0.0).then_some(tariff + top_up)
    });
    let maximum = maximum.or_else(|| available.map(|available| current + available));
    if current == 0.0 && maximum.is_none() && available.is_none() {
        return None;
    }

    let reset = json_string(&quota_json, &["until"])
        .or_else(|| {
            option_json(content, "nextRefill").and_then(|next| json_string(&next, &["next"]))
        })
        .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
        .map(|dt| dt.with_timezone(&Utc));

    Some(ParsedQuota {
        current,
        maximum,
        available,
        reset,
    })
}

fn option_json(content: &str, option_name: &str) -> Option<serde_json::Value> {
    let pattern = format!(r#"<option\s+name="{option_name}"\s+value="([^"]*)""#);
    let re = Regex::new(&pattern).ok()?;
    let raw = re.captures(content)?.get(1)?.as_str();
    let decoded = decode_xml_attr(raw);
    serde_json::from_str(&decoded).ok()
}

fn decode_xml_attr(raw: &str) -> String {
    raw.replace("&#10;", "\n")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn json_number(value: &serde_json::Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_f64().or_else(|| current.as_str()?.parse().ok())
}

fn json_string(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(str::to_string)
}
