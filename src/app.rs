use eframe::egui;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::provider::UsageData;

pub struct QuotifyApp {
    pub data: Arc<RwLock<Vec<UsageData>>>,
    pub last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    pub config: crate::config::AppConfig,
}

impl QuotifyApp {
    pub fn new(
        data: Arc<RwLock<Vec<UsageData>>>,
        last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
        config: crate::config::AppConfig,
    ) -> Self {
        Self {
            data,
            last_refresh,
            config,
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

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Redraw every second to update the "Refreshed X seconds ago" counter,
        // but ONLY if the window is active/focused. When the window loses focus,
        // it hides itself. If we request repaint while hidden, winit's swapchain
        // will instantly fail and cause a 100% CPU busy loop trying to VSync.
        if ctx.input(|i| i.focused) {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }

        // Query the OS/system theme to support dynamic light/dark mode switching
        let is_dark = match ctx.system_theme() {
            Some(egui::Theme::Dark) => true,
            Some(egui::Theme::Light) => false,
            None => ctx.style().visuals.dark_mode,
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
        let mut style = (*ctx.style()).clone();
        style.text_styles = [
            (egui::TextStyle::Heading, egui::FontId::new(20.0, egui::FontFamily::Proportional)),
            (egui::TextStyle::Name("Title".into()), egui::FontId::new(28.0, egui::FontFamily::Proportional)),
            (egui::TextStyle::Body, egui::FontId::new(14.0, egui::FontFamily::Proportional)),
            (egui::TextStyle::Monospace, egui::FontId::new(14.0, egui::FontFamily::Monospace)),
            (egui::TextStyle::Button, egui::FontId::new(14.0, egui::FontFamily::Proportional)),
            (egui::TextStyle::Small, egui::FontId::new(12.0, egui::FontFamily::Proportional)),
        ].into();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        ctx.set_style(style);

        ctx.set_visuals(visuals);

        // Semi-transparent popup panel with rounded corners and subtle border
        let panel_bg = if is_dark {
            egui::Color32::from_rgba_premultiplied(32, 32, 32, 200)
        } else {
            egui::Color32::from_rgba_premultiplied(243, 243, 243, 200)
        };
        let border_color = if is_dark {
            egui::Color32::from_rgba_premultiplied(80, 80, 80, 140)
        } else {
            egui::Color32::from_rgba_premultiplied(200, 200, 200, 180)
        };

        let popup_frame = egui::Frame::NONE
            .fill(panel_bg)
            .stroke(egui::Stroke::new(1.0, border_color))
            .corner_radius(12)
            .inner_margin(12)
            .outer_margin(0);

        egui::CentralPanel::default()
            .frame(popup_frame)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                        ui.add_space(4.0);
                        ui.heading(egui::RichText::new("Quotify").strong().size(20.0));
                        ui.label(
                            egui::RichText::new("AI Provider Quota Monitor")
                                .weak()
                                .size(11.5),
                        );

                        let last = *self.last_refresh.read();
                        let elapsed = (chrono::Utc::now() - last).num_seconds();
                        let refresh_msg = if elapsed < 60 {
                            format!("Refreshed {elapsed}s ago")
                        } else {
                            format!("Refreshed {}m ago", elapsed / 60)
                        };
                        ui.label(egui::RichText::new(refresh_msg).small().weak());
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);

                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .hscroll(false)
                        .show(ui, |ui| {
                        let data = self.data.read().clone();
                        let all_providers = [
                            ("deepseek", "DeepSeek"),
                            ("claude", "Claude"),
                            ("gemini", "Gemini"),
                            ("antigravity", "Antigravity"),
                            ("codex", "Codex / OpenAI"),
                            ("opencode", "OpenCode"),
                            ("mimo", "MiMo"),
                        ];

                        for &(name, display_name) in &all_providers {
                            let provider_data = data.iter().find(|d| d.provider == name);
                            render_provider(ui, name, display_name, provider_data);
                            ui.add_space(8.0);
                        }
                    });
            });
    }
}

