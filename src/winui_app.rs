use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use parking_lot::RwLock;
use windows::Win32::{Foundation::HWND, UI::Shell::SetWindowSubclass};
use windows_core::Interface;
use windows_reactor::{
    App, AsyncSetState, Backdrop, BrushBinding, Color, Component, Element, ElementExt, GridLength,
    ProgressBar, RenderCx, ScrollBarVisibility, SetState, ThemeRef, Thickness, VerticalAlignment,
    border, button, grid, hstack, scroll_viewer, text_block, vstack,
};

use crate::{
    config::AppConfig,
    provider::UsageData,
    ui_model::{
        ProviderStatus, format_credits_balance, provider_display_order, provider_status,
        reorder_provider, reset_time_text,
    },
};

static GLOBAL_REFRESH_SIGNAL: OnceLock<Arc<WinUiRefreshSignal>> = OnceLock::new();

windows_core::imp::define_interface!(
    IWindowNative,
    IWindowNative_Vtbl,
    0xeecdbf0e_bae9_4cb6_a68e_9598e1cb57bb
);
windows_core::imp::interface_hierarchy!(IWindowNative, windows_core::IUnknown);

impl IWindowNative {
    unsafe fn get_window_handle(
        &self,
        hwnd: *mut *mut ::core::ffi::c_void,
    ) -> windows_core::Result<()> {
        unsafe {
            (windows_core::Interface::vtable(self).get_WindowHandle)(
                windows_core::Interface::as_raw(self),
                hwnd,
            )
            .ok()
        }
    }
}

#[repr(C)]
#[doc(hidden)]
#[allow(non_snake_case)]
pub struct IWindowNative_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub get_WindowHandle: unsafe extern "system" fn(
        *mut ::core::ffi::c_void,
        *mut *mut ::core::ffi::c_void,
    ) -> windows_core::HRESULT,
}

#[derive(Default)]
pub struct WinUiRefreshSignal {
    tick: AtomicU64,
    setter: parking_lot::Mutex<Option<AsyncSetState<u64>>>,
}

impl WinUiRefreshSignal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> u64 {
        self.tick.load(Ordering::SeqCst)
    }

    pub fn bind(&self, setter: AsyncSetState<u64>) {
        *self.setter.lock() = Some(setter);
    }

    pub fn notify(&self) {
        let tick = self.tick.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(setter) = self.setter.lock().clone() {
            setter.call(tick);
        }
    }
}

pub fn request_rerender() {
    if let Some(signal) = GLOBAL_REFRESH_SIGNAL.get() {
        signal.notify();
    }
}

pub fn run_window(
    data: Arc<RwLock<Vec<UsageData>>>,
    last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    config: AppConfig,
    config_path: Option<PathBuf>,
    active_provider: Arc<RwLock<String>>,
) -> anyhow::Result<()> {
    let _bootstrap_handle = windows_reactor::bootstrap::initialize()?;
    App::new()
        .title("Quotify - WinUI Preview")
        .run_custom(move |_app| {
            let root = WinUiRoot {
                data,
                last_refresh,
                config,
                config_path,
                active_provider,
                refresh_signal: None,
            };
            let host = windows_reactor::winui::host::ReactorHost::new_with_window_options(
                "Quotify - WinUI Preview",
                Some(windows_reactor::core::Size {
                    width: 420.0,
                    height: 560.0,
                }),
                windows_reactor::core::InnerConstraints {
                    min_width: Some(360.0),
                    min_height: Some(420.0),
                    ..Default::default()
                },
                Box::new(root),
                |_| {},
            )?;
            host.set_backdrop(Backdrop::Mica);
            host.activate()?;
            Box::leak(Box::new(host));
            Ok(())
        })?;
    Ok(())
}

