use eframe::egui;
use parking_lot::RwLock;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::provider::UsageData;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate {
        latest_version: String,
    },
    NewVersionAvailable {
        latest_version: String,
        release_url: String,
    },
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
enum ProviderTestStatus {
    Idle,
    Testing {
        provider: String,
    },
    Success {
        provider: String,
        fetched_at: chrono::DateTime<chrono::Utc>,
        summary: String,
    },
    Error {
        provider: String,
        message: String,
    },
}

pub struct QuotifyApp {
    pub data: Arc<RwLock<Vec<UsageData>>>,
    pub last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    pub history: Arc<RwLock<crate::usage_history::UsageHistory>>,
    pub config: crate::config::AppConfig,
    pub config_path: Option<PathBuf>,
    pub active_provider: Arc<RwLock<String>>,
    drag: ProviderDragState,
    last_config_reload: Instant,
    update_status: Arc<parking_lot::Mutex<UpdateStatus>>,
    provider_test_status: Arc<parking_lot::Mutex<ProviderTestStatus>>,
    selected_setting_provider: String,
    show_secrets: bool,
}

impl QuotifyApp {
    pub fn new(
        data: Arc<RwLock<Vec<UsageData>>>,
        last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
        config: crate::config::AppConfig,
        config_path: Option<PathBuf>,
        active_provider: Arc<RwLock<String>>,
        history: Arc<RwLock<crate::usage_history::UsageHistory>>,
    ) -> Self {
        Self {
            data,
            last_refresh,
            history,
            config,
            config_path,
            active_provider,
            drag: ProviderDragState::default(),
            last_config_reload: Instant::now(),
            update_status: Arc::new(parking_lot::Mutex::new(UpdateStatus::Idle)),
            provider_test_status: Arc::new(parking_lot::Mutex::new(ProviderTestStatus::Idle)),
            selected_setting_provider: "openai".to_string(),
            show_secrets: false,
        }
    }
}

