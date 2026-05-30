use eframe::egui;
use parking_lot::RwLock;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::provider::UsageData;

pub struct QuotifyApp {
    pub data: Arc<RwLock<Vec<UsageData>>>,
    pub last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    pub config: crate::config::AppConfig,
    pub config_path: Option<PathBuf>,
    pub active_provider: Arc<RwLock<String>>,
    drag: ProviderDragState,
    last_config_reload: Instant,
}

impl QuotifyApp {
    pub fn new(
        data: Arc<RwLock<Vec<UsageData>>>,
        last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
        config: crate::config::AppConfig,
        config_path: Option<PathBuf>,
        active_provider: Arc<RwLock<String>>,
    ) -> Self {
        Self {
            data,
            last_refresh,
            config,
            config_path,
            active_provider,
            drag: ProviderDragState::default(),
            last_config_reload: Instant::now(),
        }
    }
}

#[derive(Default)]
struct ProviderDragState {
    held_provider: Option<String>,
    hold_started: Option<Instant>,
    dragging_provider: Option<String>,
    order_dirty: bool,
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
        self.reload_config_if_due();

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

        let is_mica = crate::IS_MICA_ACTIVE.load(std::sync::atomic::Ordering::SeqCst);
        let mut visuals = if is_dark {
            let mut v = egui::Visuals::dark();
            // Transparent window fill so DWM Mica backdrop shows through
            v.window_fill = if is_mica {
                egui::Color32::TRANSPARENT
            } else {
                egui::Color32::from_rgb(32, 32, 32)
            };
            v.panel_fill = if is_mica {
                egui::Color32::TRANSPARENT
            } else {
                egui::Color32::from_rgb(32, 32, 32)
            };
            v.extreme_bg_color = if is_mica {
                egui::Color32::from_rgba_premultiplied(26, 26, 26, 180)
            } else {
                egui::Color32::from_rgb(26, 26, 26)
            };

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
            v.window_fill = if is_mica {
                egui::Color32::TRANSPARENT
            } else {
                egui::Color32::from_rgb(243, 243, 243)
            };
            v.panel_fill = if is_mica {
                egui::Color32::TRANSPARENT
            } else {
                egui::Color32::from_rgb(243, 243, 243)
            };
            v.extreme_bg_color = if is_mica {
                egui::Color32::from_rgba_premultiplied(255, 255, 255, 200)
            } else {
                egui::Color32::from_rgb(255, 255, 255)
            };

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
            if is_mica {
                egui::Color32::from_rgba_premultiplied(20, 20, 20, 30)
            } else {
                egui::Color32::from_rgb(32, 32, 32)
            }
        } else {
            if is_mica {
                egui::Color32::from_rgba_premultiplied(255, 255, 255, 30)
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

                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(card_left_indent);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
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

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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

                            ui.add_space(2.0);

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

                            ui.add_space(4.0);

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
                        let all_providers = provider_display_order(&self.config);

                        let dragging_any = self.drag.dragging_provider.is_some();
                        let mut shown = 0usize;
                        for (name, display_name) in all_providers {
                            let Some(provider_data) = data.iter().find(|d| d.provider == name) else {
                                continue;
                            };
                            let collapse = dragging_any && self.drag.dragging_provider.as_deref() != Some(&name);
                            let is_dragged = self.drag.dragging_provider.as_deref() == Some(&name);
                            let response = render_provider(
                                ui,
                                &name,
                                display_name,
                                Some(provider_data),
                                card_width,
                                &self.active_provider,
                                &self.config,
                                self.config_path.as_ref(),
                                &self.data,
                                collapse,
                                is_dragged,
                            );
                            self.handle_provider_drag(&ctx, &response, &name);
                            shown += 1;
                            ui.add_space(6.0);
                        }

                        // Autoscroll during drag-and-drop reordering
                        if self.drag.dragging_provider.is_some() {
                            if let Some(pointer_pos) = ctx.input(|i| i.pointer.hover_pos()) {
                                let viewport = ui.clip_rect(); // Get visible viewport rect in screen coordinates
                                let scroll_margin = 40.0;      // Distance from boundary to trigger scroll
                                let scroll_step = 15.0;        // Scroll step size

                                if pointer_pos.y < viewport.min.y + scroll_margin {
                                    // Near top: scroll UP by targeting a rect just above the viewport
                                    let target_y = viewport.min.y - scroll_step;
                                    let target_rect = egui::Rect::from_center_size(
                                        egui::pos2(viewport.center().x, target_y),
                                        egui::vec2(card_width, 10.0),
                                    );
                                    ui.scroll_to_rect(target_rect, Some(egui::Align::TOP));
                                    ctx.request_repaint(); // Keep repainting to animate continuous scroll
                                } else if pointer_pos.y > viewport.max.y - scroll_margin {
                                    // Near bottom: scroll DOWN by targeting a rect just below the viewport
                                    let target_y = viewport.max.y + scroll_step;
                                    let target_rect = egui::Rect::from_center_size(
                                        egui::pos2(viewport.center().x, target_y),
                                        egui::vec2(card_width, 10.0),
                                    );
                                    ui.scroll_to_rect(target_rect, Some(egui::Align::BOTTOM));
                                    ctx.request_repaint(); // Keep repainting to animate continuous scroll
                                }
                            }
                        }

                        self.finish_provider_drag_if_released(&ctx);

                        if shown == 0 {
                            ui.vertical_centered(|ui| {
                                ui.add_space(48.0);
                                ui.label(
                                    egui::RichText::new(
                                        "No enabled providers. Configure credentials to enable cards.",
                                    )
                                    .color(egui::Color32::from_rgb(150, 150, 150)),
                                );
                            });
                        }
                    });