pub fn run_popup_window(
    data: Arc<RwLock<Vec<UsageData>>>,
    last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    config: AppConfig,
    config_path: Option<PathBuf>,
    active_provider: Arc<RwLock<String>>,
    refresh_signal: Arc<WinUiRefreshSignal>,
) -> anyhow::Result<()> {
    let _bootstrap_handle = windows_reactor::bootstrap::initialize()?;
    let _ = GLOBAL_REFRESH_SIGNAL.set(refresh_signal.clone());
    App::new().title("Quotify").run_custom(move |_app| {
        let root = WinUiRoot {
            data,
            last_refresh,
            config,
            config_path,
            active_provider,
            refresh_signal: Some(refresh_signal),
        };
        let host = windows_reactor::winui::host::ReactorHost::new_with_window_options(
            "Quotify - AI Quota Monitor",
            Some(windows_reactor::core::Size {
                width: 400.0,
                height: 520.0,
            }),
            windows_reactor::core::InnerConstraints {
                min_width: Some(360.0),
                min_height: Some(420.0),
                ..Default::default()
            },
            Box::new(root),
            |_| {},
        )?;
        host.set_backdrop(Backdrop::Mica);
        host.activate()?;

        let hwnd = window_hwnd(host.window())?;
        let _ = crate::tray::MAIN_HWND.set(crate::tray::SendHWND::new(hwnd));
        crate::apply_rounded_window_region(hwnd);
        crate::move_popup_offscreen(hwnd);
        crate::set_dwm_cloak(hwnd, true);
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(crate::main_window_subclass), 1, 0);
        }

        Box::leak(Box::new(host));
        Ok(())
    })?;
    Ok(())
}

struct WinUiRoot {
    data: Arc<RwLock<Vec<UsageData>>>,
    last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    config: AppConfig,
    config_path: Option<PathBuf>,
    active_provider: Arc<RwLock<String>>,
    refresh_signal: Option<Arc<WinUiRefreshSignal>>,
}

impl Component for WinUiRoot {
    fn render(&self, _props: &(), cx: &mut RenderCx) -> Element {
        render_root(
            cx,
            self.data.clone(),
            self.last_refresh.clone(),
            self.config.clone(),
            self.config_path.clone(),
            self.active_provider.clone(),
            self.refresh_signal.clone(),
        )
    }
}

fn render_root(
    cx: &mut RenderCx,
    data: Arc<RwLock<Vec<UsageData>>>,
    last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    base_config: AppConfig,
    config_path: Option<PathBuf>,
    active_provider: Arc<RwLock<String>>,
    refresh_signal: Option<Arc<WinUiRefreshSignal>>,
) -> Element {
    let (order, set_order) = cx.use_state(base_config.general.provider_order.clone());
    let (active, set_active) = cx.use_state(active_provider.read().clone());
    let (dragging_provider, set_dragging_provider) = cx.use_state(None::<String>);
    if let Some(signal) = refresh_signal.as_ref() {
        let (_tick, set_tick) = cx.use_async_state(signal.current());
        signal.bind(set_tick);
    }

    let set_main = {
        let refresh_signal = refresh_signal.clone();
        move || {
            crate::tray::ACTIVE_PAGE.store(0, Ordering::SeqCst);
            if let Some(signal) = refresh_signal.as_ref() {
                signal.notify();
            }
        }
    };
    let set_about = {
        let refresh_signal = refresh_signal.clone();
        move || {
            crate::tray::ACTIVE_PAGE.store(1, Ordering::SeqCst);
            if let Some(signal) = refresh_signal.as_ref() {
                signal.notify();
            }
        }
    };

    let last = *last_refresh.read();
    let elapsed = (chrono::Utc::now() - last).num_seconds();
    let refresh_age = if elapsed < 60 {
        format!("{}s ago", elapsed.max(0))
    } else {
        format!("{}m ago", elapsed / 60)
    };

    let header = hstack((
        text_block("Quotify").font_size(22.0).bold(),
        text_block(format!("Refreshed {refresh_age}"))
            .font_size(12.0)
            .opacity(0.72),
        button("Providers").on_click(set_main),
        button("About").on_click(set_about),
    ))
    .spacing(10.0)
    .vertical_alignment(VerticalAlignment::Center);

    let page = crate::tray::ACTIVE_PAGE.load(Ordering::SeqCst);
    let body = if page == 1 {
        render_about_page()
    } else {
        let mut config = base_config.clone();
        config.general.provider_order = order.clone();
        let snapshot = data.read().clone();
        let cards: Vec<Element> = provider_display_order(&config)
            .into_iter()
            .map(|(provider_id, display_name)| {
                let provider_data = snapshot
                    .iter()
                    .find(|d| d.provider.eq_ignore_ascii_case(&provider_id));
                render_provider_card(
                    provider_id,
                    display_name,
                    provider_data,
                    active.clone(),
                    active_provider.clone(),
                    base_config.clone(),
                    config_path.clone(),
                    order.clone(),
                    set_order.clone(),
                    set_active.clone(),
                    dragging_provider.clone(),
                    set_dragging_provider.clone(),
                    refresh_signal.clone(),
                )
            })
            .collect();

        scroll_viewer(vstack(cards).spacing(8.0))
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
            .into()
    };

    let header: Element = header.into();

    border(vstack((header, body)).spacing(12.0))
        .padding(Thickness::uniform(14.0))
        .background(ThemeRef::LayerFill)
        .into()
}

