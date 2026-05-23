mod app;
mod config;
mod cookies;
mod icon;
mod provider;
mod tray;

use anyhow::Result;
use clap::{Parser, Subcommand};
use parking_lot::RwLock;
use provider::{
    Provider, UsageData, antigravity::AntigravityProvider, claude::ClaudeProvider,
    codex::CodexProvider, deepseek::DeepSeekProvider, gemini::GeminiProvider, mimo::MimoProvider,
    opencode::OpenCodeProvider,
};
use std::sync::{Arc, OnceLock, atomic::Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};
use windows::core::w;
use winit::platform::windows::EventLoopBuilderExtWindows;

pub static EGUI_CONTEXT: OnceLock<eframe::egui::Context> = OnceLock::new();

#[derive(Parser)]
#[command(
    name = "quotify",
    about = "AI provider quota monitor for Windows",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, help = "Path to config file")]
    config: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    Fetch {
        #[arg(long, help = "Only fetch specific provider(s)")]
        provider: Option<Vec<String>>,
    },
    Init,
    Tray,
}

fn create_provider(name: &str, config: &config::AppConfig) -> Option<Box<dyn Provider>> {
    match name {
        "deepseek" => {
            let api_key = if !config.deepseek.api_key.is_empty() {
                config.deepseek.api_key.clone()
            } else {
                std::env::var("DEEPSEEK_API_KEY").unwrap_or_default()
            };
            if config.deepseek.enabled || !api_key.is_empty() {
                Some(Box::new(DeepSeekProvider::new(api_key)))
            } else {
                None
            }
        }
        "claude" => {
            let has_creds = config.claude.enabled
                || !config.claude.auth_file.is_empty()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".claude")
                    .join(".credentials.json")
                    .exists()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".claude")
                    .join("settings.json")
                    .exists();
            if has_creds {
                let auth_file = if config.claude.auth_file.is_empty() {
                    None
                } else {
                    Some(config.claude.auth_file.clone())
                };
                Some(Box::new(ClaudeProvider::new(auth_file)))
            } else {
                None
            }
        }
        "codex" => {
            let has_auth = config.codex.enabled
                || !config.codex.auth_file.is_empty()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".codex")
                    .join("auth.json")
                    .exists();
            if has_auth {
                let auth_file = if config.codex.auth_file.is_empty() {
                    None
                } else {
                    Some(config.codex.auth_file.clone())
                };
                Some(Box::new(CodexProvider::new(auth_file)))
            } else {
                None
            }
        }
        "gemini" => {
            let api_key = if !config.gemini.api_key.is_empty() {
                Some(config.gemini.api_key.clone())
            } else {
                None
            };
            if config.gemini.enabled
                || api_key.is_some()
                || std::env::var("GEMINI_API_KEY").is_ok()
                || std::env::var("GOOGLE_API_KEY").is_ok()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".gemini")
                    .join("oauth_creds.json")
                    .exists()
            {
                Some(Box::new(GeminiProvider::new(api_key)))
            } else {
                None
            }
        }
        "antigravity" => {
            let api_key = if !config.antigravity.api_key.is_empty() {
                Some(config.antigravity.api_key.clone())
            } else {
                None
            };
            if config.antigravity.enabled
                || api_key.is_some()
                || std::env::var("ANTIGRAVITY_API_KEY").is_ok()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".antigravity")
                    .join("oauth_creds.json")
                    .exists()
            {
                Some(Box::new(AntigravityProvider::new(api_key)))
            } else {
                None
            }
        }
        "opencode" => {
            let api_key = if !config.opencode.api_key.is_empty() {
                Some(config.opencode.api_key.clone())
            } else {
                None
            };
            if config.opencode.enabled
                || api_key.is_some()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".local")
                    .join("share")
                    .join("opencode")
                    .join("auth.json")
                    .exists()
                || std::env::var("OPENCODE_API_KEY").is_ok()
            {
                Some(Box::new(OpenCodeProvider::new(api_key)))
            } else {
                None
            }
        }
        "mimo" => {
            if config.mimo.enabled || !config.mimo.api_key.is_empty() {
                Some(Box::new(MimoProvider::new(config.mimo.api_key.clone())))
            } else {
                None
            }
        }
        _ => {
            eprintln!("Unknown provider: {name}");
            None
        }
    }
}

