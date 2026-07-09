use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn app_dir() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("quotify");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn log_dir() -> PathBuf {
    let dir = app_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn diagnostics_dir() -> PathBuf {
    let dir = app_dir().join("diagnostics");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn init_file_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let file_appender = tracing_appender::rolling::never(log_dir(), "quotify.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("quotify=info,warn"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .init();

    guard
}

pub fn write_diagnostic_report(
    config_path: Option<&std::path::Path>,
    history: Option<&crate::usage_history::UsageHistory>,
) -> Result<PathBuf> {
    let now = chrono::Utc::now();
    let filename = format!("diagnostic-{}.txt", now.format("%Y%m%d-%H%M%S"));
    let path = diagnostics_dir().join(filename);

    let mut report = String::new();
    report.push_str("Quotify Diagnostic Report\n");
    report.push_str("=========================\n\n");
    report.push_str(&format!("Generated: {now}\n"));
    report.push_str(&format!("Version: {}\n", env!("GIT_TAG")));
    report.push_str(&format!("Config dir: {}\n", app_dir().display()));
    report.push_str(&format!("Log dir: {}\n", log_dir().display()));
    report.push_str(&format!(
        "Config path: {}\n",
        config_path
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| crate::config::AppConfig::config_path()
                .display()
                .to_string())
    ));
    report.push_str(&format!(
        "Startup enabled: {}\n",
        crate::startup::is_enabled().unwrap_or(false)
    ));
    report.push('\n');

    if let Some(history) = history {
        report.push_str("Usage history\n");
        report.push_str("-------------\n");
        report.push_str(&format!("Entries: {}\n", history.entries.len()));
        if let Some(latest) = history.entries.last() {
            report.push_str(&format!("Latest snapshot: {}\n", latest.fetched_at));
            report.push_str(&format!("Latest providers: {}\n", latest.providers.len()));
        }
        report.push('\n');
    }

    report.push_str("Recent log files\n");
    report.push_str("----------------\n");
    if let Ok(entries) = std::fs::read_dir(log_dir()) {
        let mut paths: Vec<_> = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .collect();
        paths.sort();
        for path in paths.into_iter().rev().take(5) {
            report.push_str(&format!("{}\n", path.display()));
        }
    }

    std::fs::write(&path, report).with_context(|| format!("Failed to write {:?}", path))?;
    Ok(path)
}