                // If a card is being dragged, render a floating preview of the card that follows the mouse cursor
                if let Some(dragging_name) = &self.drag.dragging_provider {
                    if let Some(pointer_pos) = ctx.input(|i| i.pointer.hover_pos()) {
                        let preview_pos = pointer_pos - egui::vec2(card_width / 2.0, 12.0);

                        egui::Area::new(egui::Id::new("provider_drag_preview"))
                            .fixed_pos(preview_pos)
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
                                if let Some(provider_data) = data.iter().find(|d| d.provider == *dragging_name) {
                                    let display_name = provider_catalog()
                                        .iter()
                                        .find(|(id, _)| id.eq_ignore_ascii_case(dragging_name))
                                        .map(|(_, d)| *d)
                                        .unwrap_or(dragging_name)
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
                                            dragging_name,
                                            &display_name,
                                            status,
                                            credits,
                                            error_msg,
                                            windows,
                                            is_dark,
                                            card_frame,
                                            card_width,
                                            &self.active_provider,
                                            &self.config,
                                            self.config_path.as_ref(),
                                            &self.data,
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
        if self.drag.dragging_provider.is_some()
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
            Ok(config) => {
                self.config = config;
                *self.active_provider.write() =
                    self.config.general.active_provider.trim().to_string();
            }
            Err(err) => tracing::debug!("Failed to reload UI config: {err}"),
        }
    }

    fn save_config(&self) {
        let result = if let Some(path) = &self.config_path {
            self.config.save_to(path)
        } else {
            self.config.save()
        };
        if let Err(err) = result {
            tracing::error!("Failed to save provider order: {err}");
        }
    }

    fn handle_provider_drag(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        provider_name: &str,
    ) {
        let pointer_down = ctx.input(|i| i.pointer.primary_down());
        if !pointer_down {
            return;
        }

        let now = Instant::now();
        if response.hovered() && self.drag.held_provider.is_none() {
            self.drag.held_provider = Some(provider_name.to_string());
            self.drag.hold_started = Some(now);
        }

        if self.drag.held_provider.as_deref() == Some(provider_name)
            && self.drag.dragging_provider.is_none()
            && self
                .drag
                .hold_started
                .is_some_and(|started| now.duration_since(started) >= Duration::from_millis(350))
            && response.dragged()
        {
            self.drag.dragging_provider = Some(provider_name.to_string());
        }

        let Some(dragging_provider) = self.drag.dragging_provider.clone() else {
            return;
        };

        // During dragging, egui locks pointer focus to the dragged card, so response.hovered()
        // returns false for all other cards. We bypass this by manually checking if the pointer is within the rect.
        let hovered = if let Some(pointer_pos) = ctx.input(|i| i.pointer.hover_pos()) {
            response.rect.contains(pointer_pos)
        } else {
            false
        };

        if dragging_provider == provider_name || !hovered {
            return;
        }

        if reorder_provider(
            &mut self.config.general.provider_order,
            &dragging_provider,
            provider_name,
        ) {
            self.drag.order_dirty = true;
            ctx.request_repaint();
        }
    }

    fn finish_provider_drag_if_released(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.pointer.primary_down()) {
            return;
        }

        if self.drag.order_dirty {
            self.save_config();
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

fn reorder_provider(order: &mut Vec<String>, dragged: &str, target: &str) -> bool {
    ensure_provider_order(order);
    let Some(from) = order.iter().position(|id| id == dragged) else {
        return false;
    };
    let Some(to) = order.iter().position(|id| id == target) else {
        return false;
    };
    if from == to {
        return false;
    }

    let item = order.remove(from);
    let target_index = order.iter().position(|id| id == target).unwrap_or(to);
    order.insert(target_index, item);
    true
}

#[allow(clippy::too_many_arguments)]
fn render_provider(
    ui: &mut egui::Ui,
    provider_name: &str,
    provider_display_name: &str,
    data: Option<&UsageData>,
    card_width: f32,
    active_provider: &Arc<RwLock<String>>,
    config: &crate::config::AppConfig,
    config_path: Option<&PathBuf>,
    all_data: &Arc<RwLock<Vec<UsageData>>>,
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
                    collapse,
                )
            },
        );
        inner.inner.interact(egui::Sense::click_and_drag())
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
    config: &crate::config::AppConfig,
    config_path: Option<&PathBuf>,
    all_data: &Arc<RwLock<Vec<UsageData>>>,
    collapse: bool,
) -> egui::Response {
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
    config: &crate::config::AppConfig,
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
    config: &crate::config::AppConfig,
    config_path: Option<&PathBuf>,
    data: &Arc<RwLock<Vec<UsageData>>>,
) {
    *active_provider.write() = provider_name.to_string();

    let mut updated_config = config.clone();
    updated_config.general.active_provider = provider_name.to_string();
    let result = if let Some(path) = config_path {
        updated_config.save_to(path)
    } else {
        updated_config.save()
    };
    if let Err(err) = result {
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