#[derive(Default)]
struct ProviderDragState {
    held_provider: Option<String>,
    hold_started: Option<Instant>,
    dragging: Option<ProviderDragPayload>,
    pointer_offset: Option<egui::Vec2>,
    card_size: Option<egui::Vec2>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProviderDragPayload {
    provider: String,
    row: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProviderDropTarget {
    Item { provider: String, row: usize },
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderStatus {
    Active,
    Error,
    Disabled,
}

impl eframe::App for QuotifyApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Fully transparent so the DWM Mica backdrop shows through
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui_extras::install_image_loaders(&ctx);

        let active_page = crate::tray::ACTIVE_PAGE.load(std::sync::atomic::Ordering::SeqCst);
        if active_page != 2 {
            self.reload_config_if_due();
        }

        // Redraw every second to update the "Refreshed X seconds ago" counter,
        // but ONLY if the window is active/focused. When the window loses focus,
        // it hides itself. If we request repaint while hidden, winit's swapchain
        // will instantly fail and cause a 100% CPU busy loop trying to VSync.
        let is_visible = crate::tray::WINDOW_VISIBLE.load(std::sync::atomic::Ordering::SeqCst);
        if is_visible && ctx.input(|i| i.focused) {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }

        // Query the OS/system theme to support dynamic light/dark mode switching
        let is_dark = match self.config.general.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => match ctx.system_theme() {
                Some(egui::Theme::Dark) => true,
                Some(egui::Theme::Light) => false,
                None => ctx.global_style().visuals.dark_mode,
            },
        };

        // Update Windows DWM immersive dark mode attribute to match the calculated theme
        if let Some(send_hwnd) = crate::tray::MAIN_HWND.get() {
            let hwnd = send_hwnd.raw();
            use windows::Win32::Graphics::Dwm::{
                DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
            };
            let dark = if is_dark { 1_i32 } else { 0_i32 };
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    &dark as *const _ as *const _,
                    std::mem::size_of::<i32>() as u32,
                );
            }
        }

        let is_mica = crate::IS_MICA_ACTIVE.load(std::sync::atomic::Ordering::SeqCst);
        let mut visuals = if is_dark {
            let mut v = egui::Visuals::dark();
            // Solid window fill for dropdown menus and popups to ensure they are opaque
            v.window_fill = egui::Color32::from_rgb(45, 45, 45);
            v.panel_fill = if is_mica {
                egui::Color32::TRANSPARENT
            } else {
                egui::Color32::from_rgb(32, 32, 32)
            };
            v.extreme_bg_color = if is_mica {
                egui::Color32::from_rgba_premultiplied(32, 32, 32, 180)
            } else {
                egui::Color32::from_rgb(32, 32, 32)
            };

            // Semi-transparent Acrylic Plate card backgrounds (Dark mode).
            // Cards stay fairly opaque for text contrast, while the panel
            // background between them is transparent to show the Mica backdrop.
            v.widgets.noninteractive.bg_fill =
                egui::Color32::from_rgba_premultiplied(45, 45, 45, 200);
            v.widgets.inactive.bg_fill = egui::Color32::from_rgba_premultiplied(50, 50, 50, 200);
            v.widgets.hovered.bg_fill = egui::Color32::from_rgba_premultiplied(56, 56, 56, 220);
            v.widgets.active.bg_fill = egui::Color32::from_rgba_premultiplied(62, 62, 62, 230);

            // Windows 11 Card plate borders (Dark mode)
            v.widgets.noninteractive.bg_stroke =
                egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(60, 60, 60, 160));
            v.widgets.inactive.bg_stroke =
                egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(70, 70, 70, 160));
            v
        } else {
            let mut v = egui::Visuals::light();
            // Solid window fill for dropdown menus and popups to ensure they are opaque
            v.window_fill = egui::Color32::from_rgb(255, 255, 255);
            v.panel_fill = if is_mica {
                egui::Color32::TRANSPARENT
            } else {
                egui::Color32::from_rgb(243, 243, 243)
            };
            v.extreme_bg_color = if is_mica {
                egui::Color32::from_rgba_premultiplied(229, 229, 229, 200)
            } else {
                egui::Color32::from_rgb(229, 229, 229)
            };

            // Semi-transparent Acrylic Plate card backgrounds (Light mode).
            // Cards stay fairly opaque for text contrast, while the panel
            // background between them is transparent to show the Mica backdrop.
            v.widgets.noninteractive.bg_fill =
                egui::Color32::from_rgba_premultiplied(255, 255, 255, 180);
            v.widgets.inactive.bg_fill = egui::Color32::from_rgba_premultiplied(229, 229, 229, 255);
            v.widgets.hovered.bg_fill = egui::Color32::from_rgba_premultiplied(243, 243, 243, 200);
            v.widgets.active.bg_fill = egui::Color32::from_rgba_premultiplied(235, 235, 235, 220);

            // Windows 11 Card plate borders (Light mode)
            v.widgets.noninteractive.bg_stroke = egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_premultiplied(229, 229, 229, 180),
            );
            v.widgets.inactive.bg_stroke = egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_premultiplied(240, 240, 240, 180),
            );
            v
        };

        // Standard Windows 11 layout corner roundings
        visuals.window_corner_radius = 12.into(); // standard Win11 window rounding
        visuals.widgets.noninteractive.corner_radius = 8.into(); // standard Win11 card rounding
        visuals.widgets.inactive.corner_radius = 6.into(); // standard Win11 control rounding
        visuals.widgets.hovered.corner_radius = 6.into();
        visuals.widgets.active.corner_radius = 6.into();
        visuals.popup_shadow = egui::Shadow::NONE;
        visuals.window_shadow = egui::Shadow::NONE;

        // Set WinUI 3 typography metrics
        let mut style = (*ctx.global_style()).clone();
        style.text_styles = [
            (
                egui::TextStyle::Heading,
                egui::FontId::new(20.0, egui::FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Name("Title".into()),
                egui::FontId::new(28.0, egui::FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Body,
                egui::FontId::new(14.0, egui::FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Monospace,
                egui::FontId::new(14.0, egui::FontFamily::Monospace),
            ),
            (
                egui::TextStyle::Button,
                egui::FontId::new(14.0, egui::FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Small,
                egui::FontId::new(12.0, egui::FontFamily::Proportional),
            ),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        // Disable the top/bottom fade-out gradients on the card scroll area.
        style.spacing.scroll.fade.strength = 0.0;
        ctx.set_global_style(style);

        ctx.set_visuals(visuals);

        // Semi-transparent popup panel to let native Mica show through.
        // We let Windows DWM handle the window rounded corners and native border/shadow,
        // avoiding drawing a second rounded border in egui to prevent mismatched curvatures.
        // Native Win11 flyout: cards stay opaque for contrast, the panel
        // background is fully transparent so the Mica backdrop shows through
        // the gaps between cards. Without Mica we fall back to a solid fill.
        let panel_bg = if is_dark {
            if is_mica {
                egui::Color32::TRANSPARENT
            } else {
                egui::Color32::from_rgb(32, 32, 32)
            }
        } else {
            if is_mica {
                egui::Color32::TRANSPARENT
            } else {
                egui::Color32::from_rgb(243, 243, 243)
            }
        };

        let popup_frame = egui::Frame::NONE
            .fill(panel_bg)
            .corner_radius(12)
            .inner_margin(12)
            .outer_margin(0);

        egui::CentralPanel::default()
            .frame(popup_frame)
            .show_inside(ui, |ui| {
                let content_width = ui.available_width();
                let card_width = (content_width - 2.0).clamp(0.0, 352.0);
                let card_left_indent = ((content_width - card_width) / 2.0).max(0.0);
                let last = *self.last_refresh.read();
                let elapsed = (chrono::Utc::now() - last).num_seconds();
                let refresh_age = if elapsed < 60 {
                    format!("{}s ago", elapsed.max(0))
                } else {
                    format!("{}m ago", elapsed / 60)
                };

                let active_page = crate::tray::ACTIVE_PAGE.load(std::sync::atomic::Ordering::SeqCst);

                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(card_left_indent);

                        if active_page == 1 {
                            ui.horizontal_centered(|ui| {
                                let back_btn = ui.add_sized(
                                    egui::vec2(24.0, 24.0),
                                    egui::Button::new(
                                        egui::RichText::new("\u{E72B}")
                                            .size(12.0)
                                    )
                                    .frame(true)
                                    .corner_radius(12)
                                );
                                if back_btn.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                if back_btn.clicked() {
                                    crate::tray::ACTIVE_PAGE.store(0, std::sync::atomic::Ordering::SeqCst);
                                    ctx.request_repaint();
                                }
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new("About")
                                        .strong()
                                        .size(16.0)
                                        .line_height(Some(24.0)),
                                );
                            });
                        } else if active_page == 2 {
                            ui.horizontal_centered(|ui| {
                                let back_btn = ui.add_sized(
                                    egui::vec2(24.0, 24.0),
                                    egui::Button::new(
                                        egui::RichText::new("\u{E72B}")
                                            .size(12.0)
                                    )
                                    .frame(true)
                                    .corner_radius(12)
                                );
                                if back_btn.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                if back_btn.clicked() {
                                    crate::tray::ACTIVE_PAGE.store(0, std::sync::atomic::Ordering::SeqCst);
                                    ctx.request_repaint();
                                }
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new("Settings")
                                        .strong()
                                        .size(16.0)
                                        .line_height(Some(24.0)),
                                );
                            });
                        } else {
                            let header_response = ui.horizontal_centered(|ui| {
                                let logo = egui::Image::new(egui::include_image!(
                                    "../assets/icons/quotify.svg"
                                ))
                                .fit_to_exact_size(egui::vec2(18.0, 18.0))
                                .maintain_aspect_ratio(true);
                                ui.add(logo);

                                ui.add_space(6.0);

                                ui.label(
                                    egui::RichText::new("Quotify")
                                        .strong()
                                        .size(16.0)
                                        .line_height(Some(24.0)),
                                );
                            });

                            let header_interact = ui.interact(
                                header_response.response.rect,
                                header_response.response.id,
                                egui::Sense::click(),
                            );
                            if header_interact.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if header_interact.clicked() {
                                crate::tray::ACTIVE_PAGE.store(1, std::sync::atomic::Ordering::SeqCst);
                                ctx.request_repaint();
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let settings = ui.add_sized(
                                    egui::vec2(24.0, 24.0),
                                    egui::Button::new(
                                        egui::RichText::new("\u{E713}")
                                            .strong()
                                            .size(12.0)
                                    )
                                    .frame(false)
                                    .corner_radius(4)
                                ).on_hover_text("Settings");

                                if settings.clicked() {
                                    crate::tray::ACTIVE_PAGE.store(2, std::sync::atomic::Ordering::SeqCst);
                                    ctx.request_repaint();
                                }

                                ui.add_space(2.0);

                                let refresh = ui.add_sized(
                                    egui::vec2(24.0, 24.0),
                                    egui::Button::new(
                                        egui::RichText::new("\u{E72C}")
                                            .strong()
                                            .size(12.0)
                                    )
                                    .frame(false)
                                    .corner_radius(4)
                                ).on_hover_text("Refresh usage now");

                                if refresh.clicked() {
                                    crate::tray::request_refresh();
                                    ctx.request_repaint();
                                }

                                ui.add_space(4.0);

                                ui.add_sized(
                                    [60.0, 24.0],
                                    egui::Label::new(
                                        egui::RichText::new(refresh_age)
                                            .small()
                                            .color(ui.visuals().weak_text_color()),
                                    )
                                    .truncate(),
                                );
                            });
                        }
                    },
                );

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(8.0);

                if active_page == 1 {
                    self.render_about_page(ui, &ctx, card_width, card_left_indent);
                } else if active_page == 2 {
                    self.render_settings_page(ui, &ctx, card_width, card_left_indent);
                } else {
                    let provider_drag_active =
                        self.drag.held_provider.is_some() || self.drag.dragging.is_some();
                    let scroll_source = egui::containers::scroll_area::ScrollSource {
                        drag: !provider_drag_active,
                        ..egui::containers::scroll_area::ScrollSource::ALL
                    };
                    let _scroll_output = egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .hscroll(false)
                        .scroll_source(scroll_source)
                        .show(ui, |ui| {
                            let data = self.data.read().clone();
                            let all_providers = provider_display_order(&self.config);
                            let visible_providers = all_providers
                                .into_iter()
                                .filter(|(name, _)| data.iter().any(|d| d.provider == *name))
                                .collect::<Vec<_>>();

                            // Compute the scrollbar's interactive rect — matches egui's own
                            // `max_bar_rect` for the floating vertical scrollbar. When the
                            // pointer is inside this area the scrollbar owns the interaction,
                            // so we must not arm or continue any card drag.
                            let scrollbar_left = provider_scrollbar_left(ui);
                            let pointer_in_scrollbar = ctx.input(|i| {
                                i.pointer
                                    .hover_pos()
                                    .is_some_and(|pos| pos.x >= scrollbar_left)
                            });

                            if pointer_in_scrollbar {
                                // Scrollbar owns the interaction — cancel any pending card hold
                                // so a long-press that drifts onto the bar doesn't arm a drag.
                                self.drag.held_provider = None;
                                self.drag.hold_started = None;
                            }

                            let mut drop_target = None;
                            let mut last_card_bottom = None;
                            for (row_idx, (name, display_name)) in
                                visible_providers.iter().enumerate()
                            {
                                let Some(provider_data) = data.iter().find(|d| d.provider == *name) else {
                                    continue;
                                };
                                // Keep all cards at full height while dragging so positions stay
                                // stable and autoscroll moves smoothly. Only the dragged card is
                                // dimmed via set_opacity in render_provider.
                                let is_dragged = self
                                    .drag
                                    .dragging
                                    .as_ref()
                                    .is_some_and(|payload| payload.provider == *name);
                                let response = render_provider(
                                    ui,
                                    name,
                                    display_name,
                                    Some(provider_data),
                                    card_width,
                                    &self.active_provider,
                                    &mut self.config,
                                    self.config_path.as_ref(),
                                    &self.data,
                                    &self.history,
                                    false,
                                    is_dragged,
                                );
                                if !pointer_in_scrollbar {
                                    drop_target = drop_target.or_else(|| {
                                        self.handle_provider_drag(&ctx, ui, &response, name, row_idx)
                                    });
                                }
                                last_card_bottom = Some(response.rect.bottom());
                                ui.add_space(6.0);
                            }

                            self.autoscroll_provider_drag(ui, &ctx, card_width);

                            if drop_target.is_none()
                                && self.drag.dragging.is_some()
                                && !ctx.input(|i| i.pointer.primary_down())
                                && ctx.input(|i| {
                                    i.pointer.hover_pos().is_some_and(|pos| {
                                        ui.clip_rect().contains(pos)
                                            && last_card_bottom.is_some_and(|bottom| pos.y > bottom)
                                    })
                                })
                            {
                                drop_target = Some(ProviderDropTarget::End);
                            }

                            let visible_provider_names = visible_providers
                                .iter()
                                .map(|(name, _)| name.as_str())
                                .collect::<Vec<_>>();
                            self.finish_provider_drag_if_released(
                                &ctx,
                                drop_target,
                                &visible_provider_names,
                            );

                            if visible_providers.is_empty() {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(48.0);
                                    ui.label(
                                        egui::RichText::new(
                                            "No enabled providers. Configure credentials to enable cards.",
                                        )
                                        .color(ui.visuals().weak_text_color()),
                                    );
                                });
                            }
                        });
                }

                // If a card is being dragged, render a floating preview of the card that follows the mouse cursor
                if let Some(dragging) = &self.drag.dragging {
                    if let Some(pointer_pos) = ctx.input(|i| i.pointer.hover_pos()) {
                        let preview_rect = self.dragged_provider_rect(pointer_pos, card_width);

                        egui::Area::new(egui::Id::new("provider_drag_preview"))
                            .fixed_pos(preview_rect.min)
                            .order(egui::Order::Tooltip)
                            .interactable(false)
                            .show(&ctx, |ui| {
                                ui.style_mut().visuals.widgets.noninteractive.bg_fill = ui
                                    .style_mut()
                                    .visuals
                                    .widgets
                                    .noninteractive
                                    .bg_fill
                                    .linear_multiply(0.85);

                                let data = self.data.read().clone();
                                if let Some(provider_data) =
                                    data.iter().find(|d| d.provider == dragging.provider)
                                {
                                    let display_name = provider_catalog()
                                        .iter()
                                        .find(|(id, _)| id.eq_ignore_ascii_case(&dragging.provider))
                                        .map(|(_, d)| *d)
                                        .unwrap_or(&dragging.provider)
                                        .to_string();

                                    let status = match provider_data.error.is_some() {
                                        true => ProviderStatus::Error,
                                        false => ProviderStatus::Active,
                                    };
                                    let credits = provider_data.credits.as_ref();
                                    let error_msg = provider_data.error.as_deref();
                                    let windows = &provider_data.windows;
                                    let is_dark = ui.visuals().dark_mode;

                                    let card_frame = egui::Frame::NONE
                                        .fill(ui.visuals().widgets.noninteractive.bg_fill)
                                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                                        .corner_radius(8)
                                        .inner_margin(egui::Margin::symmetric(10, 8));

                                    ui.allocate_ui(egui::vec2(card_width, 0.0), |ui| {
                                        ui.set_min_width(card_width);
                                        ui.set_max_width(card_width);
                                        render_provider_card(
                                            ui,
                                            &dragging.provider,
                                            &display_name,
                                            status,
                                            credits,
                                            error_msg,
                                            windows,
                                            is_dark,
                                            card_frame,
                                            card_width,
                                            &self.active_provider,
                                            &mut self.config,
                                            self.config_path.as_ref(),
                                            &self.data,
                                            &self.history,
                                            false,
                                        );
                                    });
                                }
                            });
                    }
                }
            });
    }
}

impl QuotifyApp {
    fn reload_config_if_due(&mut self) {
        if self.drag.dragging.is_some()
            || self.last_config_reload.elapsed() < Duration::from_secs(2)
        {
            return;
        }
        self.last_config_reload = Instant::now();

        let loaded = if let Some(path) = &self.config_path {
            crate::config::AppConfig::load_from(path)
        } else {
            crate::config::AppConfig::load()
        };

        match loaded {
            Ok(mut config) => {
                crate::secrets::hydrate_config(&mut config);
                self.config = config;
                *self.active_provider.write() =
                    self.config.general.active_provider.trim().to_string();
            }
            Err(err) => tracing::debug!("Failed to reload UI config: {err}"),
        }
    }

    fn save_config(&self) {
        let result = save_config_without_secrets(&self.config, self.config_path.as_ref());
        if let Err(err) = result {
            tracing::error!("Failed to save config: {err}");
        }
    }