#[allow(clippy::too_many_arguments)]
fn render_provider_card(
    provider_id: String,
    display_name: &'static str,
    data: Option<&UsageData>,
    active: String,
    active_provider: Arc<RwLock<String>>,
    base_config: AppConfig,
    config_path: Option<PathBuf>,
    order: Vec<String>,
    set_order: SetState<Vec<String>>,
    set_active: SetState<String>,
    dragging_provider: Option<String>,
    set_dragging_provider: SetState<Option<String>>,
    refresh_signal: Option<Arc<WinUiRefreshSignal>>,
) -> Element {
    let status = provider_status(data);
    let credits = data.and_then(|d| d.credits.as_ref());
    let windows = data.map(|d| d.windows.clone()).unwrap_or_default();
    let error = data.and_then(|d| d.error.clone());
    let is_active = active.eq_ignore_ascii_case(&provider_id);
    let is_dragging = dragging_provider
        .as_deref()
        .is_some_and(|id| id.eq_ignore_ascii_case(&provider_id));

    let mut rows: Vec<Element> = vec![
        hstack((
            provider_initial(display_name),
            text_block(display_name).font_size(15.0).bold(),
            status_badge(status),
            if is_dragging {
                let moving: Element = text_block("Moving").font_size(11.0).bold().into();
                moving
            } else {
                let empty: Element = text_block("").font_size(1.0).into();
                empty
            },
            if is_active {
                let primary: Element = text_block("Primary").font_size(11.0).bold().into();
                primary
            } else {
                let id = provider_id.clone();
                let active_provider = active_provider.clone();
                let config_path = config_path.clone();
                let config = base_config.clone();
                let set_active = set_active.clone();
                let primary_refresh_signal = refresh_signal.clone();
                button("Set primary")
                    .subtle()
                    .on_click(move || {
                        save_active_provider(
                            &id,
                            &active_provider,
                            config.clone(),
                            config_path.clone(),
                        );
                        set_active.call(id.clone());
                        if let Some(signal) = primary_refresh_signal.as_ref() {
                            signal.notify();
                        }
                    })
                    .into()
            },
        ))
        .spacing(8.0)
        .vertical_alignment(VerticalAlignment::Center)
        .into(),
    ];

    if let Some(c) = credits {
        rows.push(
            text_block(format!(
                "Credits: {} {}",
                format_credits_balance(c.balance),
                c.currency
            ))
            .font_size(12.0)
            .opacity(0.82)
            .into(),
        );
    }

    match status {
        ProviderStatus::Disabled => {
            rows.push(
                text_block("No usage data yet.")
                    .font_size(12.0)
                    .opacity(0.62)
                    .into(),
            );
        }
        ProviderStatus::Error => {
            rows.push(
                border(text_block(
                    error.unwrap_or_else(|| "Unknown provider error".to_string()),
                ))
                .background(Color::rgb(253, 232, 232))
                .border_brush(Color::rgb(196, 43, 28))
                .border_thickness(Thickness::uniform(1.0))
                .corner_radius(6.0)
                .padding(Thickness::uniform(8.0))
                .into(),
            );
        }
        ProviderStatus::Active => {
            if windows.is_empty() {
                rows.push(
                    text_block("No active usage windows.")
                        .font_size(12.0)
                        .opacity(0.62)
                        .into(),
                );
            } else {
                for window in windows {
                    rows.push(usage_window_row(&window));
                }
            }
        }
    }

    let press_provider_id = provider_id.clone();
    let release_provider_id = provider_id.clone();
    let release_order = order.clone();
    let release_config = base_config.clone();
    let release_config_path = config_path.clone();
    let release_set_order = set_order.clone();
    let release_set_dragging_provider = set_dragging_provider.clone();
    let release_dragging_provider = dragging_provider.clone();
    let release_refresh_signal = refresh_signal.clone();

    let card_background = if is_dragging {
        BrushBinding::from(Color::rgb(232, 240, 255))
    } else {
        BrushBinding::from(ThemeRef::CardBackground)
    };

    border(vstack(rows).spacing(8.0))
        .background(card_background)
        .border_brush(ThemeRef::CardStroke)
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(8.0)
        .padding(Thickness::uniform(12.0))
        .on_pointer_pressed(move |info| {
            if info.is_left_button_pressed {
                set_dragging_provider.call(Some(press_provider_id.clone()));
            }
        })
        .on_pointer_released(move |_info| {
            let Some(dragged) = release_dragging_provider.clone() else {
                return;
            };

            if dragged != release_provider_id {
                let mut next = release_order.clone();
                if reorder_provider(&mut next, &dragged, &release_provider_id) {
                    save_provider_order(&next, release_config.clone(), release_config_path.clone());
                    release_set_order.call(next);
                }
            }

            release_set_dragging_provider.call(None);
            if let Some(signal) = release_refresh_signal.as_ref() {
                signal.notify();
            }
        })
        .into()
}

