use eframe::egui;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::provider::UsageData;

pub struct QuotifyApp {
    pub data: Arc<RwLock<Vec<UsageData>>>,
    pub last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    pub config: crate::config::AppConfig,
    pub active_provider: Arc<RwLock<String>>,
}

impl QuotifyApp {
    pub fn new(
        data: Arc<RwLock<Vec<UsageData>>>,
        last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
        config: crate::config::AppConfig,
        active_provider: Arc<RwLock<String>>,
    ) -> Self {
        Self {
            data,
            last_refresh,
            config,
            active_provider,
        }
    }
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

        // Redraw every second to update the "Refreshed X seconds ago" counter,
        // but ONLY if the window is active/focused. When the window loses focus,
        // it hides itself. If we request repaint while hidden, winit's swapchain
        // will instantly fail and cause a 100% CPU busy loop trying to VSync.
        let is_visible = crate::tray::WINDOW_VISIBLE.load(std::sync::atomic::Ordering::SeqCst);
        if is_visible && ctx.input(|i| i.focused) {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }

        // Query the OS/system theme to support dynamic light/dark mode switching
        let is_dark = match ctx.system_theme() {
            Some(egui::Theme::Dark) => true,
            Some(egui::Theme::Light) => false,
            None => ctx.global_style().visuals.dark_mode,
        };

        let mut visuals = if is_dark {
            let mut v = egui::Visuals::dark();
            // Transparent window fill so DWM Mica backdrop shows through
            v.window_fill = egui::Color32::TRANSPARENT;
            v.panel_fill = egui::Color32::TRANSPARENT;
            v.extreme_bg_color = egui::Color32::from_rgba_premultiplied(26, 26, 26, 180);

            // Semi-transparent Acrylic Plate card backgrounds (Dark mode)
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
            // Transparent window fill so DWM Mica backdrop shows through
            v.window_fill = egui::Color32::TRANSPARENT;
            v.panel_fill = egui::Color32::TRANSPARENT;
            v.extreme_bg_color = egui::Color32::from_rgba_premultiplied(255, 255, 255, 200);

            // Semi-transparent Acrylic Plate card backgrounds (Light mode)
            v.widgets.noninteractive.bg_fill =
                egui::Color32::from_rgba_premultiplied(255, 255, 255, 180);
            v.widgets.inactive.bg_fill = egui::Color32::from_rgba_premultiplied(249, 249, 249, 180);
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
        ctx.set_global_style(style);

        ctx.set_visuals(visuals);

        // Semi-transparent popup panel to let native Mica show through.
        // We let Windows DWM handle the window rounded corners and native border/shadow,
        // avoiding drawing a second rounded border in egui to prevent mismatched curvatures.
        let panel_bg = if is_dark {
            egui::Color32::from_rgba_premultiplied(20, 20, 20, 30)
        } else {
            egui::Color32::from_rgba_premultiplied(255, 255, 255, 30)
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

                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(card_left_indent);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new("Quotify")
                                    .strong()
                                    .size(16.0)
                                    .line_height(Some(24.0)),
                            );

                            ui.add_space(1.0);

                            let settings = icon_button(
                                ui,
                                egui::include_image!("../assets/icons/settings.svg"),
                                egui::vec2(24.0, 24.0),
                                egui::vec2(16.0, 16.0),
                                "Open configuration file",
                            );
                            if settings.clicked()
                                && let Err(err) = open_config_file()
                            {
                                tracing::error!("Failed to open config file: {err}");
                            }
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let refresh = icon_button(
                                ui,
                                egui::include_image!("../assets/icons/refresh.svg"),
                                egui::vec2(24.0, 24.0),
                                egui::vec2(16.0, 16.0),
                                "Refresh usage now",
                            );
                            if refresh.clicked() {
                                crate::tray::request_refresh();
                                ctx.request_repaint();
                            }

                            ui.add_sized(
                                [60.0, 24.0],
                                egui::Label::new(
                                    egui::RichText::new(refresh_age)
                                        .small()
                                        .color(egui::Color32::from_rgb(150, 150, 150)),
                                )
                                .truncate(),
                            );
                        });
                    },
                );

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .hscroll(false)
                    .show(ui, |ui| {
                        let data = self.data.read().clone();
                        let all_providers = [
                            ("codex", "Codex"),
                            ("opencode", "OpenCode"),
                            ("claude", "Claude"),
                            ("gemini", "Gemini"),
                            ("antigravity", "Antigravity"),
                            ("deepseek", "DeepSeek"),
                            ("mimo", "MiMo"),
                        ];

                        for &(name, display_name) in &all_providers {
                            let provider_data = data.iter().find(|d| d.provider == name);
                            render_provider(
                                ui,
                                name,
                                display_name,
                                provider_data,
                                card_width,
                                &self.active_provider,
                                &self.config,
                                &self.data,
                            );
                            ui.add_space(6.0);
                        }
                    });
            });
    }
}