    fn render_provider_test_controls(&self, ui: &mut egui::Ui, provider_id: &str) {
        let status = self.provider_test_status.lock().clone();
        let testing_this = matches!(
            &status,
            ProviderTestStatus::Testing { provider } if provider == provider_id
        );
        let testing_other = matches!(&status, ProviderTestStatus::Testing { .. }) && !testing_this;

        ui.horizontal(|ui| {
            let button = egui::Button::new(if testing_this {
                "Testing..."
            } else {
                "Test Provider"
            })
            .min_size(egui::vec2(104.0, 26.0));

            if ui
                .add_enabled(!testing_this && !testing_other, button)
                .clicked()
            {
                self.trigger_provider_test(provider_id.to_string(), ui.ctx().clone());
            }

            if testing_this {
                ui.spinner();
            }
        });

        match status {
            ProviderTestStatus::Success {
                provider,
                fetched_at,
                summary,
            } if provider == provider_id => {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Test passed at {}. {summary}",
                        fetched_at.with_timezone(&chrono::Local).format("%H:%M:%S")
                    ))
                    .small()
                    .color(egui::Color32::from_rgb(46, 125, 50)),
                );
            }
            ProviderTestStatus::Error { provider, message } if provider == provider_id => {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!("Test failed: {message}"))
                        .small()
                        .color(egui::Color32::from_rgb(198, 40, 40)),
                );
            }
            ProviderTestStatus::Testing { provider } if provider == provider_id => {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Fetching usage with the current provider settings...")
                        .small()
                        .weak(),
                );
            }
            _ => {}
        }
    }

    fn trigger_provider_test(&self, provider_id: String, ctx: egui::Context) {
        self.save_config();

        let mut config = self.config.clone();
        crate::secrets::hydrate_config(&mut config);
        enable_provider_for_test(&mut config, &provider_id);

        *self.provider_test_status.lock() = ProviderTestStatus::Testing {
            provider: provider_id.clone(),
        };

        let status = self.provider_test_status.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<UsageData> {
                let provider = crate::create_provider(&provider_id, &config).ok_or_else(|| {
                    anyhow::anyhow!("Provider could not be created from the current settings")
                })?;
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(provider.fetch_usage())
            })();

            *status.lock() = match result {
                Ok(data) => {
                    if let Some(error) = data.error.clone() {
                        ProviderTestStatus::Error {
                            provider: provider_id,
                            message: error,
                        }
                    } else {
                        ProviderTestStatus::Success {
                            provider: provider_id,
                            fetched_at: data.fetched_at,
                            summary: summarize_provider_test(&data),
                        }
                    }
                }
                Err(err) => ProviderTestStatus::Error {
                    provider: provider_id,
                    message: err.to_string(),
                },
            };
            ctx.request_repaint();
        });
    }

    fn dragged_provider_rect(&self, pointer_pos: egui::Pos2, fallback_width: f32) -> egui::Rect {
        let pointer_offset = self
            .drag
            .pointer_offset
            .unwrap_or_else(|| egui::vec2(fallback_width / 2.0, 12.0));
        let card_size = self
            .drag
            .card_size
            .unwrap_or_else(|| egui::vec2(fallback_width, 0.0));

        egui::Rect::from_min_size(pointer_pos - pointer_offset, card_size)
    }

    fn autoscroll_provider_drag(&self, ui: &mut egui::Ui, ctx: &egui::Context, card_width: f32) {
        if self.drag.dragging.is_none() {
            return;
        }

        let Some(pointer_pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };

        let viewport = ui.clip_rect();
        let dragged_rect = self.dragged_provider_rect(pointer_pos, card_width);
        let top_overflow = (viewport.min.y - dragged_rect.min.y).max(0.0);
        let bottom_overflow = (dragged_rect.max.y - viewport.max.y).max(0.0);
        let scroll_delta = if top_overflow > 0.0 {
            -provider_drag_scroll_step(top_overflow)
        } else if bottom_overflow > 0.0 {
            provider_drag_scroll_step(bottom_overflow)
        } else {
            0.0
        };

        if scroll_delta == 0.0 {
            return;
        }

        let target_y = if scroll_delta < 0.0 {
            viewport.min.y + scroll_delta
        } else {
            viewport.max.y + scroll_delta
        };
        let target_rect = egui::Rect::from_center_size(
            egui::pos2(viewport.center().x, target_y),
            egui::vec2(card_width, 1.0),
        );
        let align = if scroll_delta < 0.0 {
            egui::Align::TOP
        } else {
            egui::Align::BOTTOM
        };

        ui.scroll_to_rect(target_rect, Some(align));
        ctx.request_repaint();
    }

    fn handle_provider_drag(
        &mut self,
        ctx: &egui::Context,
        ui: &egui::Ui,
        response: &egui::Response,
        provider_name: &str,
        row_idx: usize,
    ) -> Option<ProviderDropTarget> {
        let pointer_down = ctx.input(|i| i.pointer.primary_down());
        let pointer_pos = ctx.input(|i| i.pointer.hover_pos());
        let contains_pointer = pointer_pos.is_some_and(|pos| response.rect.contains(pos));
        let can_start_from_card = response.contains_pointer();

        if self.drag.dragging.is_some() && contains_pointer {
            let insert_after = pointer_pos.is_some_and(|pos| pos.y >= response.rect.center().y);
            paint_provider_drop_preview(ui, response.rect, insert_after);

            if !pointer_down {
                return Some(ProviderDropTarget::Item {
                    provider: provider_name.to_string(),
                    row: row_idx + usize::from(insert_after),
                });
            }
        }

        if !pointer_down {
            return None;
        }

        let now = Instant::now();
        if can_start_from_card && self.drag.held_provider.is_none() {
            self.drag.held_provider = Some(provider_name.to_string());
            self.drag.hold_started = Some(now);
        }

        let pointer_moved = ctx.input(|i| i.pointer.delta().length_sq() > 0.25);
        if self.drag.held_provider.as_deref() == Some(provider_name)
            && self.drag.dragging.is_none()
            && self
                .drag
                .hold_started
                .is_some_and(|started| now.duration_since(started) >= Duration::from_millis(350))
            && pointer_moved
        {
            self.drag.dragging = Some(ProviderDragPayload {
                provider: provider_name.to_string(),
                row: row_idx,
            });
            self.drag.pointer_offset = pointer_pos.map(|pos| pos - response.rect.min);
            self.drag.card_size = Some(response.rect.size());
        }
        None
    }

    fn finish_provider_drag_if_released(
        &mut self,
        ctx: &egui::Context,
        drop_target: Option<ProviderDropTarget>,
        visible_providers: &[&str],
    ) {
        if ctx.input(|i| i.pointer.primary_down()) {
            return;
        }

        let changed =
            if let (Some(dragging), Some(target)) = (self.drag.dragging.as_ref(), drop_target) {
                reorder_provider(
                    &mut self.config.general.provider_order,
                    dragging,
                    target,
                    visible_providers,
                )
            } else {
                false
            };

        if changed {
            self.save_config();
            ctx.request_repaint();
        }
        self.drag = ProviderDragState::default();
    }
}

fn format_credits_balance(balance: f64) -> String {
    if balance.abs() >= 1_000_000_000.0 {
        let val = balance / 1_000_000_000.0;
        format!("{:.2}B", val)
    } else if balance.abs() >= 1_000_000.0 {
        let val = balance / 1_000_000.0;
        if (val - val.round()).abs() < 0.01 {
            format!("{:.0}M", val)
        } else {
            format!("{:.2}M", val)
        }
    } else if balance.abs() >= 1_000.0 {
        let val = balance / 1_000.0;
        if (val - val.round()).abs() < 0.01 {
            format!("{:.0}K", val)
        } else {
            format!("{:.2}K", val)
        }
    } else if (balance - balance.round()).abs() < 0.01 {
        format!("{:.0}", balance)
    } else {
        format!("{:.2}", balance)
    }
}

fn open_config_file(config_path: Option<&PathBuf>) -> anyhow::Result<()> {
    let path = config_path
        .cloned()
        .unwrap_or_else(crate::config::AppConfig::config_path);
    if !path.exists() {
        crate::config::AppConfig::default().save_to(&path)?;
    }

    std::process::Command::new("notepad.exe")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(anyhow::Error::from)
}

fn open_folder(path: &std::path::Path) -> anyhow::Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(anyhow::Error::from)
}

fn provider_catalog() -> &'static [(&'static str, &'static str)] {
    &[
        ("codex", "Codex"),
        ("openai", "OpenAI"),
        ("opencode", "OpenCode"),
        ("opencodego", "OpenCode Go"),
        ("claude", "Claude"),
        ("gemini", "Gemini"),
        ("antigravity", "Antigravity"),
        ("deepseek", "DeepSeek"),
        ("openrouter", "OpenRouter"),
        ("moonshot", "Moonshot"),
        ("elevenlabs", "ElevenLabs"),
        ("doubao", "Doubao"),
        ("zai", "z.ai"),
        ("venice", "Venice"),
        ("crof", "Crof"),
        ("synthetic", "Synthetic"),
        ("warp", "Warp"),
        ("groqcloud", "GroqCloud"),
        ("deepgram", "Deepgram"),
        ("llmproxy", "LLM Proxy"),
        ("codebuff", "Codebuff"),
        ("kiro", "Kiro"),
        ("copilot", "Copilot"),
        ("azureopenai", "Azure OpenAI"),
        ("ollama", "Ollama"),
        ("minimax", "MiniMax"),
        ("jetbrains", "JetBrains AI"),
        ("kimi", "Kimi"),
        ("kilo", "Kilo Code"),
        ("augment", "Augment"),
        ("bedrock", "AWS Bedrock"),
        ("vertexai", "Vertex AI"),
        ("stepfun", "StepFun"),
        ("abacus", "Abacus AI"),
        ("alibabatoken", "Alibaba Token"),
        ("t3chat", "T3 Chat"),
        ("amp", "Amp"),
        ("mistral", "Mistral"),
        ("grok", "Grok"),
        ("cursor", "Cursor"),
        ("droid", "Factory Droid"),
        ("windsurf", "Windsurf"),
        ("mimo", "MiMo"),
    ]
}

fn provider_display_order(config: &crate::config::AppConfig) -> Vec<(String, &'static str)> {
    let mut ordered = Vec::new();
    for configured in &config.general.provider_order {
        if let Some((id, display_name)) = provider_catalog()
            .iter()
            .find(|(id, _)| id.eq_ignore_ascii_case(configured))
            && !ordered.iter().any(|(existing, _)| existing == id)
        {
            ordered.push(((*id).to_string(), *display_name));
        }
    }

    for (id, display_name) in provider_catalog() {
        if !ordered.iter().any(|(existing, _)| existing == id) {
            ordered.push(((*id).to_string(), *display_name));
        }
    }

    ordered
}

fn ensure_provider_order(order: &mut Vec<String>) {
    let mut normalized = Vec::new();
    for configured in order.iter() {
        if let Some((id, _)) = provider_catalog()
            .iter()
            .find(|(id, _)| id.eq_ignore_ascii_case(configured))
            && !normalized.iter().any(|existing| existing == id)
        {
            normalized.push((*id).to_string());
        }
    }

    for (id, _) in provider_catalog() {
        if !normalized.iter().any(|existing| existing == id) {
            normalized.push((*id).to_string());
        }
    }

    *order = normalized;
}