fn provider_initial(display_name: &str) -> Element {
    let initial = display_name.chars().next().unwrap_or('?').to_string();
    border(text_block(initial).font_size(13.0).bold())
        .width(28.0)
        .height(28.0)
        .corner_radius(14.0)
        .background(Color::rgb(232, 240, 255))
        .padding(Thickness::uniform(5.0))
        .into()
}

fn status_badge(status: ProviderStatus) -> Element {
    let (label, bg, fg) = match status {
        ProviderStatus::Active => ("ACTIVE", Color::rgb(225, 244, 229), Color::rgb(16, 124, 65)),
        ProviderStatus::Error => ("ERROR", Color::rgb(253, 232, 232), Color::rgb(196, 43, 28)),
        ProviderStatus::Disabled => (
            "OFFLINE",
            Color::rgb(243, 243, 243),
            Color::rgb(118, 118, 118),
        ),
    };
    border(text_block(label).font_size(10.0).bold().foreground(fg))
        .background(bg)
        .corner_radius(4.0)
        .padding(Thickness::xy(6.0, 2.0))
        .into()
}

fn usage_window_row(window: &crate::provider::UsageWindow) -> Element {
    let pct = window.used_percent.clamp(0.0, 100.0);
    grid((
        text_block(&window.label)
            .font_size(12.0)
            .bold()
            .grid_column(0),
        ProgressBar::new(pct).range(0.0, 100.0).grid_column(1),
        text_block(format!("{pct:.0}%"))
            .font_size(12.0)
            .bold()
            .grid_column(2),
        text_block(reset_time_text(window.resets_at))
            .font_size(12.0)
            .opacity(0.65)
            .grid_column(3),
    ))
    .columns([
        GridLength::Pixel(86.0),
        GridLength::Star(1.0),
        GridLength::Pixel(44.0),
        GridLength::Pixel(72.0),
    ])
    .column_spacing(8.0)
    .into()
}

fn render_about_page() -> Element {
    border(
        vstack((
            text_block("Quotify").font_size(26.0).bold(),
            text_block(format!("Version: {}", env!("GIT_TAG"))).font_size(13.0),
            text_block("Author: zuoxinyu").font_size(13.0),
            text_block("WinUI reactor preview")
                .font_size(13.0)
                .opacity(0.72),
        ))
        .spacing(8.0),
    )
    .background(ThemeRef::CardBackground)
    .border_brush(ThemeRef::CardStroke)
    .border_thickness(Thickness::uniform(1.0))
    .corner_radius(8.0)
    .padding(Thickness::uniform(16.0))
    .into()
}

fn save_active_provider(
    provider_id: &str,
    active_provider: &Arc<RwLock<String>>,
    mut config: AppConfig,
    config_path: Option<PathBuf>,
) {
    *active_provider.write() = provider_id.to_string();
    config.general.active_provider = provider_id.to_string();
    save_config(config, config_path);
}

fn save_provider_order(order: &[String], mut config: AppConfig, config_path: Option<PathBuf>) {
    config.general.provider_order = order.to_vec();
    save_config(config, config_path);
}

fn save_config(config: AppConfig, config_path: Option<PathBuf>) {
    let result = if let Some(path) = config_path {
        config.save_to(&path)
    } else {
        config.save()
    };
    if let Err(err) = result {
        tracing::error!("Failed to save WinUI config change: {err}");
    }
}

fn window_hwnd(window: &impl Interface) -> windows_core::Result<HWND> {
    let native: IWindowNative = window.cast()?;
    let mut raw: *mut ::core::ffi::c_void = std::ptr::null_mut();
    unsafe { native.get_window_handle(&mut raw as *mut _)? };
    Ok(HWND(raw))
}
