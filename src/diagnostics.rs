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

pub fn clean_old_logs(days_to_keep: i64) {
    let log_dir = log_dir();
    let now = chrono::Utc::now();
    if let Ok(entries) = std::fs::read_dir(log_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    if file_name.starts_with("quotify.log") {
                        if let Ok(metadata) = entry.metadata() {
                            if let Ok(modified) = metadata.modified() {
                                let modified_chrono: chrono::DateTime<chrono::Utc> = modified.into();
                                let age = now.signed_duration_since(modified_chrono);
                                if age.num_days() > days_to_keep {
                                    let _ = std::fs::remove_file(path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn init_file_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    // Clean logs older than 7 days
    clean_old_logs(7);

    let file_appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("quotify.log")
        .build(log_dir());

    match file_appender {
        Ok(file_appender) => {
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
            let filter = EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("quotify=info,warn"));
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
                .init();
            Some(guard)
        }
        Err(err) => {
            eprintln!("Failed to initialize file logging, falling back to stderr: {err}");
            let filter = EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("quotify=info,warn"));
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(std::io::stderr).with_ansi(false))
                .init();
            None
        }
    }
}

pub fn setup_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.as_str()
        } else {
            "Unknown panic payload"
        };

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let backtrace = std::backtrace::Backtrace::capture();

        let crash_report = format!(
            "Quotify Crash Report\n\
             ====================\n\
             Version: {}\n\
             Time: {}\n\
             Location: {}\n\
             Error: {}\n\n\
             Stack Trace:\n\
             {}",
            env!("GIT_TAG"),
            chrono::Utc::now(),
            location,
            payload,
            backtrace
        );

        let report_dir = diagnostics_dir();
        let filename = format!("crash-{}.txt", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
        let report_path = report_dir.join(&filename);
        let _ = std::fs::write(&report_path, &crash_report);

        // Display a Windows message box
        #[cfg(target_os = "windows")]
        unsafe {
            use windows::core::w;
            use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

            let message = format!(
                "Quotify has encountered a fatal error and has crashed.\n\n\
                 Error details: {}\n\
                 Location: {}\n\n\
                 A detailed crash report has been saved to:\n\
                 {}",
                payload,
                location,
                report_path.display()
            );

            let message_wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

            MessageBoxW(
                None,
                windows::core::PCWSTR(message_wide.as_ptr()),
                w!("Quotify Crash"),
                MB_ICONERROR | MB_OK,
            );
        }

        #[cfg(not(target_os = "windows"))]
        {
            eprintln!("Crash details:\n{}", crash_report);
        }
    }));
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