async fn fetch_all_providers(
    config: &config::AppConfig,
    data: Arc<RwLock<Vec<UsageData>>>,
    last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
) {
    let all_providers = [
        "deepseek",
        "claude",
        "codex",
        "gemini",
        "antigravity",
        "opencode",
        "mimo",
    ];

    let active: Vec<String> = all_providers
        .iter()
        .filter(|name| create_provider(name, config).is_some())
        .map(|s| s.to_string())
        .collect();

    let provider_names = if active.is_empty() {
        all_providers.iter().map(|s| s.to_string()).collect()
    } else {
        active
    };

    let mut results = Vec::new();

    for name in &provider_names {
        if let Some(provider) = create_provider(name, config) {
            match provider.fetch_usage().await {
                Ok(d) => results.push(d),
                Err(e) => {
                    tracing::error!("Failed to fetch {}: {}", name, e);
                    results.push(UsageData {
                        provider: name.clone(),
                        windows: vec![provider::UsageWindow {
                            label: "Error".to_string(),
                            used_percent: 0.0,
                            limit: None,
                            used: None,
                            unit: None,
                            resets_at: None,
                        }],
                        credits: None,
                        fetched_at: chrono::Utc::now(),
                        error: Some(e.to_string()),
                    });
                }
            }
        }
    }

    *data.write() = results;
    *last_refresh.write() = chrono::Utc::now();
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();

    let config = if let Some(ref path) = cli.config {
        let path = std::path::PathBuf::from(path);
        config::AppConfig::load_from(&path)?
    } else {
        config::AppConfig::load()?
    };

    match cli.command.unwrap_or(Commands::Tray) {
        Commands::Fetch {
            provider: providers,
        } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(run_fetch(&config, providers))?;
        }
        Commands::Init => {
            let path = config::AppConfig::config_path();
            config.save()?;
            println!("Config written to: {}", path.display());
        }
        Commands::Tray => {
            run_tray(config)?;
        }
    }

    Ok(())
}

async fn run_fetch(config: &config::AppConfig, providers: Option<Vec<String>>) -> Result<()> {
    let all_providers = [
        "deepseek",
        "claude",
        "codex",
        "gemini",
        "antigravity",
        "opencode",
        "mimo",
    ];

    let provider_names = providers.unwrap_or_else(|| {
        let active: Vec<String> = all_providers
            .iter()
            .filter(|name| create_provider(name, config).is_some())
            .map(|s| s.to_string())
            .collect();

        if active.is_empty() {
            all_providers.iter().map(|s| s.to_string()).collect()
        } else {
            active
        }
    });

    let mut results = Vec::new();

    for name in &provider_names {
        if let Some(provider) = create_provider(name, config) {
            match provider.fetch_usage().await {
                Ok(data) => results.push(data),
                Err(e) => {
                    tracing::error!("Failed to fetch {}: {}", name, e);
                    results.push(UsageData {
                        provider: name.clone(),
                        windows: vec![provider::UsageWindow {
                            label: "Error".to_string(),
                            used_percent: 0.0,
                            limit: None,
                            used: None,
                            unit: None,
                            resets_at: None,
                        }],
                        credits: None,
                        fetched_at: chrono::Utc::now(),
                        error: Some(e.to_string()),
                    });
                }
            }
        }
    }

    let json = serde_json::to_string_pretty(&results)?;
    println!("{json}");

    Ok(())
}