fn reorder_provider(
    order: &mut Vec<String>,
    dragged: &ProviderDragPayload,
    target: ProviderDropTarget,
    visible_providers: &[&str],
) -> bool {
    ensure_provider_order(order);

    let mut visible_order = visible_providers
        .iter()
        .map(|provider| (*provider).to_string())
        .collect::<Vec<_>>();
    let Some(from) = visible_order
        .iter()
        .position(|provider| provider == &dragged.provider)
    else {
        return false;
    };

    let mut to = match target {
        ProviderDropTarget::Item { provider, row } => {
            if provider == dragged.provider {
                row
            } else {
                visible_order
                    .iter()
                    .position(|candidate| candidate == &provider)
                    .map(|target_row| {
                        if row > target_row {
                            target_row + 1
                        } else {
                            target_row
                        }
                    })
                    .unwrap_or(row)
            }
        }
        ProviderDropTarget::End => visible_order.len(),
    };

    if from < to {
        to -= 1;
    }
    to = to.min(visible_order.len().saturating_sub(1));
    if from == to {
        return false;
    }

    let item = visible_order.remove(from);
    visible_order.insert(to, item);

    let mut reordered_visible = visible_order.into_iter();
    for provider in order.iter_mut() {
        if visible_providers
            .iter()
            .any(|visible| provider.eq_ignore_ascii_case(visible))
            && let Some(next_visible) = reordered_visible.next()
        {
            *provider = next_visible;
        }
    }

    true
}

fn provider_drag_scroll_step(overflow: f32) -> f32 {
    (overflow * 0.55 + overflow.powf(1.25) * 0.18).clamp(3.0, 90.0)
}

fn provider_scrollbar_left(ui: &egui::Ui) -> f32 {
    let scroll_style = &ui.style().spacing.scroll;
    let full_width = scroll_style.bar_width.max(scroll_style.floating_width);
    ui.clip_rect().max.x - scroll_style.bar_outer_margin - full_width
}

fn paint_provider_drop_preview(ui: &egui::Ui, rect: egui::Rect, insert_after: bool) {
    let y = if insert_after {
        rect.bottom()
    } else {
        rect.top()
    };
    let stroke = ui.visuals().selection.stroke;
    ui.painter()
        .hline(rect.x_range(), y, egui::Stroke::new(2.0, stroke.color));
}

#[allow(clippy::too_many_arguments)]
fn render_provider(
    ui: &mut egui::Ui,
    provider_name: &str,
    provider_display_name: &str,
    data: Option<&UsageData>,
    card_width: f32,
    active_provider: &Arc<RwLock<String>>,
    config: &mut crate::config::AppConfig,
    config_path: Option<&PathBuf>,
    all_data: &Arc<RwLock<Vec<UsageData>>>,
    history: &Arc<RwLock<crate::usage_history::UsageHistory>>,
    collapse: bool,
    is_dragged: bool,
) -> egui::Response {
    let status = match data {
        Some(d) if d.error.is_some() => ProviderStatus::Error,
        Some(_) => ProviderStatus::Active,
        None => ProviderStatus::Disabled,
    };

    let credits = data.and_then(|d| d.credits.as_ref());
    let error_msg = data.and_then(|d| d.error.as_deref());
    let empty_vec = Vec::new();
    let windows = data.map(|d| &d.windows).unwrap_or(&empty_vec);

    let is_dark = ui.visuals().dark_mode;

    let card_frame = egui::Frame::NONE
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8));

    let left_indent = ((ui.available_width() - card_width) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(left_indent);
        let inner = ui.allocate_ui_with_layout(
            egui::vec2(card_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(card_width);
                ui.set_max_width(card_width);
                if is_dragged {
                    ui.set_opacity(0.45);
                }
                render_provider_card(
                    ui,
                    provider_name,
                    provider_display_name,
                    status,
                    credits,
                    error_msg,
                    windows,
                    is_dark,
                    card_frame,
                    card_width,
                    active_provider,
                    config,
                    config_path,
                    all_data,
                    history,
                    collapse,
                )
            },
        );
        inner.inner
    })
    .inner
}

#[allow(clippy::too_many_arguments)]
fn render_provider_card(
    ui: &mut egui::Ui,
    provider_name: &str,
    provider_display_name: &str,
    status: ProviderStatus,
    credits: Option<&crate::provider::CreditsInfo>,
    error_msg: Option<&str>,
    windows: &[crate::provider::UsageWindow],
    is_dark: bool,
    card_frame: egui::Frame,
    card_width: f32,
    active_provider: &Arc<RwLock<String>>,
    config: &mut crate::config::AppConfig,
    config_path: Option<&PathBuf>,
    all_data: &Arc<RwLock<Vec<UsageData>>>,
    history: &Arc<RwLock<crate::usage_history::UsageHistory>>,
    collapse: bool,
) -> egui::Response {
    let response = card_frame.show(ui, |ui| {
        // Enforce uniform width across all cards based on parent width minus horizontal margins (cast i8 margins to f32)
        let margin_x = (card_frame.inner_margin.left
            + card_frame.inner_margin.right
            + card_frame.outer_margin.left
            + card_frame.outer_margin.right) as f32;
        let content_width = (card_width - margin_x).max(0.0);
        let is_primary = active_provider.read().eq_ignore_ascii_case(provider_name);
        ui.set_min_width(content_width);
        ui.set_max_width(content_width);
        // Header Row - allocate the full content width to ensure all cards are identical in size
        ui.allocate_ui_with_layout(
            egui::vec2(content_width, 24.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if render_provider_icon(
                    ui,
                    provider_name,
                    is_dark,
                    active_provider,
                    config,
                    config_path,
                    all_data,
                ) {
                    ui.add_space(6.0);
                }

                // Title
                ui.label(
                    egui::RichText::new(provider_display_name)
                        .strong()
                        .size(13.5),
                );
                ui.add_space(6.0);

                if is_primary {
                    let (primary_bg, primary_border, primary_fg) = if is_dark {
                        (
                            egui::Color32::from_rgb(34, 43, 66),
                            egui::Color32::from_rgb(118, 185, 237),
                            egui::Color32::from_rgb(118, 185, 237),
                        )
                    } else {
                        (
                            egui::Color32::from_rgb(229, 242, 255),
                            egui::Color32::from_rgb(0, 120, 212),
                            egui::Color32::from_rgb(0, 91, 161),
                        )
                    };
                    let primary_frame = egui::Frame::NONE
                        .fill(primary_bg)
                        .stroke(egui::Stroke::new(1.0, primary_border))
                        .corner_radius(4)
                        .inner_margin(egui::Margin::symmetric(5, 2));
                    primary_frame.show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("PRIMARY")
                                .strong()
                                .size(8.0)
                                .color(primary_fg),
                        );
                    });
                    ui.add_space(4.0);
                }

                // Fluent-styled Badging (bordered plates with soft tints)
                let (status_text, bg_color, border_color, fg_color) = if is_dark {
                    match status {
                        ProviderStatus::Active => (
                            "ACTIVE",
                            egui::Color32::from_rgb(29, 45, 36),
                            egui::Color32::from_rgb(108, 203, 95),
                            egui::Color32::from_rgb(108, 203, 95),
                        ),
                        ProviderStatus::Error => (
                            "ERROR",
                            egui::Color32::from_rgb(62, 30, 30),
                            egui::Color32::from_rgb(255, 108, 108),
                            egui::Color32::from_rgb(255, 108, 108),
                        ),
                        ProviderStatus::Disabled => (
                            "OFFLINE",
                            egui::Color32::from_rgb(40, 40, 40),
                            egui::Color32::from_rgb(80, 80, 80),
                            egui::Color32::from_rgb(161, 161, 161),
                        ),
                    }
                } else {
                    match status {
                        ProviderStatus::Active => (
                            "ACTIVE",
                            egui::Color32::from_rgb(225, 244, 229),
                            egui::Color32::from_rgb(16, 124, 65),
                            egui::Color32::from_rgb(16, 124, 65),
                        ),
                        ProviderStatus::Error => (
                            "ERROR",
                            egui::Color32::from_rgb(253, 232, 232),
                            egui::Color32::from_rgb(196, 43, 28),
                            egui::Color32::from_rgb(196, 43, 28),
                        ),
                        ProviderStatus::Disabled => (
                            "OFFLINE",
                            egui::Color32::from_rgb(243, 243, 243),
                            egui::Color32::from_rgb(204, 204, 204),
                            egui::Color32::from_rgb(118, 118, 118),
                        ),
                    }
                };

                let badge_frame = egui::Frame::NONE
                    .fill(bg_color)
                    .stroke(egui::Stroke::new(1.0, border_color))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(5, 2));
                badge_frame.show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(status_text)
                            .strong()
                            .size(8.0)
                            .color(fg_color),
                    );
                });

                // Credits Badge (Windows 11 accent tint badge)
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let codex_reset_credits = if provider_name == "codex" {
                        windows.iter().find(|w| w.label == "Reset Credits").and_then(|w| {
                            w.unit.as_deref().and_then(|json_str| {
                                serde_json::from_str::<crate::provider::CodexResetCredits>(json_str).ok()
                            })
                        })
                    } else {
                        None
                    };

                    if let Some(resets) = codex_reset_credits {
                        let credit_text = format!("{} Resets", resets.available_count);

                        let (credits_bg, credits_border, credits_fg) = if is_dark {
                            (
                                egui::Color32::from_rgb(28, 46, 60),
                                egui::Color32::from_rgb(96, 205, 255), // Fluent Accent Blue
                                egui::Color32::from_rgb(96, 205, 255),
                            )
                        } else {
                            (
                                egui::Color32::from_rgb(224, 244, 255),
                                egui::Color32::from_rgb(0, 120, 212), // Fluent Accent Blue (Light)
                                egui::Color32::from_rgb(0, 120, 212),
                            )
                        };

                        let credits_frame = egui::Frame::NONE
                            .fill(credits_bg)
                            .stroke(egui::Stroke::new(1.0, credits_border))
                            .corner_radius(4)
                            .inner_margin(egui::Margin::symmetric(8, 3));
                        let badge_resp = credits_frame.show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(credit_text)
                                    .strong()
                                    .size(10.0)
                                    .color(credits_fg),
                            );
                        }).response;

                        badge_resp.on_hover_ui(|ui| {
                            ui.set_max_width(260.0);
                            ui.heading("Codex Reset Credits");
                            ui.add_space(4.0);
                            ui.label(format!("Available resets: {}", resets.available_count));
                            if resets.credits.is_empty() {
                                ui.label("No active reset credits.");
                            } else {
                                for (i, credit) in resets.credits.iter().enumerate() {
                                    ui.separator();
                                    ui.strong(format!("Credit #{}", i + 1));
                                    ui.label(format!("Status: {}", credit.status));
                                    if let Some(granted) = credit.granted_at {
                                        ui.label(format!(
                                            "Granted: {}",
                                            granted.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S")
                                        ));
                                    }
                                    if let Some(expires) = credit.expires_at {
                                        ui.label(format!(
                                            "Expires: {}",
                                            expires.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S")
                                        ));
                                    }
                                }
                            }
                        });
                    } else if let Some(c) = credits {
                        let credit_text =
                            format!("{} {}", format_credits_balance(c.balance), c.currency);

                        let (credits_bg, credits_border, credits_fg) = if is_dark {
                            (
                                egui::Color32::from_rgb(28, 46, 60),
                                egui::Color32::from_rgb(96, 205, 255), // Fluent Accent Blue
                                egui::Color32::from_rgb(96, 205, 255),
                            )
                        } else {
                            (
                                egui::Color32::from_rgb(224, 244, 255),
                                egui::Color32::from_rgb(0, 120, 212), // Fluent Accent Blue (Light)
                                egui::Color32::from_rgb(0, 120, 212),
                            )
                        };

                        let credits_frame = egui::Frame::NONE
                            .fill(credits_bg)
                            .stroke(egui::Stroke::new(1.0, credits_border))
                            .corner_radius(4)
                            .inner_margin(egui::Margin::symmetric(8, 3));
                        credits_frame.show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(credit_text)
                                    .strong()
                                    .size(10.0)
                                    .color(credits_fg),
                            );
                        });
                    }
                });
            },
        );

        if !collapse {
            let trend = history.read().trend_for(provider_name, 7);
            match status {
                ProviderStatus::Disabled => {}
                ProviderStatus::Error => {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // Fluent Callout Infolink / Infobar styling
                    let (err_bg, err_border, err_fg, warning_symbol_color) = if is_dark {
                        (
                            egui::Color32::from_rgb(61, 38, 38),
                            egui::Color32::from_rgb(196, 43, 28),
                            egui::Color32::from_rgb(255, 153, 153),
                            egui::Color32::from_rgb(255, 108, 108),
                        )
                    } else {
                        (
                            egui::Color32::from_rgb(253, 232, 232),
                            egui::Color32::from_rgb(196, 43, 28),
                            egui::Color32::from_rgb(153, 27, 27),
                            egui::Color32::from_rgb(196, 43, 28),
                        )
                    };

                    let error_frame = egui::Frame::NONE
                        .fill(err_bg)
                        .stroke(egui::Stroke::new(1.0, err_border))
                        .corner_radius(6)
                        .inner_margin(8);
                    error_frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(warning_symbol_color, "⚠");
                            ui.add_space(4.0);
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center)
                                    .with_main_wrap(true),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(
                                            error_msg.unwrap_or("Unknown error occurred"),
                                        )
                                        .small()
                                        .color(err_fg),
                                    );
                                },
                            );
                        });
                    });
                }
                ProviderStatus::Active => {
                    let active_windows: Vec<_> = windows.iter().filter(|w| w.label != "Reset Credits").collect();
                    if active_windows.is_empty() {
                        if credits.is_none() {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("No active usage windows.")
                                    .small()
                                    .weak(),
                            );
                        }
                    } else {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        let available_width = ui.available_width();
                        let gap = 8.0;
                        let label_width = 88.0_f32;
                        let reset_width = 82.0_f32;
                        let progress_width =
                            (available_width - label_width - reset_width - gap * 2.0).max(96.0);

                        for window in active_windows {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;

                                ui.allocate_ui_with_layout(
                                    egui::vec2(label_width, 18.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.add_sized(
                                            [label_width, 18.0],
                                            egui::Label::new(
                                                egui::RichText::new(&window.label)
                                                    .strong()
                                                    .size(11.0),
                                            )
                                            .truncate(),
                                        );
                                    },
                                );

                                ui.allocate_ui_with_layout(
                                    egui::vec2(progress_width, 18.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        render_usage_progress(
                                            ui,
                                            window.used_percent as f32,
                                            is_dark,
                                        );
                                    },
                                );

                                ui.allocate_ui_with_layout(
                                    egui::vec2(reset_width, 18.0),
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let reset_text = reset_time_text(window.resets_at);
                                        ui.add_sized(
                                            [reset_width, 18.0],
                                            egui::Label::new(
                                                egui::RichText::new(reset_text).small().weak(),
                                            )
                                            .truncate(),
                                        );
                                    },
                                );
                            });
                            ui.add_space(4.0);
                        }

                        if let Some(trend) = trend {
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new(format_trend_summary(&trend))
                                    .small()
                                    .weak(),
                            );
                        }
                    }
                }
            }
        }
    });

    if response.response.rect.width() < card_width {
        ui.allocate_space(egui::vec2(card_width, 0.0));
    }

    response.response
}