fn render_provider(
    ui: &mut egui::Ui,
    provider_name: &str,
    provider_display_name: &str,
    data: Option<&UsageData>,
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
        .inner_margin(12);

    card_frame.show(ui, |ui| {
        // Header Row
        ui.horizontal(|ui| {
            // Status Dot
            let dot_color = if is_dark {
                match status {
                    ProviderStatus::Active => egui::Color32::from_rgb(108, 203, 95), // Fluent Green
                    ProviderStatus::Error => egui::Color32::from_rgb(255, 108, 108), // Fluent Red
                    ProviderStatus::Disabled => egui::Color32::from_rgb(161, 161, 161), // Fluent Gray
                }
            } else {
                match status {
                    ProviderStatus::Active => egui::Color32::from_rgb(16, 124, 65), // Fluent Green (Darker)
                    ProviderStatus::Error => egui::Color32::from_rgb(196, 43, 28), // Fluent Red (Darker)
                    ProviderStatus::Disabled => egui::Color32::from_rgb(118, 118, 118), // Fluent Gray (Darker)
                }
            };
            let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(dot_rect.center(), 4.0, dot_color);
            ui.add_space(6.0);

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
        });

        match status {
            ProviderStatus::Disabled => {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(
                        "Provider not configured. Configure credentials to enable.",
                    )
                    .small()
                    .weak(),
                );
            }
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

                    egui::Grid::new(format!("grid_{provider_name}"))
                        .num_columns(4)
                        .spacing([12.0, 8.0])
                        .min_col_width(45.0)
                        .show(ui, |ui| {
                            for window in windows {
                                // Col 1: Label
                                ui.label(egui::RichText::new(&window.label).strong().size(11.0));

                                // Col 2: Fluent-styled Progress Bar & Percentage
                                ui.horizontal(|ui| {
                                    let pct = window.used_percent as f32;
                                    let bar_width = 100.0;
                                    let bar_height = 8.0;
                                    let rounding = 4.0;

                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(bar_width, bar_height),
                                        egui::Sense::hover(),
                                    );

                                    if ui.is_rect_visible(rect) {
                                        // Track color
                                        let track_color = if is_dark {
                                            egui::Color32::from_rgb(32, 32, 32)
                                        } else {
                                            egui::Color32::from_rgb(229, 229, 229)
                                        };

                                        ui.painter().rect_filled(rect, rounding, track_color);

                                        let fill_width = bar_width * (pct / 100.0).clamp(0.0, 1.0);
                                        if fill_width > 0.0 {
                                            let fill_rect = egui::Rect::from_min_size(
                                                rect.min,
                                                egui::vec2(fill_width, bar_height),
                                            );

                                            // Dynamic filling colors for Dark/Light modes
                                            let fill_color = if is_dark {
                                                if pct >= 80.0 {
                                                    egui::Color32::from_rgb(241, 112, 122) // Fluent Red (Dark)
                                                } else if pct >= 50.0 {
                                                    egui::Color32::from_rgb(255, 185, 0) // Fluent Gold (Dark)
                                                } else {
                                                    egui::Color32::from_rgb(96, 205, 255) // Fluent Accent Blue (Dark)
                                                }
                                            } else {
                                                if pct >= 80.0 {
                                                    egui::Color32::from_rgb(196, 43, 28) // Fluent Red (Light)
                                                } else if pct >= 50.0 {
                                                    egui::Color32::from_rgb(179, 123, 0) // Fluent Gold (Light)
                                                } else {
                                                    egui::Color32::from_rgb(0, 120, 212) // Fluent Accent Blue (Light)
                                                }
                                            };
                                            ui.painter()
                                                .rect_filled(fill_rect, rounding, fill_color);
                                        }
                                    }

                                    ui.add_space(4.0);

                                    let pct_color = if is_dark {
                                        if pct >= 80.0 {
                                            egui::Color32::from_rgb(241, 112, 122)
                                        } else if pct >= 50.0 {
                                            egui::Color32::from_rgb(255, 200, 0)
                                        } else {
                                            egui::Color32::from_rgb(96, 205, 255)
                                        }
                                    } else {
                                        if pct >= 80.0 {
                                            egui::Color32::from_rgb(196, 43, 28)
                                        } else if pct >= 50.0 {
                                            egui::Color32::from_rgb(179, 123, 0)
                                        } else {
                                            egui::Color32::from_rgb(0, 120, 212)
                                        }
                                    };

                                    ui.label(
                                        egui::RichText::new(format!("{pct:.0}%"))
                                            .color(pct_color)
                                            .strong()
                                            .size(10.0),
                                    );
                                });

                                // Col 3: Monospace Metrics
                                let count_text = if let Some(used) = window.used {
                                    let unit = window.unit.as_deref().unwrap_or("");
                                    if let Some(limit) = window.limit {
                                        format!("{used:.0}/{limit:.0} {unit}")
                                    } else {
                                        format!("{used:.0} {unit}")
                                    }
                                } else {
                                    "-".to_string()
                                };
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(count_text).monospace().size(10.0),
                                        );
                                    },
                                );

                                // Col 4: Muted Reset Time
                                let reset_text = if let Some(resets) = window.resets_at {
                                    let remaining = resets - chrono::Utc::now();
                                    if remaining.num_seconds() > 0 {
                                        let h = remaining.num_hours();
                                        let m = remaining.num_minutes() % 60;
                                        format!("resets in {h}h{m}m")
                                    } else {
                                        "resetting...".to_string()
                                    }
                                } else {
                                    "".to_string()
                                };
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(egui::RichText::new(reset_text).small().weak());
                                    },
                                );

                                ui.end_row();
                            }
                        });
                }
            }
        }
    });
}