fn run_tray(config: config::AppConfig) -> Result<()> {
    let data: Arc<RwLock<Vec<UsageData>>> = Arc::new(RwLock::new(Vec::new()));
    let last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>> =
        Arc::new(RwLock::new(chrono::Utc::now()));

    let tray_controller =
        Arc::new(tray::TrayController::new().expect("Failed to create tray controller"));

    // Set initial loading icon before data is fetched
    let initial_icon = {
        let d = data.read();
        icon::generate_icon(&d)
    };
    if let Ok(hicon) = initial_icon.to_hicon() {
        tray_controller.update_icon(hicon);
    }

    let refresh_interval = config.general.refresh_interval;
    let data_bg = data.clone();
    let last_refresh_bg = last_refresh.clone();
    let config_bg = config.clone();
    let tc_bg = tray_controller.clone();

    // Spawn background refresh thread
    std::thread::spawn(move || {
        let bg_rt = tokio::runtime::Runtime::new().expect("Failed to create background runtime");
        // Set last_fetch such that the first loop iteration triggers an immediate fetch
        let mut last_fetch =
            std::time::Instant::now() - std::time::Duration::from_secs(refresh_interval + 1);
        let min_refresh_interval = refresh_interval.max(10);
        loop {
            let forced = tray::REFRESH_REQUESTED.swap(false, Ordering::SeqCst);
            let now = std::time::Instant::now();
            if forced || now.duration_since(last_fetch).as_secs() >= min_refresh_interval {
                bg_rt.block_on(fetch_all_providers(
                    &config_bg,
                    data_bg.clone(),
                    last_refresh_bg.clone(),
                ));

                // Regenerate HICON
                let d = data_bg.read();
                let new_icon = icon::generate_icon(&d);
                if let Ok(hicon) = new_icon.to_hicon() {
                    tc_bg.update_icon(hicon);
                }

                // Notify the main window to redraw with new data
                if let Some(&main_hwnd) = tray::MAIN_HWND.get() {
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                            Some(main_hwnd.0),
                            tray::WM_APP_UPDATE_DATA,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
                last_fetch = std::time::Instant::now(); // Update last_fetch here after a successful run
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });

    // Spawn the eframe UI window on a separate thread
    let data_window = data.clone();
    let last_refresh_window = last_refresh.clone();
    let config_window = config.clone();

    std::thread::spawn(move || {
        let win_w = 400.0_f32;
        let win_h = 520.0_f32;
        let pos = compute_popup_position(win_w, win_h);

        let app = app::QuotifyApp::new(data_window, last_refresh_window, config_window);

        let native_options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([win_w, win_h])
                .with_position(eframe::egui::pos2(pos[0], pos[1]))
                .with_title("Quotify - AI Quota Monitor")
                .with_resizable(false)
                .with_decorations(false)
                .with_taskbar(false)
                .with_always_on_top()
                .with_transparent(true)
                .with_visible(false),
            event_loop_builder: Some(Box::new(|builder| {
                #[cfg(target_os = "windows")]
                {
                    builder.with_any_thread(true);
                }
            })),
            ..Default::default()
        };

        if let Err(err) = eframe::run_native(
            "Quotify",
            native_options,
            Box::new(move |cc| {
                let _ = EGUI_CONTEXT.set(cc.egui_ctx.clone());

                // Try to load Segoe UI Variable for true Windows 11 Fluent typography
                let font_path = std::path::Path::new("C:\\Windows\\Fonts\\SegUIVar.ttf");
                if let Ok(font_data) = std::fs::read(font_path) {
                    let mut fonts = eframe::egui::FontDefinitions::default();
                    fonts.font_data.insert(
                        "SegoeUIVariable".to_owned(),
                        std::sync::Arc::new(eframe::egui::FontData::from_owned(font_data).tweak(
                            eframe::egui::FontTweak {
                                scale: 1.05, // Slightly upscale to match expected reading size
                                ..Default::default()
                            },
                        )),
                    );
                    fonts
                        .families
                        .get_mut(&eframe::egui::FontFamily::Proportional)
                        .unwrap()
                        .insert(0, "SegoeUIVariable".to_owned());
                    cc.egui_ctx.set_fonts(fonts);
                }

                use windows::Win32::UI::Shell::SetWindowSubclass;
                use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
                let title = w!("Quotify - AI Quota Monitor");
                let mut hwnd = HWND(std::ptr::null_mut());

                for _ in 0..20 {
                    hwnd = unsafe { FindWindowW(None, title).unwrap_or(HWND::default()) };
                    if !hwnd.0.is_null() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }

                if !hwnd.0.is_null() {
                    let _ = tray::MAIN_HWND.set(tray::SendHWND(hwnd));
                    apply_mica_backdrop(hwnd);
                    unsafe {
                        let _ = SetWindowSubclass(hwnd, Some(main_window_subclass), 1, 0);
                        // Initially hide the window so it only pops up on click
                        use windows::Win32::UI::WindowsAndMessaging::ShowWindow;
                        let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
                    }
                } else {
                    tracing::error!("Could not find Quotify main window HWND");
                }

                Ok(Box::new(app))
            }),
        ) {
            tracing::error!("Detail window failed: {err}");
        }
    });

    // Run the main Win32 tray message loop (proper blocking pump)
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    Ok(())
}

fn compute_popup_position(win_w: f32, win_h: f32) -> [f32; 2] {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
        };
        use windows::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let mut pt = POINT { x: 0, y: 0 };
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }

        // Try to get actual tray icon rect
        let mut has_icon_rect = false;
        if let Some(&shwnd) = crate::tray::TRAY_HWND.get() {
            let identifier = NOTIFYICONIDENTIFIER {
                cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: shwnd.0,
                uID: 1,
                guidItem: Default::default(),
            };
            unsafe {
                if let Ok(rect) = Shell_NotifyIconGetRect(&identifier) {
                    has_icon_rect = true;
                    // Use icon center as the reference point instead of arbitrary cursor pos
                    pt.x = rect.left + (rect.right - rect.left) / 2;
                    pt.y = rect.top + (rect.bottom - rect.top) / 2;
                }
            }
        }

        unsafe {
            let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..std::mem::zeroed()
            };
            if GetMonitorInfoW(hmon, &mut mi).as_bool() {
                let work = mi.rcWork;
                let monitor = mi.rcMonitor;
                let margin = 12.0;

                // Determine taskbar position by comparing work area to monitor area
                if work.bottom < monitor.bottom {
                    // Taskbar is at the bottom
                    let mut x = if has_icon_rect {
                        // Fluent style: Align center to icon, but keep within screen bounds
                        pt.x as f32 - win_w / 2.0
                    } else {
                        pt.x as f32 - win_w / 2.0
                    };
                    x = x.clamp(
                        work.left as f32 + margin,
                        (work.right as f32 - win_w - margin).max(work.left as f32),
                    );
                    let y = work.bottom as f32 - win_h - margin;
                    return [x, y];
                } else if work.top > monitor.top {
                    // Taskbar is at the top
                    let mut x = if has_icon_rect {
                        pt.x as f32 - win_w / 2.0
                    } else {
                        pt.x as f32 - win_w / 2.0
                    };
                    x = x.clamp(
                        work.left as f32 + margin,
                        (work.right as f32 - win_w - margin).max(work.left as f32),
                    );
                    let y = work.top as f32 + margin;
                    return [x, y];
                } else if work.left > monitor.left {
                    // Taskbar is on the left
                    let x = work.left as f32 + margin;
                    let mut y = if has_icon_rect {
                        pt.y as f32 - win_h / 2.0
                    } else {
                        pt.y as f32 - win_h / 2.0
                    };
                    y = y.clamp(
                        work.top as f32 + margin,
                        (work.bottom as f32 - win_h - margin).max(work.top as f32),
                    );
                    return [x, y];
                } else if work.right < monitor.right {
                    // Taskbar is on the right
                    let x = work.right as f32 - win_w - margin;
                    let mut y = if has_icon_rect {
                        pt.y as f32 - win_h / 2.0
                    } else {
                        pt.y as f32 - win_h / 2.0
                    };
                    y = y.clamp(
                        work.top as f32 + margin,
                        (work.bottom as f32 - win_h - margin).max(work.top as f32),
                    );
                    return [x, y];
                } else {
                    // Fallback: Default to bottom right if taskbar is hidden or not detected
                    let mut x = pt.x as f32 - win_w / 2.0;
                    x = x.clamp(
                        work.left as f32 + margin,
                        (work.right as f32 - win_w - margin).max(work.left as f32),
                    );
                    let y = work.bottom as f32 - win_h - margin;
                    return [x, y];
                }
            }
        }

        [
            (pt.x as f32 - win_w / 2.0).max(0.0),
            (pt.y as f32 - win_h).max(0.0),
        ]
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (win_w, win_h);
        [100.0, 100.0]
    }
}