fn format_trend_summary(trend: &crate::usage_history::ProviderTrend) -> String {
    let delta = trend
        .previous_percent
        .map(|previous| trend.latest_percent - previous)
        .filter(|value| value.abs() >= 0.05)
        .map(|value| {
            if value >= 0.0 {
                format!("+{value:.1} pp")
            } else {
                format!("{value:.1} pp")
            }
        })
        .unwrap_or_else(|| "flat".to_string());

    format!(
        "7d trend: avg {:.0}% · peak {:.0}% · {delta} · {} samples",
        trend.average_percent, trend.peak_percent, trend.samples
    )
}

fn render_secret_status(ui: &mut egui::Ui, provider: &str, field: &str, env_names: &[&str]) {
    ui.horizontal(|ui| {
        let status = if crate::secrets::configured(provider, field, env_names) {
            "Stored in Windows Credential Manager"
        } else {
            "Not stored"
        };
        ui.label(egui::RichText::new(status).small().weak());
        if ui.link("Clear").clicked()
            && let Err(err) = crate::secrets::delete(provider, field)
        {
            tracing::error!("Failed to clear credential {provider}/{field}: {err}");
        }
    });
}

fn provider_icon(provider_name: &str, is_dark: bool) -> Option<egui::ImageSource<'static>> {
    match (provider_name, is_dark) {
        ("abacus", true) => Some(egui::include_image!(
            "../assets/provider-icons/abacus-ai-dark.svg"
        )),
        ("abacus", false) => Some(egui::include_image!(
            "../assets/provider-icons/abacus-ai.png"
        )),
        ("alibabatoken", _) => Some(egui::include_image!("../assets/provider-icons/alibaba.svg")),
        ("amp", _) => Some(egui::include_image!("../assets/provider-icons/amp.svg")),
        ("augment", _) => Some(egui::include_image!("../assets/provider-icons/augment.svg")),
        ("codex", true) => Some(egui::include_image!(
            "../assets/provider-icons/codex-dark.svg"
        )),
        ("codex", false) => Some(egui::include_image!("../assets/provider-icons/codex.svg")),
        ("codebuff", true) => Some(egui::include_image!(
            "../assets/provider-icons/codebuff-dark.svg"
        )),
        ("codebuff", false) => Some(egui::include_image!(
            "../assets/provider-icons/codebuff.svg"
        )),
        ("copilot", _) => Some(egui::include_image!("../assets/provider-icons/copilot.svg")),
        ("cursor", _) => Some(egui::include_image!("../assets/provider-icons/cursor.svg")),
        ("droid", true) => Some(egui::include_image!(
            "../assets/provider-icons/droid-dark.svg"
        )),
        ("droid", false) => Some(egui::include_image!("../assets/provider-icons/droid.svg")),
        ("elevenlabs", _) => Some(egui::include_image!(
            "../assets/provider-icons/elevenlabs.svg"
        )),
        ("jetbrains", _) => Some(egui::include_image!(
            "../assets/provider-icons/jetbrains-ai.svg"
        )),
        ("kilo", _) => Some(egui::include_image!("../assets/provider-icons/kilo.svg")),
        ("kimi", _) => Some(egui::include_image!("../assets/provider-icons/kimi.svg")),
        ("kiro", true) => Some(egui::include_image!(
            "../assets/provider-icons/kiro-dark.svg"
        )),
        ("kiro", false) => Some(egui::include_image!("../assets/provider-icons/kiro.svg")),
        ("minimax", _) => Some(egui::include_image!("../assets/provider-icons/minimax.svg")),
        ("mistral", _) => Some(egui::include_image!("../assets/provider-icons/mistral.svg")),
        ("ollama", _) => Some(egui::include_image!("../assets/provider-icons/ollama.svg")),
        ("opencode" | "opencodego", true) => Some(egui::include_image!(
            "../assets/provider-icons/opencode-dark.svg"
        )),
        ("opencode" | "opencodego", false) => Some(egui::include_image!(
            "../assets/provider-icons/opencode.svg"
        )),
        ("openrouter", _) => Some(egui::include_image!(
            "../assets/provider-icons/openrouter.svg"
        )),
        ("claude", _) => Some(egui::include_image!("../assets/provider-icons/claude.svg")),
        ("gemini", _) => Some(egui::include_image!("../assets/provider-icons/gemini.svg")),
        ("antigravity", _) => Some(egui::include_image!(
            "../assets/provider-icons/antigravity.svg"
        )),
        ("deepseek", _) => Some(egui::include_image!(
            "../assets/provider-icons/deepseek.svg"
        )),
        ("synthetic", true) => Some(egui::include_image!(
            "../assets/provider-icons/synthetic-dark.svg"
        )),
        ("synthetic", false) => Some(egui::include_image!(
            "../assets/provider-icons/synthetic.svg"
        )),
        ("vertexai", _) => Some(egui::include_image!(
            "../assets/provider-icons/vertex-ai.svg"
        )),
        ("warp", _) => Some(egui::include_image!("../assets/provider-icons/warp.svg")),
        ("zai", true) => Some(egui::include_image!(
            "../assets/provider-icons/zai-dark.svg"
        )),
        ("zai", false) => Some(egui::include_image!("../assets/provider-icons/zai.svg")),
        _ => None,
    }
}

fn render_provider_icon(
    ui: &mut egui::Ui,
    provider_name: &str,
    is_dark: bool,
    active_provider: &Arc<RwLock<String>>,
    config: &mut crate::config::AppConfig,
    config_path: Option<&PathBuf>,
    data: &Arc<RwLock<Vec<UsageData>>>,
) -> bool {
    let is_active = active_provider.read().eq_ignore_ascii_case(provider_name);
    let tooltip = if is_active {
        "Primary provider"
    } else {
        "Double-click to set as primary provider"
    };

    if let Some(icon) = provider_icon(provider_name, is_dark) {
        let response = ui
            .add(
                egui::Image::new(icon)
                    .fit_to_exact_size(egui::vec2(18.0, 18.0))
                    .maintain_aspect_ratio(true)
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(tooltip);
        if response.double_clicked() {
            set_active_provider(provider_name, active_provider, config, config_path, data);
        }
        return true;
    }

    if provider_name == "mimo" {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
        let (bg, fg) = if is_dark {
            (
                egui::Color32::from_rgb(54, 69, 89),
                egui::Color32::from_rgb(210, 225, 255),
            )
        } else {
            (
                egui::Color32::from_rgb(232, 240, 255),
                egui::Color32::from_rgb(37, 70, 130),
            )
        };
        ui.painter().circle_filled(rect.center(), 9.0, bg);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "M",
            egui::FontId::proportional(11.0),
            fg,
        );
        if response.on_hover_text(tooltip).double_clicked() {
            set_active_provider(provider_name, active_provider, config, config_path, data);
        }
        return true;
    }

    false
}