fn open_config_file() -> anyhow::Result<()> {
    let path = crate::config::AppConfig::config_path();
    if !path.exists() {
        crate::config::AppConfig::default().save_to(&path)?;
    }

    std::process::Command::new("notepad.exe")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(anyhow::Error::from)
}

fn icon_button(
    ui: &mut egui::Ui,
    image: egui::ImageSource<'static>,
    button_size: egui::Vec2,
    icon_size: egui::Vec2,
    tooltip: &str,
) -> egui::Response {
    let image = egui::Image::new(image)
        .fit_to_exact_size(icon_size)
        .maintain_aspect_ratio(true);
    ui.add_sized(
        button_size,
        egui::Button::image(image)
            .frame(true)
            .frame_when_inactive(false),
    )
    .on_hover_text(tooltip)
}

fn render_provider(
    ui: &mut egui::Ui,
    provider_name: &str,
    provider_display_name: &str,
    data: Option<&UsageData>,
    card_width: f32,
    active_provider: &Arc<RwLock<String>>,
    config: &crate::config::AppConfig,
    all_data: &Arc<RwLock<Vec<UsageData>>>,
) {
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
        ui.allocate_ui_with_layout(
            egui::vec2(card_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(card_width);
                ui.set_max_width(card_width);
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
                    all_data,
                );
            },
        );
    });
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
    config: &crate::config::AppConfig,
    all_data: &Arc<RwLock<Vec<UsageData>>>,
) {
    let response = card_frame.show(ui, |ui| {
        // Enforce uniform width across all cards based on parent width minus horizontal margins (cast i8 margins to f32)
        let margin_x = (card_frame.inner_margin.left
            + card_frame.inner_margin.right
            + card_frame.outer_margin.left
            + card_frame.outer_margin.right) as f32;
        let content_width = (card_width - margin_x).max(0.0);
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
                    if let Some(c) = credits {
                        let credit_text = format!("{:.2} {}", c.balance, c.currency);

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
                            egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
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
                if windows.is_empty() {
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

                    for window in windows {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;

                            ui.allocate_ui_with_layout(
                                egui::vec2(label_width, 18.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add_sized(
                                        [label_width, 18.0],
                                        egui::Label::new(
                                            egui::RichText::new(&window.label).strong().size(11.0),
                                        )
                                        .truncate(),
                                    );
                                },
                            );

                            ui.allocate_ui_with_layout(
                                egui::vec2(progress_width, 18.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    render_usage_progress(ui, window.used_percent as f32, is_dark);
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
                }
            }
        }
    });

    if response.response.rect.width() < card_width {
        ui.allocate_space(egui::vec2(card_width, 0.0));
    }
}

fn provider_icon(provider_name: &str, is_dark: bool) -> Option<egui::ImageSource<'static>> {
    match (provider_name, is_dark) {
        ("codex", true) => Some(egui::include_image!(
            "../assets/provider-icons/codex-dark.svg"
        )),
        ("codex", false) => Some(egui::include_image!("../assets/provider-icons/codex.svg")),
        ("opencode", true) => Some(egui::include_image!(
            "../assets/provider-icons/opencode-dark.svg"
        )),
        ("opencode", false) => Some(egui::include_image!(
            "../assets/provider-icons/opencode.svg"
        )),
        ("claude", _) => Some(egui::include_image!("../assets/provider-icons/claude.svg")),
        ("gemini", _) => Some(egui::include_image!("../assets/provider-icons/gemini.svg")),
        ("antigravity", _) => Some(egui::include_image!(
            "../assets/provider-icons/antigravity.svg"
        )),
        ("deepseek", _) => Some(egui::include_image!(
            "../assets/provider-icons/deepseek.svg"
        )),
        _ => None,
    }
}

fn render_provider_icon(
    ui: &mut egui::Ui,
    provider_name: &str,
    is_dark: bool,
    active_provider: &Arc<RwLock<String>>,
    config: &crate::config::AppConfig,
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
            set_active_provider(provider_name, active_provider, config, data);
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
            set_active_provider(provider_name, active_provider, config, data);
        }
        return true;
    }

    false
}

fn set_active_provider(
    provider_name: &str,
    active_provider: &Arc<RwLock<String>>,
    config: &crate::config::AppConfig,
    data: &Arc<RwLock<Vec<UsageData>>>,
) {
    *active_provider.write() = provider_name.to_string();

    let mut updated_config = config.clone();
    updated_config.general.active_provider = provider_name.to_string();
    if let Err(err) = updated_config.save() {
        tracing::error!("Failed to save active provider {provider_name}: {err}");
    }

    update_tray_icon_for_active_provider(provider_name, data);
    crate::tray::request_refresh();
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