fn apply_mica_backdrop(hwnd: HWND) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Graphics::Dwm::{
            DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DwmSetWindowAttribute,
        };

        if !hwnd.0.is_null() {
            let backdrop_type = DWMSBT_MAINWINDOW.0;
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_SYSTEMBACKDROP_TYPE,
                    &backdrop_type as *const _ as *const _,
                    std::mem::size_of::<i32>() as u32,
                );
            }
        }
    }
}

unsafe extern "system" fn main_window_subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _ref_data: usize,
) -> LRESULT { unsafe {
    use windows::Win32::UI::Shell::DefSubclassProc;
    use windows::Win32::UI::WindowsAndMessaging::{
        SW_HIDE, SW_SHOW, SetForegroundWindow, ShowWindow, WA_INACTIVE, WM_ACTIVATE, WM_CLOSE,
        WM_DESTROY,
    };

    match msg {
        tray::WM_APP_SHOW => {
            let win_w = 400.0_f32;
            let win_h = 520.0_f32;
            let pos = compute_popup_position(win_w, win_h);

            use windows::Win32::UI::WindowsAndMessaging::{
                SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowPos,
            };
            let _ = SetWindowPos(
                hwnd,
                None,
                pos[0] as i32,
                pos[1] as i32,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
            );

            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);

            use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
            let _ = SetFocus(Some(hwnd));

            if let Some(ctx) = EGUI_CONTEXT.get() {
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(true));
            }

            LRESULT(0)
        }
        WM_ACTIVATE => {
            let active_state = (wparam.0 & 0xFFFF) as u32;
            if active_state == WA_INACTIVE {
                let _ = ShowWindow(hwnd, SW_HIDE);
                if let Some(ctx) = EGUI_CONTEXT.get() {
                    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
                }
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            if let Some(ctx) = EGUI_CONTEXT.get() {
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
            }
            LRESULT(0)
        }
        tray::WM_APP_UPDATE_DATA => {
            if let Some(ctx) = EGUI_CONTEXT.get() {
                ctx.request_repaint();
            }
            LRESULT(0)
        }
        tray::WM_APP_QUIT => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = windows::Win32::UI::Shell::RemoveWindowSubclass(
                hwnd,
                Some(main_window_subclass),
                1,
            );
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
        _ => DefSubclassProc(hwnd, msg, wparam, lparam),
    }
}}