fn set_active_provider(
    provider_name: &str,
    active_provider: &Arc<RwLock<String>>,
    config: &mut crate::config::AppConfig,
    config_path: Option<&PathBuf>,
    data: &Arc<RwLock<Vec<UsageData>>>,
) {
    *active_provider.write() = provider_name.to_string();

    config.general.active_provider = provider_name.to_string();
    if let Err(err) = save_config_without_secrets(config, config_path) {
        tracing::error!("Failed to save active provider {provider_name}: {err}");
    }

    update_tray_icon_for_active_provider(provider_name, data);
    crate::tray::request_refresh();
}

fn save_config_without_secrets(
    config: &crate::config::AppConfig,
    config_path: Option<&PathBuf>,
) -> anyhow::Result<()> {
    let mut config_to_save = config.clone();
    crate::secrets::store_and_scrub_config(&mut config_to_save);
    if let Some(path) = config_path {
        config_to_save.save_to(path)
    } else {
        config_to_save.save()
    }
}

fn enable_provider_for_test(config: &mut crate::config::AppConfig, provider: &str) {
    match provider {
        "deepseek" => config.deepseek.enabled = Some(true),
        "claude" => config.claude.enabled = Some(true),
        "codex" => config.codex.enabled = Some(true),
        "gemini" => config.gemini.enabled = Some(true),
        "antigravity" => config.antigravity.enabled = Some(true),
        "opencode" | "opencodego" => config.opencode.enabled = Some(true),
        "mimo" => config.mimo.enabled = Some(true),
        _ => {
            if let Some(cfg) = api_key_provider_config_mut(config, provider) {
                cfg.enabled = Some(true);
            }
        }
    }
}

fn api_key_provider_config_mut<'a>(
    config: &'a mut crate::config::AppConfig,
    provider: &str,
) -> Option<&'a mut crate::config::ApiKeyProviderConfig> {
    match provider {
        "openai" => Some(&mut config.openai),
        "openrouter" => Some(&mut config.openrouter),
        "moonshot" => Some(&mut config.moonshot),
        "elevenlabs" => Some(&mut config.elevenlabs),
        "doubao" => Some(&mut config.doubao),
        "zai" => Some(&mut config.zai),
        "venice" => Some(&mut config.venice),
        "crof" => Some(&mut config.crof),
        "synthetic" => Some(&mut config.synthetic),
        "warp" => Some(&mut config.warp),
        "groqcloud" => Some(&mut config.groqcloud),
        "deepgram" => Some(&mut config.deepgram),
        "llmproxy" => Some(&mut config.llmproxy),
        "codebuff" => Some(&mut config.codebuff),
        "kiro" => Some(&mut config.kiro),
        "copilot" => Some(&mut config.copilot),
        "azureopenai" => Some(&mut config.azureopenai),
        "ollama" => Some(&mut config.ollama),
        "minimax" => Some(&mut config.minimax),
        "jetbrains" => Some(&mut config.jetbrains),
        "kimi" => Some(&mut config.kimi),
        "kilo" => Some(&mut config.kilo),
        "augment" => Some(&mut config.augment),
        "bedrock" => Some(&mut config.bedrock),
        "vertexai" => Some(&mut config.vertexai),
        "stepfun" => Some(&mut config.stepfun),
        "abacus" => Some(&mut config.abacus),
        "alibabatoken" => Some(&mut config.alibabatoken),
        "t3chat" => Some(&mut config.t3chat),
        "amp" => Some(&mut config.amp),
        "mistral" => Some(&mut config.mistral),
        "grok" => Some(&mut config.grok),
        "cursor" => Some(&mut config.cursor),
        "droid" => Some(&mut config.droid),
        "windsurf" => Some(&mut config.windsurf),
        _ => None,
    }
}

fn summarize_provider_test(data: &UsageData) -> String {
    let max_percent = data.max_used_percent();
    let windows = data.windows.len();
    let credits = data
        .credits
        .as_ref()
        .map(|credits| {
            format!(
                " Credits: {} {}.",
                format_credits_balance(credits.balance),
                credits.currency
            )
        })
        .unwrap_or_default();

    if windows == 0 && data.credits.is_none() {
        "Provider responded, but no usage windows or credits were returned.".to_string()
    } else {
        format!("Returned {windows} usage window(s), max usage {max_percent:.0}%.{credits}")
    }
}

fn update_tray_icon_for_active_provider(provider_name: &str, data: &Arc<RwLock<Vec<UsageData>>>) {
    let data = data.read();
    let active_provider = Some(provider_name);
    let icon = crate::icon::generate_icon(&data, active_provider);
    let tooltip = crate::icon::tray_tooltip(&data, active_provider);
    if let Ok(hicon) = icon.to_hicon()
        && let Some(&tray_hwnd) = crate::tray::TRAY_HWND.get()
    {
        let controller = crate::tray::TrayController::from_hwnd(tray_hwnd.raw());
        controller.update_icon_with_tooltip(hicon, &tooltip);
    }
}

fn render_usage_progress(ui: &mut egui::Ui, pct: f32, is_dark: bool) {
    let pct = pct.clamp(0.0, 100.0);
    let pct_width = 34.0;
    let bar_width = (ui.available_width() - pct_width - 6.0).max(48.0);
    let bar_height = 8.0;
    let rounding = 4.0;

    let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_width, bar_height), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let track_color = if is_dark {
            egui::Color32::from_rgb(32, 32, 32)
        } else {
            egui::Color32::from_rgb(229, 229, 229)
        };
        ui.painter().rect_filled(rect, rounding, track_color);

        let fill_width = bar_width * (pct / 100.0);
        if fill_width > 0.0 {
            let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(fill_width, bar_height));
            ui.painter()
                .rect_filled(fill_rect, rounding, progress_color(pct, is_dark));
        }
    }

    ui.add_space(4.0);
    ui.add_sized(
        [pct_width, 18.0],
        egui::Label::new(
            egui::RichText::new(format!("{pct:.0}%"))
                .color(progress_color(pct, is_dark))
                .strong()
                .size(10.0),
        ),
    );
}

fn progress_color(pct: f32, is_dark: bool) -> egui::Color32 {
    if is_dark {
        if pct >= 80.0 {
            egui::Color32::from_rgb(241, 112, 122)
        } else if pct >= 50.0 {
            egui::Color32::from_rgb(255, 200, 0)
        } else {
            egui::Color32::from_rgb(96, 205, 255)
        }
    } else if pct >= 80.0 {
        egui::Color32::from_rgb(196, 43, 28)
    } else if pct >= 50.0 {
        egui::Color32::from_rgb(179, 123, 0)
    } else {
        egui::Color32::from_rgb(0, 120, 212)
    }
}

fn reset_time_text(resets_at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    let Some(resets) = resets_at else {
        return "-".to_string();
    };

    let remaining = resets - chrono::Utc::now();
    if remaining.num_seconds() <= 0 {
        return "resetting".to_string();
    }

    let days = remaining.num_days();
    let hours = remaining.num_hours() % 24;
    let minutes = remaining.num_minutes() % 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

impl QuotifyApp {
    fn render_about_page(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        card_width: f32,
        card_left_indent: f32,
    ) {
        let about_frame = egui::Frame::NONE
            .fill(ui.visuals().widgets.noninteractive.bg_fill)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(16, 16));

        ui.horizontal(|ui| {
            ui.add_space(card_left_indent);
            ui.allocate_ui(egui::vec2(card_width, 0.0), |ui| {
                about_frame.show(ui, |ui| {
                    ui.set_min_width(card_width - 32.0);
                    ui.vertical(|ui| {
                        let logo =
                            egui::Image::new(egui::include_image!("../assets/icons/quotify.svg"))
                                .fit_to_exact_size(egui::vec2(48.0, 48.0))
                                .maintain_aspect_ratio(true);
                        ui.add(logo);

                        ui.add_space(12.0);

                        ui.label(egui::RichText::new("Quotify").heading().size(24.0).strong());

                        ui.add_space(8.0);

                        let version_str = env!("GIT_TAG");
                        ui.label(
                            egui::RichText::new(format!("Version: {version_str}"))
                                .size(14.0)
                                .color(ui.visuals().widgets.noninteractive.text_color()),
                        );

                        ui.add_space(4.0);

                        ui.label(
                            egui::RichText::new("Author: zuoxinyu")
                                .size(14.0)
                                .color(ui.visuals().widgets.noninteractive.text_color()),
                        );

                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("GitHub: ")
                                    .size(14.0)
                                    .color(ui.visuals().widgets.noninteractive.text_color()),
                            );
                            ui.hyperlink_to(
                                egui::RichText::new("zuoxinyu/quotify")
                                    .size(14.0)
                                    .underline(),
                                "https://github.com/zuoxinyu/quotify",
                            );
                        });

                        ui.add_space(16.0);
                        ui.separator();
                        ui.add_space(16.0);

                        ui.label(egui::RichText::new("Check for Updates").strong().size(16.0));

                        ui.add_space(8.0);

                        let status = self.update_status.lock().clone();
                        match status {
                            UpdateStatus::Idle => {
                                let check_btn = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("Check for Updates")
                                            .size(10.0)
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(0, 95, 184)) // WinUI Accent Blue
                                    .corner_radius(4)
                                    .min_size(egui::vec2(96.0, 24.0)),
                                );
                                if check_btn.clicked() {
                                    self.trigger_check_update(ctx.clone());
                                }
                            }
                            UpdateStatus::Checking => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new("Checking latest release...")
                                            .size(14.0)
                                            .color(ui.visuals().weak_text_color()),
                                    );
                                });
                            }
                            UpdateStatus::UpToDate { latest_version } => {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "You are up to date! (Latest: {latest_version})"
                                    ))
                                    .size(14.0)
                                    .color(egui::Color32::from_rgb(46, 125, 50)),
                                );
                                ui.add_space(6.0);

                                let check_again = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("Check Again").size(10.0),
                                    )
                                    .corner_radius(4)
                                    .min_size(egui::vec2(64.0, 24.0)),
                                );
                                if check_again.clicked() {
                                    self.trigger_check_update(ctx.clone());
                                }
                            }
                            UpdateStatus::NewVersionAvailable {
                                latest_version,
                                release_url,
                            } => {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "A new version is available: {latest_version}"
                                    ))
                                    .size(14.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(22, 101, 216)),
                                );
                                ui.label(
                                    egui::RichText::new(
                                        "Please download manually from GitHub releases.",
                                    )
                                    .size(12.0)
                                    .color(ui.visuals().widgets.noninteractive.text_color()),
                                );
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    let dl_btn = ui.add(
                                        egui::Button::new(
                                            egui::RichText::new("Download Now")
                                                .size(13.0)
                                                .strong()
                                                .color(egui::Color32::WHITE),
                                        )
                                        .fill(egui::Color32::from_rgb(0, 95, 184))
                                        .corner_radius(4)
                                        .min_size(egui::vec2(100.0, 26.0)),
                                    );
                                    if dl_btn.clicked() {
                                        open_browser(&release_url);
                                    }

                                    let check_again = ui.add(
                                        egui::Button::new(
                                            egui::RichText::new("Check Again").size(12.0),
                                        )
                                        .corner_radius(4)
                                        .min_size(egui::vec2(80.0, 26.0)),
                                    );
                                    if check_again.clicked() {
                                        self.trigger_check_update(ctx.clone());
                                    }
                                });
                            }
                            UpdateStatus::Error(err) => {
                                ui.label(
                                    egui::RichText::new(format!("Error: {err}"))
                                        .size(14.0)
                                        .color(egui::Color32::from_rgb(198, 40, 40)),
                                );
                                ui.add_space(6.0);

                                let retry_btn = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("Retry")
                                            .size(13.0)
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(0, 95, 184))
                                    .corner_radius(4)
                                    .min_size(egui::vec2(80.0, 26.0)),
                                );
                                if retry_btn.clicked() {
                                    self.trigger_check_update(ctx.clone());
                                }
                            }
                        }
                    });
                });
            });
        });
    }

    fn trigger_check_update(&self, ctx: egui::Context) {
        *self.update_status.lock() = UpdateStatus::Checking;

        let status = self.update_status.clone();
        let proxy = self.config.network.proxy.clone();
        let current_version = env!("GIT_TAG").to_string();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    *status.lock() = UpdateStatus::Error(format!("Runtime error: {}", err));
                    ctx.request_repaint();
                    return;
                }
            };

            rt.block_on(async {
                let client = crate::provider::http_client(Some(&proxy));

                let res = client
                    .get("https://api.github.com/repos/zuoxinyu/quotify/releases/latest")
                    .header("User-Agent", "Quotify-Update-Checker")
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await;

                match res {
                    Ok(response) => {
                        if response.status() == reqwest::StatusCode::NOT_FOUND {
                            *status.lock() = UpdateStatus::UpToDate {
                                latest_version: "None (No releases found)".to_string(),
                            };
                            ctx.request_repaint();
                            return;
                        }

                        if !response.status().is_success() {
                            let err_msg = format!("GitHub API returned HTTP {}", response.status());
                            *status.lock() = UpdateStatus::Error(err_msg);
                            ctx.request_repaint();
                            return;
                        }

                        #[derive(serde::Deserialize)]
                        struct GithubRelease {
                            tag_name: String,
                            html_url: String,
                        }

                        match response.json::<GithubRelease>().await {
                            Ok(release) => {
                                if is_newer(&current_version, &release.tag_name) {
                                    *status.lock() = UpdateStatus::NewVersionAvailable {
                                        latest_version: release.tag_name,
                                        release_url: release.html_url,
                                    };
                                } else {
                                    *status.lock() = UpdateStatus::UpToDate {
                                        latest_version: release.tag_name,
                                    };
                                }
                            }
                            Err(err) => {
                                let err_msg = format!("Failed to parse release: {err}");
                                *status.lock() = UpdateStatus::Error(err_msg);
                            }
                        }
                    }
                    Err(err) => {
                        let err_msg = format!("Network error: {err}");
                        *status.lock() = UpdateStatus::Error(err_msg);
                    }
                }
                ctx.request_repaint();
            });
        });
    }

    fn render_settings_page(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
        card_width: f32,
        card_left_indent: f32,
    ) {
        let card_frame = egui::Frame::NONE
            .fill(ui.visuals().widgets.noninteractive.bg_fill)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(16, 12));

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(card_left_indent);
                    ui.allocate_ui(egui::vec2(card_width, 0.0), |ui| {
                        ui.vertical(|ui| {
                            // Section 1: General Settings
                            ui.label(egui::RichText::new("General Settings").strong().size(14.0));
                            ui.add_space(4.0);

                            card_frame.show(ui, |ui| {
                                ui.set_min_width(card_width - 32.0);
                                ui.vertical(|ui| {
                                    // Theme Settings
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(egui::RichText::new("Theme").strong().size(13.0));
                                            ui.label(egui::RichText::new("Configure app color palette").small().color(ui.visuals().weak_text_color()));
                                        });
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            let mut current_theme = self.config.general.theme.clone();
                                            egui::ComboBox::from_id_salt("theme_combobox")
                                                .selected_text(match current_theme.as_str() {
                                                    "dark" => "Dark",
                                                    "light" => "Light",
                                                    _ => "System",
                                                })
                                                .show_ui(ui, |ui| {
                                                    let mut changed = false;
                                                    changed |= ui.selectable_value(&mut current_theme, "system".to_string(), "System").changed();
                                                    changed |= ui.selectable_value(&mut current_theme, "dark".to_string(), "Dark").changed();
                                                    changed |= ui.selectable_value(&mut current_theme, "light".to_string(), "Light").changed();
                                                    if changed {
                                                        self.config.general.theme = current_theme;
                                                        self.save_config();
                                                    }
                                                });
                                        });
                                    });

                                    ui.add_space(10.0);
                                    ui.separator();
                                    ui.add_space(10.0);

                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new("Start with Windows")
                                                    .strong()
                                                    .size(13.0),
                                            );
                                            ui.label(
                                                egui::RichText::new("Launch Quotify when you sign in")
                                                    .small()
                                                    .color(ui.visuals().weak_text_color()),
                                            );
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let mut enabled =
                                                    self.config.general.start_with_windows;
                                                if ui.checkbox(&mut enabled, "").changed() {
                                                    match crate::startup::set_enabled(enabled) {
                                                        Ok(()) => {
                                                            self.config.general.start_with_windows =
                                                                enabled;
                                                            self.save_config();
                                                        }
                                                        Err(err) => {
                                                            tracing::error!(
                                                                "Failed to update startup setting: {err}"
                                                            );
                                                        }
                                                    }
                                                }
                                            },
                                        );
                                    });

                                    ui.add_space(10.0);
                                    ui.separator();
                                    ui.add_space(10.0);

                                    // Refresh Interval Settings
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new("Refresh Interval").strong().size(13.0));
                                        ui.label(egui::RichText::new("Background update frequency").small().color(ui.visuals().weak_text_color()));
                                        ui.add_space(4.0);
                                        let mut interval = self.config.general.refresh_interval;
                                        let slider = ui.add_sized(
                                            egui::vec2(ui.available_width() - 12.0, 20.0),
                                            egui::Slider::new(&mut interval, 10..=3600)
                                                .suffix("s")
                                                .show_value(true)
                                                .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.4 })
                                        );
                                        if slider.changed() {
                                            self.config.general.refresh_interval = interval;
                                            self.save_config();
                                        }
                                    });

                                    ui.add_space(10.0);
                                    ui.separator();
                                    ui.add_space(10.0);

                                    // Network Proxy Settings
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new("Network Proxy").strong().size(13.0));
                                        ui.label(egui::RichText::new("Supports http://, https://, or socks5://").small().color(ui.visuals().weak_text_color()));
                                        ui.add_space(4.0);
                                        let mut proxy = self.config.network.proxy.clone();
                                        let text_edit = ui.add(
                                            egui::TextEdit::singleline(&mut proxy)
                                                .desired_width(f32::INFINITY)
                                                .hint_text("e.g. http://127.0.0.1:7890")
                                        );
                                        if text_edit.changed() {
                                            self.config.network.proxy = proxy;
                                            self.save_config();
                                        }
                                    });
                                });
                            });

                            ui.add_space(16.0);

                            // Section 2: Provider Configs
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Provider Settings").strong().size(14.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let btn_text = if self.show_secrets { "Hide Secrets" } else { "Show Secrets" };
                                    if ui.button(btn_text).clicked() {
                                        self.show_secrets = !self.show_secrets;
                                    }
                                });
                            });
                            ui.add_space(4.0);

                            card_frame.show(ui, |ui| {
                                ui.set_min_width(card_width - 32.0);
                                ui.vertical(|ui| {
                                    // Dropdown selection to pick which provider to configure
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Select Provider").strong().size(13.0));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            let current_provider = self.selected_setting_provider.clone();
                                            let catalog = provider_catalog();
                                            let display_name = catalog.iter()
                                                .find(|(id, _)| *id == current_provider)
                                                .map(|(_, name)| *name)
                                                .unwrap_or(&current_provider);

                                            egui::ComboBox::from_id_salt("provider_select_combo")
                                                .selected_text(display_name)
                                                .show_ui(ui, |ui| {
                                                    let mut selected = self.selected_setting_provider.clone();
                                                    for (id, display) in catalog {
                                                        if ui.selectable_value(&mut selected, id.to_string(), *display).clicked() {
                                                            self.selected_setting_provider = selected.clone();
                                                        }
                                                    }
                                                });
                                        });
                                    });

                                    ui.add_space(10.0);
                                    ui.separator();
                                    ui.add_space(10.0);

                                    // Render the fields for the selected provider
                                    let provider_id = self.selected_setting_provider.clone();
                                    let mut changed = false;

                                    match provider_id.as_str() {
                                        "deepseek" => {
                                            let mut enabled = self.config.deepseek.enabled.unwrap_or(false);
                                            if ui.checkbox(&mut enabled, "Enable Provider").changed() {
                                                self.config.deepseek.enabled = Some(enabled);
                                                changed = true;
                                            }
                                            ui.add_space(8.0);
                                            ui.label(egui::RichText::new("API Key").strong().size(12.0));
                                            let text_edit = ui.add(
                                                egui::TextEdit::singleline(&mut self.config.deepseek.api_key)
                                                    .password(!self.show_secrets)
                                                    .desired_width(f32::INFINITY)
                                            );
                                            if text_edit.changed() {
                                                changed = true;
                                            }
                                        }
                                        "claude" => {
                                            let mut enabled = self.config.claude.enabled.unwrap_or(false);
                                            if ui.checkbox(&mut enabled, "Enable Provider").changed() {
                                                self.config.claude.enabled = Some(enabled);
                                                changed = true;
                                            }
                                            ui.add_space(8.0);
                                            ui.label(egui::RichText::new("API Key").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.claude.api_key).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new("Session Key").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.claude.session_key).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new("Access Token").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.claude.access_token).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new("Auth File Path").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.claude.auth_file).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                        }
                                        "codex" => {
                                            let mut enabled = self.config.codex.enabled.unwrap_or(false);
                                            if ui.checkbox(&mut enabled, "Enable Provider").changed() {
                                                self.config.codex.enabled = Some(enabled);
                                                changed = true;
                                            }
                                            ui.add_space(8.0);
                                            ui.label(egui::RichText::new("Auth File Path").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.codex.auth_file).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                        }
                                        "gemini" => {
                                            let mut enabled = self.config.gemini.enabled.unwrap_or(false);
                                            if ui.checkbox(&mut enabled, "Enable Provider").changed() {
                                                self.config.gemini.enabled = Some(enabled);
                                                changed = true;
                                            }
                                            ui.add_space(8.0);
                                            ui.label(egui::RichText::new("API Key").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.gemini.api_key).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                        }
                                        "antigravity" => {
                                            let mut enabled = self.config.antigravity.enabled.unwrap_or(false);
                                            if ui.checkbox(&mut enabled, "Enable Provider").changed() {
                                                self.config.antigravity.enabled = Some(enabled);
                                                changed = true;
                                            }
                                            ui.add_space(8.0);
                                            ui.label(egui::RichText::new("API Key").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.antigravity.api_key).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                        }
                                        "opencode" => {
                                            let mut enabled = self.config.opencode.enabled.unwrap_or(false);
                                            if ui.checkbox(&mut enabled, "Enable Provider").changed() {
                                                self.config.opencode.enabled = Some(enabled);
                                                changed = true;
                                            }
                                            ui.add_space(8.0);
                                            ui.label(egui::RichText::new("API Key").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.opencode.api_key).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new("Workspace ID").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.opencode.workspace_id).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new("Auth Cookie").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.opencode.auth_cookie).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                        }
                                        "mimo" => {
                                            let mut enabled = self.config.mimo.enabled.unwrap_or(false);
                                            if ui.checkbox(&mut enabled, "Enable Provider").changed() {
                                                self.config.mimo.enabled = Some(enabled);
                                                changed = true;
                                            }
                                            ui.add_space(8.0);
                                            ui.label(egui::RichText::new("API Key").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.mimo.api_key).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new("Service Token").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.mimo.service_token).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new("Cookie Header").strong().size(12.0));
                                            if ui.add(egui::TextEdit::singleline(&mut self.config.mimo.cookie_header).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                changed = true;
                                            }
                                        }
                                        "opencodego" => {
                                            ui.label(egui::RichText::new("OpenCode Go is configured using the OpenCode settings. Please select 'OpenCode' from the dropdown to edit its configuration.").italics().color(ui.visuals().weak_text_color()));
                                        }
                                        // All standard ApiKeyProviderConfig providers
                                        _ => {
                                            let cfg = match provider_id.as_str() {
                                                "openai" => Some(&mut self.config.openai),
                                                "openrouter" => Some(&mut self.config.openrouter),
                                                "moonshot" => Some(&mut self.config.moonshot),
                                                "elevenlabs" => Some(&mut self.config.elevenlabs),
                                                "doubao" => Some(&mut self.config.doubao),
                                                "zai" => Some(&mut self.config.zai),
                                                "venice" => Some(&mut self.config.venice),
                                                "crof" => Some(&mut self.config.crof),
                                                "synthetic" => Some(&mut self.config.synthetic),
                                                "warp" => Some(&mut self.config.warp),
                                                "groqcloud" => Some(&mut self.config.groqcloud),
                                                "deepgram" => Some(&mut self.config.deepgram),
                                                "llmproxy" => Some(&mut self.config.llmproxy),
                                                "codebuff" => Some(&mut self.config.codebuff),
                                                "kiro" => Some(&mut self.config.kiro),
                                                "copilot" => Some(&mut self.config.copilot),
                                                "azureopenai" => Some(&mut self.config.azureopenai),
                                                "ollama" => Some(&mut self.config.ollama),
                                                "minimax" => Some(&mut self.config.minimax),
                                                "jetbrains" => Some(&mut self.config.jetbrains),
                                                "kimi" => Some(&mut self.config.kimi),
                                                "kilo" => Some(&mut self.config.kilo),
                                                "augment" => Some(&mut self.config.augment),
                                                "bedrock" => Some(&mut self.config.bedrock),
                                                "vertexai" => Some(&mut self.config.vertexai),
                                                "stepfun" => Some(&mut self.config.stepfun),
                                                "abacus" => Some(&mut self.config.abacus),
                                                "alibabatoken" => Some(&mut self.config.alibabatoken),
                                                "t3chat" => Some(&mut self.config.t3chat),
                                                "amp" => Some(&mut self.config.amp),
                                                "mistral" => Some(&mut self.config.mistral),
                                                "grok" => Some(&mut self.config.grok),
                                                "cursor" => Some(&mut self.config.cursor),
                                                "droid" => Some(&mut self.config.droid),
                                                "windsurf" => Some(&mut self.config.windsurf),
                                                _ => None,
                                            };

                                            if let Some(cfg) = cfg {
                                                let mut enabled = cfg.enabled.unwrap_or(false);
                                                if ui.checkbox(&mut enabled, "Enable Provider").changed() {
                                                    cfg.enabled = Some(enabled);
                                                    changed = true;
                                                }
                                                ui.add_space(8.0);
                                                ui.label(egui::RichText::new("API Key / Token").strong().size(12.0));
                                                render_secret_status(ui, provider_id.as_str(), "api_key", &[]);
                                                if ui.add(egui::TextEdit::singleline(&mut cfg.api_key).password(!self.show_secrets).desired_width(f32::INFINITY)).changed() {
                                                    changed = true;
                                                }
                                                ui.add_space(6.0);
                                                ui.label(egui::RichText::new("Base URL").strong().size(12.0));
                                                if ui.add(egui::TextEdit::singleline(&mut cfg.base_url).desired_width(f32::INFINITY)).changed() {
                                                    changed = true;
                                                }
                                                ui.add_space(6.0);
                                                ui.label(egui::RichText::new("Deployment").strong().size(12.0));
                                                if ui.add(egui::TextEdit::singleline(&mut cfg.deployment).desired_width(f32::INFINITY)).changed() {
                                                    changed = true;
                                                }
                                            } else {
                                                ui.label("Unknown provider settings.");
                                            }
                                        }
                                    }

                                    if changed {
                                        self.save_config();
                                    }

                                    ui.add_space(12.0);
                                    ui.separator();
                                    ui.add_space(10.0);
                                    self.render_provider_test_controls(ui, &provider_id);
                                });
                            });

                            ui.add_space(16.0);
                            // Link/button to open file in Notepad directly
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    if ui.link("Open config file in text editor").clicked() {
                                        let _ = open_config_file(self.config_path.as_ref());
                                    }
                                    ui.separator();
                                    if ui.link("Open logs").clicked() {
                                        let _ = open_folder(&crate::diagnostics::log_dir());
                                    }
                                    ui.separator();
                                    if ui.link("Create diagnostic report").clicked() {
                                        match crate::diagnostics::write_diagnostic_report(
                                            self.config_path.as_deref(),
                                            Some(&self.history.read()),
                                        ) {
                                            Ok(path) => {
                                                let _ = open_folder(path.parent().unwrap_or_else(|| std::path::Path::new(".")));
                                            }
                                            Err(err) => tracing::error!("Failed to write diagnostic report: {err}"),
                                        }
                                    }
                                });
                            });
                            ui.add_space(20.0);
                        });
                    });
                });
            });
    }
}

fn normalize_version(v: &str) -> String {
    let v = v.trim().trim_start_matches('v').trim_start_matches('V');
    if let Some((main, _)) = v.split_once('-') {
        main.to_string()
    } else {
        v.to_string()
    }
}

fn is_newer(current: &str, latest: &str) -> bool {
    let current_norm = normalize_version(current);
    let latest_norm = normalize_version(latest);

    let current_parts: Vec<u32> = current_norm
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let latest_parts: Vec<u32> = latest_norm
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();

    for i in 0..std::cmp::max(current_parts.len(), latest_parts.len()) {
        let curr = current_parts.get(i).cloned().unwrap_or(0);
        let lat = latest_parts.get(i).cloned().unwrap_or(0);
        if lat > curr {
            return true;
        } else if curr > lat {
            return false;
        }
    }
    false
}

fn open_browser(url: &str) {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_version() {
        assert_eq!(normalize_version("v0.1.0-1-gae62f96"), "0.1.0");
        assert_eq!(normalize_version("V1.2.3"), "1.2.3");
        assert_eq!(normalize_version("2.0.0"), "2.0.0");
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("v0.1.0-1-gae62f96", "v0.2.0"));
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(!is_newer("v0.2.0", "v0.1.0"));
        assert!(!is_newer("v0.1.0", "v0.1.0"));
        assert!(is_newer("v0.1.0", "1.0.0"));
    }

    #[test]
    fn test_reorder_provider_moves_to_item_row() {
        let mut order = vec![
            "openai".to_string(),
            "claude".to_string(),
            "codex".to_string(),
        ];
        let dragged = ProviderDragPayload {
            provider: "codex".to_string(),
            row: 2,
        };

        assert!(reorder_provider(
            &mut order,
            &dragged,
            ProviderDropTarget::Item {
                provider: "openai".to_string(),
                row: 0,
            },
            &["openai", "claude", "codex"],
        ));
        assert_eq!(&order[..3], ["codex", "openai", "claude"]);
    }

    #[test]
    fn test_reorder_provider_moves_to_visible_end_without_moving_hidden_slots() {
        let mut order = vec![
            "openai".to_string(),
            "deepseek".to_string(),
            "claude".to_string(),
            "codex".to_string(),
        ];
        let dragged = ProviderDragPayload {
            provider: "openai".to_string(),
            row: 0,
        };

        assert!(reorder_provider(
            &mut order,
            &dragged,
            ProviderDropTarget::End,
            &["openai", "claude", "codex"],
        ));
        assert_eq!(&order[..4], ["claude", "deepseek", "codex", "openai"]);
    }

    #[test]
    fn test_reorder_provider_ignores_same_position_drop() {
        let mut order = vec![
            "openai".to_string(),
            "claude".to_string(),
            "codex".to_string(),
        ];
        let dragged = ProviderDragPayload {
            provider: "claude".to_string(),
            row: 1,
        };

        assert!(!reorder_provider(
            &mut order,
            &dragged,
            ProviderDropTarget::Item {
                provider: "claude".to_string(),
                row: 1,
            },
            &["openai", "claude", "codex"],
        ));
        assert_eq!(&order[..3], ["openai", "claude", "codex"]);
    }

    #[test]
    fn test_enable_provider_for_test_does_not_require_ui_enabled_state() {
        let mut config = crate::config::AppConfig::default();
        config.openai.enabled = Some(false);
        config.opencode.enabled = Some(false);

        enable_provider_for_test(&mut config, "openai");
        enable_provider_for_test(&mut config, "opencodego");

        assert_eq!(config.openai.enabled, Some(true));
        assert_eq!(config.opencode.enabled, Some(true));
    }
}
