use gpui::prelude::{FluentBuilder, InteractiveElement, ParentElement, Styled};
use gpui::*;
use gpui_component::{
    Disableable, IndexPath, Selectable, Sizable,
    alert::Alert,
    button::{Button, ButtonVariants},
    divider::Divider,
    group_box::{GroupBox, GroupBoxVariants},
    input::{Input, InputEvent, InputState},
    link::Link,
    progress::Progress,
    scroll::ScrollableElement,
    select::{Select, SelectEvent, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    tag::Tag,
};
use parking_lot::RwLock;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, atomic::Ordering},
    time::Duration,
};

use crate::provider::UsageData;

static COMPONENT_THEME_SETTING: OnceLock<RwLock<String>> = OnceLock::new();

fn component_theme_setting() -> &'static RwLock<String> {
    COMPONENT_THEME_SETTING.get_or_init(|| RwLock::new("system".to_string()))
}

pub fn current_component_theme_setting() -> String {
    component_theme_setting().read().clone()
}

pub fn apply_component_theme(theme_setting: &str, window: Option<&mut Window>, cx: &mut App) {
    *component_theme_setting().write() = theme_setting.to_string();

    let mode = match theme_setting {
        "dark" => gpui_component::ThemeMode::Dark,
        "light" => gpui_component::ThemeMode::Light,
        _ => window
            .as_ref()
            .map(|window| window.appearance())
            .unwrap_or_else(|| cx.window_appearance())
            .into(),
    };

    gpui_component::Theme::change(mode, window, cx);

    let theme = gpui_component::Theme::global_mut(cx);
    theme.font_family = "Segoe UI".into();
    theme.font_size = px(14.0);
    theme.radius = px(6.0);
    theme.radius_lg = px(10.0);
    theme.background = Hsla::transparent_black();
    theme.primary = gpui::rgb(0x0067c0).into();
    theme.primary_hover = gpui::rgb(0x1975c5).into();
    theme.primary_active = gpui::rgb(0x005a9e).into();
    theme.ring = gpui::rgb(0x0067c0).into();
    theme.progress_bar = gpui::rgb(0x0067c0).into();

    if mode.is_dark() {
        theme.group_box = gpui::rgba(0x2b2b2baa).into();
        theme.popover = gpui::rgba(0x2b2b2bf2).into();
        theme.border = gpui::rgba(0xffffff26).into();
        theme.input = gpui::rgba(0xffffff3d).into();
        theme.muted = gpui::rgba(0xffffff24).into();
        theme.switch = gpui::rgba(0xffffff33).into();
    } else {
        theme.group_box = gpui::rgba(0xffffffaa).into();
        theme.popover = gpui::rgba(0xf9f9f9f2).into();
        theme.border = gpui::rgba(0x0000001f).into();
        theme.input = gpui::rgba(0x0000003d).into();
        theme.muted = gpui::rgba(0x00000014).into();
        theme.switch = gpui::rgba(0x0000002e).into();
    }
}

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
pub enum ProviderTestStatus {
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

#[derive(Default, Clone)]
pub struct ProviderDragState {
    held_provider: Option<String>,
    drag_start_pos: Option<Point<Pixels>>,
    current_mouse_pos: Option<Point<Pixels>>,
    dragging: Option<String>,
}

struct InputFieldState {
    input: Entity<InputState>,
    masked: bool,
    _subscription: Subscription,
}

struct ProviderSelectFieldState {
    select: Entity<SelectState<Vec<String>>>,
    _subscription: Subscription,
}

struct RefreshSliderFieldState {
    slider: Entity<SliderState>,
    _subscription: Subscription,
}

pub struct QuotifyApp {
    pub data: Arc<RwLock<Vec<UsageData>>>,
    pub last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    pub history: Arc<RwLock<crate::usage_history::UsageHistory>>,
    pub config: crate::config::AppConfig,
    pub config_path: Option<PathBuf>,
    pub active_provider: Arc<RwLock<String>>,
    pub drag: ProviderDragState,
    pub update_status: Arc<parking_lot::Mutex<UpdateStatus>>,
    pub provider_test_status: Arc<parking_lot::Mutex<ProviderTestStatus>>,
    pub selected_setting_provider: String,
    pub show_secrets: bool,
}

impl QuotifyApp {
    pub fn new(
        data: Arc<RwLock<Vec<UsageData>>>,
        last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
        config: crate::config::AppConfig,
        config_path: Option<PathBuf>,
        active_provider: Arc<RwLock<String>>,
        history: Arc<RwLock<crate::usage_history::UsageHistory>>,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            data,
            last_refresh,
            history,
            config,
            config_path,
            active_provider,
            drag: ProviderDragState::default(),
            update_status: Arc::new(parking_lot::Mutex::new(UpdateStatus::Idle)),
            provider_test_status: Arc::new(parking_lot::Mutex::new(ProviderTestStatus::Idle)),
            selected_setting_provider: "openai".to_string(),
            show_secrets: false,
        }
    }

    fn save_config(&self) {
        let mut config_to_save = self.config.clone();
        crate::secrets::store_and_scrub_config(&mut config_to_save);
        let res = if let Some(ref path) = self.config_path {
            config_to_save.save_to(path)
        } else {
            config_to_save.save()
        };
        if let Err(err) = res {
            tracing::error!("Failed to save config: {err}");
        }
    }

    fn trigger_provider_test(&self, provider_id: String, cx: &mut Context<Self>) {
        self.save_config();

        let mut config = self.config.clone();
        crate::secrets::hydrate_config(&mut config);
        enable_provider_for_test(&mut config, &provider_id);

        *self.provider_test_status.lock() = ProviderTestStatus::Testing {
            provider: provider_id.clone(),
        };

        let status = self.provider_test_status.clone();
        cx.spawn(|this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                let provider_id_clone = provider_id.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let provider = crate::create_provider(&provider_id_clone, &config)
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "Provider could not be created from the current settings"
                                )
                            })?;
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()?;
                        rt.block_on(provider.fetch_usage())
                    })
                    .await;

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

                cx.update(|cx| {
                    this.update(cx, |_view, cx| {
                        cx.notify();
                    })
                    .ok();
                })
                .ok();
            }
        })
        .detach();
    }

    fn trigger_check_update(&self, cx: &mut Context<Self>) {
        *self.update_status.lock() = UpdateStatus::Checking;
        cx.notify();

        let status = self.update_status.clone();
        cx.spawn(|this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let client = reqwest::Client::builder()
                            .timeout(Duration::from_secs(10))
                            .build()?;
                        let resp = client
                            .get("https://api.github.com/repos/zuoxinyu/quotify/releases/latest")
                            .header("User-Agent", "Quotify-App")
                            .send()
                            .await?
                            .json::<serde_json::Value>()
                            .await?;

                        let latest_tag = resp["tag_name"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("No tag_name"))?
                            .to_string();
                        let release_url = resp["html_url"]
                            .as_str()
                            .unwrap_or("https://github.com/zuoxinyu/quotify/releases")
                            .to_string();
                        let ret: anyhow::Result<(String, String)> = Ok((latest_tag, release_url));
                        ret
                    })
                    .await;

                *status.lock() = match result {
                    Ok((latest_tag, release_url)) => {
                        let current = env!("GIT_TAG");
                        if is_newer(current, &latest_tag) {
                            UpdateStatus::NewVersionAvailable {
                                latest_version: latest_tag,
                                release_url,
                            }
                        } else {
                            UpdateStatus::UpToDate {
                                latest_version: latest_tag,
                            }
                        }
                    }
                    Err(err) => UpdateStatus::Error(err.to_string()),
                };

                cx.update(|cx| {
                    this.update(cx, |_view, cx| {
                        cx.notify();
                    })
                    .ok();
                })
                .ok();
            }
        })
        .detach();
    }
}

impl Render for QuotifyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_page = crate::tray::ACTIVE_PAGE.load(Ordering::SeqCst);

        // Determine Theme colors
        let is_dark = match self.config.general.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                cx.window_appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };

        // UI style tokens (single hex parameter for gpui::rgba in 0.2.2)
        let mica_active = crate::IS_MICA_ACTIVE.load(Ordering::SeqCst);
        let bg_fill = if mica_active {
            gpui::rgba(0x00000000)
        } else if is_dark {
            gpui::rgb(0x202020)
        } else {
            gpui::rgb(0xf3f3f3)
        };
        let text_color = if is_dark {
            gpui::rgb(0xffffff)
        } else {
            gpui::rgb(0x000000)
        };
        let border_color = if is_dark {
            gpui::rgba(0x55555566)
        } else {
            gpui::rgba(0xffffff99)
        };

        // Outer layout container matching Windows 11 Mica backdrop popup dimensions 400x520
        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .p(px(12.0))
            .bg(bg_fill)
            .text_color(text_color)
            .font_family("Segoe UI")
            .child(
                // Header block
                self.render_header(active_page, is_dark, cx),
            )
            .child(
                // Line separator
                Divider::horizontal().mt(px(4.0)).color(border_color),
            )
            .child(
                // Body View
                div()
                    .flex_1()
                    .w_full()
                    .px(px(12.0))
                    .py(px(10.0))
                    .child(match active_page {
                        1 => self.render_about(cx).into_any_element(),
                        2 => self.render_settings(is_dark, window, cx).into_any_element(),
                        _ => self.render_dashboard(is_dark, cx).into_any_element(),
                    })
                    .id("body_view")
                    .overflow_y_scrollbar(),
            )
    }
}

impl QuotifyApp {
    fn render_header(&self, active_page: u32, is_dark: bool, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let weak_text = if is_dark {
            gpui::rgba(0xffffff99)
        } else {
            gpui::rgba(0x00000099)
        };

        let refresh_age = {
            let last = *self.last_refresh.read();
            let elapsed = chrono::Utc::now() - last;
            let secs = elapsed.num_seconds();
            if secs < 0 {
                "just now".to_string()
            } else if secs < 60 {
                format!("{secs}s ago")
            } else {
                format!("{}m ago", secs / 60)
            }
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .w_full()
            .h(px(32.0)) // Matches egui's header row height (approx 28px + small margin)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(if active_page != 0 {
                        // Back Button
                        Button::new("back_btn")
                            .ghost()
                            .xsmall()
                            .w(px(26.0))
                            .h(px(26.0))
                            .font_family("Segoe Fluent Icons")
                            .font_weight(gpui::FontWeight::THIN)
                            .text_size(px(12.0))
                            .label("\u{E72B}")
                            .tooltip("Back")
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    crate::tray::ACTIVE_PAGE.store(0, Ordering::SeqCst);
                                    app.update(cx, |_, cx| cx.notify()).ok();
                                }
                            })
                    } else {
                        // App Logo
                        Button::new("app_logo")
                            .ghost()
                            .xsmall()
                            .w(px(26.0))
                            .h(px(26.0))
                            .child(img("assets/icons/quotify.svg").w(px(18.0)).h(px(18.0)))
                            .tooltip("About Quotify")
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    crate::tray::ACTIVE_PAGE.store(1, Ordering::SeqCst);
                                    app.update(cx, |_, cx| cx.notify()).ok();
                                }
                            })
                    })
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_size(px(16.0))
                            .child(match active_page {
                                1 => "About",
                                2 => "Settings",
                                _ => "Quotify",
                            }),
                    ),
            )
            .child(if active_page == 0 {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(weak_text)
                            .child(refresh_age),
                    )
                    .child(
                        // Refresh button
                        Button::new("refresh_btn")
                            .ghost()
                            .xsmall()
                            .w(px(26.0))
                            .h(px(26.0))
                            .font_family("Segoe Fluent Icons")
                            .font_weight(gpui::FontWeight::THIN)
                            .text_size(px(12.0))
                            .label("\u{E72C}")
                            .tooltip("Refresh usage")
                            .on_click(move |_, _, _| {
                                crate::tray::request_refresh();
                            }),
                    )
                    .child(
                        // Settings button
                        Button::new("settings_btn")
                            .ghost()
                            .xsmall()
                            .w(px(26.0))
                            .h(px(26.0))
                            .font_family("Segoe Fluent Icons")
                            .font_weight(gpui::FontWeight::THIN)
                            .text_size(px(12.0))
                            .label("\u{E713}")
                            .tooltip("Settings")
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    crate::tray::ACTIVE_PAGE.store(2, Ordering::SeqCst);
                                    app.update(cx, |_, cx| cx.notify()).ok();
                                }
                            }),
                    )
            } else {
                div()
            })
            .into_any_element()
    }

    fn render_dashboard(&self, is_dark: bool, cx: &mut Context<Self>) -> AnyElement {
        let data = self.data.read().clone();
        let all_providers = provider_display_order(&self.config);
        let visible_providers = all_providers
            .into_iter()
            .filter(|(name, _)| data.iter().any(|d| d.provider == *name))
            .collect::<Vec<_>>();

        if visible_providers.is_empty() {
            return div()
                .flex()
                .flex_col()
                .items_center()
                .w_full()
                .pt_8()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(if is_dark {
                            gpui::rgba(0xffffff7f)
                        } else {
                            gpui::rgba(0x0000007f)
                        })
                        .child("No enabled providers. Configure credentials to enable cards."),
                )
                .into_any_element();
        }

        let mut cards = Vec::new();
        for (idx, (name, display_name)) in visible_providers.iter().enumerate() {
            if let Some(pdata) = data.iter().find(|d| d.provider == *name) {
                cards.push(self.render_provider_card(
                    name,
                    SharedString::from(*display_name),
                    pdata,
                    idx,
                    is_dark,
                    cx,
                ));
            }
        }

        // Render card preview floating at cursor position if dragged
        let drag_preview = if let Some(ref dragged_prov) = self.drag.dragging {
            if let Some(mouse_pos) = self.drag.current_mouse_pos {
                if let Some(pdata) = data.iter().find(|d| d.provider == *dragged_prov) {
                    let display_name = provider_catalog()
                        .iter()
                        .find(|(id, _)| id.eq_ignore_ascii_case(dragged_prov))
                        .map(|(_, name)| SharedString::from(*name))
                        .unwrap_or_else(|| SharedString::from(dragged_prov.clone()));
                    let dx = mouse_pos.x / px(1.0);
                    let dy = mouse_pos.y / px(1.0);
                    Some(
                        div()
                            .absolute()
                            .left(px(dx - 150.0)) // center preview horizontally around cursor
                            .top(px(dy - 25.0))
                            .child(self.render_provider_card(
                                dragged_prov,
                                display_name,
                                pdata,
                                999,
                                is_dark,
                                cx,
                            )),
                    )
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        div()
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .gap_5()
            .py(px(8.0))
            .children(cards)
            .children(drag_preview)
            .into_any_element()
    }

    fn render_provider_card(
        &self,
        name: &str,
        display_name: SharedString,
        data: &UsageData,
        row_idx: usize,
        is_dark: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let provider_name = name.to_string();
        let card_id = provider_name.clone();
        let card_elt_id = SharedString::from(format!("card_{name}"));

        let trend = self.history.read().trend_for(name, 7);

        div()
            .w_full()
            .id(card_elt_id)
            .child(
                GroupBox::new()
                    .fill()
                    .child(self.render_card_header(name, display_name, data, is_dark))
                    .child(self.render_card_body(data, trend, is_dark)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(
                    move |this: &mut Self,
                          event: &MouseDownEvent,
                          _window: &mut gpui::Window,
                          cx: &mut gpui::Context<Self>| {
                        this.drag.held_provider = Some(card_id.clone());
                        this.drag.drag_start_pos = Some(event.position);
                        cx.notify();
                    },
                ),
            )
            .on_mouse_move(cx.listener(
                move |this: &mut Self,
                      event: &MouseMoveEvent,
                      _window: &mut gpui::Window,
                      cx: &mut gpui::Context<Self>| {
                    if let Some(ref held) = this.drag.held_provider {
                        if let Some(start_pos) = this.drag.drag_start_pos {
                            let dx = (event.position.x - start_pos.x) / px(1.0);
                            let dy = (event.position.y - start_pos.y) / px(1.0);
                            let dist = dx.abs() + dy.abs();
                            if dist > 6.0 {
                                this.drag.dragging = Some(held.clone());
                            }
                        }
                        this.drag.current_mouse_pos = Some(event.position);
                        cx.notify();
                    }
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(
                    move |this: &mut Self,
                          _event: &MouseUpEvent,
                          _window: &mut gpui::Window,
                          cx: &mut gpui::Context<Self>| {
                        if let Some(ref dragged) = this.drag.dragging {
                            let mut order = this.config.general.provider_order.clone();
                            if let Some(pos) = order.iter().position(|p| p == dragged) {
                                order.remove(pos);
                            }
                            let drop_idx = row_idx.min(order.len());
                            order.insert(drop_idx, dragged.clone());
                            this.config.general.provider_order = order;
                            this.save_config();
                        }
                        this.drag = ProviderDragState::default();
                        cx.notify();
                    },
                ),
            )
            .into_any_element()
    }

    fn render_card_header(
        &self,
        name: &str,
        display_name: SharedString,
        data: &UsageData,
        is_dark: bool,
    ) -> AnyElement {
        let is_primary = self.active_provider.read().eq_ignore_ascii_case(name);

        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(if let Some(icon_path) = provider_icon(name, is_dark) {
                        div()
                            .w(px(16.0))
                            .h(px(16.0))
                            .child(img(icon_path).w_full().h_full())
                    } else {
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded_full()
                            .bg(if is_dark {
                                gpui::rgb(0x364559)
                            } else {
                                gpui::rgb(0xe8f0ff)
                            })
                            .text_color(if is_dark {
                                gpui::rgb(0xd2e1ff)
                            } else {
                                gpui::rgb(0x254682)
                            })
                            .text_size(px(10.0))
                            .font_weight(gpui::FontWeight::EXTRA_BOLD)
                            .child("M")
                    })
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_size(px(12.0))
                            .child(display_name),
                    )
                    .child(if is_primary {
                        Tag::primary()
                            .small()
                            .outline()
                            .child("PRIMARY")
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(if let Some(ref credits) = data.credits {
                let text = format!(
                    "{} {}",
                    format_credits_balance(credits.balance),
                    credits.currency
                );
                Tag::info().small().outline().child(text).into_any_element()
            } else {
                div().into_any_element()
            })
            .into_any_element()
    }

    fn render_card_body(
        &self,
        data: &UsageData,
        trend: Option<crate::usage_history::ProviderTrend>,
        is_dark: bool,
    ) -> AnyElement {
        if let Some(ref error) = data.error {
            return Alert::error("provider-error", error.clone())
                .small()
                .into_any_element();
        }

        let mut children = data
            .windows
            .iter()
            .map(|w| Self::render_progress_row(w, is_dark))
            .collect::<Vec<_>>();

        if let Some(trend_val) = trend {
            let trend_text = format_trend_summary(&trend_val);
            children.push(
                div()
                    .mt_2() // Top margin for trend summary matches layout spacing
                    .text_size(px(10.0))
                    .text_color(if is_dark {
                        gpui::rgba(0xffffff7f)
                    } else {
                        gpui::rgba(0x0000007f)
                    })
                    .child(trend_text)
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(px(12.0)) // 8px gap between progress rows matches egui's default spacing
            .children(children)
            .into_any_element()
    }

    fn render_progress_row(w: &crate::provider::UsageWindow, is_dark: bool) -> AnyElement {
        let pct = w.used_percent.clamp(0.0, 100.0);
        let fill_color = if pct >= 80.0 {
            if is_dark {
                gpui::rgb(0xf1707a)
            } else {
                gpui::rgb(0xc42b1c)
            }
        } else if pct >= 50.0 {
            if is_dark {
                gpui::rgb(0xffc800)
            } else {
                gpui::rgb(0xb37b00)
            }
        } else {
            if is_dark {
                gpui::rgb(0x60cdff)
            } else {
                gpui::rgb(0x0078d4)
            }
        };

        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .w(px(88.0))
                    .justify_center()
                    .font_family("Segoe UI")
                    .font_weight(gpui::FontWeight::EXTRA_LIGHT)
                    .text_size(px(11.0))
                    .child(w.label.clone()),
            )
            .child(
                Progress::new()
                    .value(pct as f32)
                    .bg(fill_color)
                    .flex_1()
                    .h(px(8.0)),
            )
            .child(
                div()
                    .w(px(34.0))
                    .text_size(px(10.0))
                    .text_color(fill_color)
                    .font_family("Segoe UI")
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(format!("{pct:.0}%")),
            )
            .child(
                div()
                    .w(px(42.0))
                    .flex()
                    .justify_end()
                    .text_size(px(10.0))
                    .text_color(if is_dark {
                        gpui::rgba(0xffffff7f)
                    } else {
                        gpui::rgba(0x0000007f)
                    })
                    .child(reset_time_text(w.resets_at)),
            )
            .into_any_element()
    }

    fn render_about(&self, cx: &mut Context<Self>) -> AnyElement {
        let ver = env!("GIT_TAG");

        GroupBox::new()
            .fill()
            .child(self.render_about_header(ver))
            .child(Divider::horizontal())
            .child(self.render_update_section(cx))
            .into_any_element()
    }

    fn render_about_header(&self, ver: &str) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(42.0))
                            .h(px(42.0))
                            .child(img("assets/icons/quotify.svg").w_full().h_full()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_size(px(18.0))
                                    .child("Quotify"),
                            )
                            .child(div().text_size(px(12.0)).child(format!("Version: {ver}"))),
                    ),
            )
            .child(div().text_size(px(12.0)).child("Author: zuoxinyu"))
            .child(
                div().flex().gap_2().child("GitHub: ").child(
                    Link::new("github_link")
                        .href("https://github.com/zuoxinyu/quotify")
                        .child("zuoxinyu/quotify"),
                ),
            )
            .into_any_element()
    }

    fn render_update_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let status = self.update_status.lock().clone();
        let checking = matches!(&status, UpdateStatus::Checking);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_size(px(14.0))
                    .child("Check for Updates"),
            )
            .child(
                Button::new("check_updates_btn")
                    .primary()
                    .small()
                    .w(px(130.0))
                    .label("Check now")
                    .loading(checking)
                    .disabled(checking)
                    .on_click(move |_, _, cx| {
                        app.update(cx, |this, cx| this.trigger_check_update(cx))
                            .ok();
                    }),
            )
            .child(match status {
                UpdateStatus::Checking => Alert::info("update-checking", "Checking for updates...")
                    .small()
                    .into_any_element(),
                UpdateStatus::UpToDate { .. } => {
                    Alert::success("update-current", "App is up to date.")
                        .small()
                        .into_any_element()
                }
                UpdateStatus::NewVersionAvailable {
                    latest_version,
                    release_url,
                } => div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        Alert::warning(
                            "update-available",
                            format!("New version {latest_version} available!"),
                        )
                        .small(),
                    )
                    .child(
                        Link::new("view_release_page_link")
                            .href(release_url)
                            .text_size(px(11.0))
                            .child("View Release Page"),
                    )
                    .into_any_element(),
                UpdateStatus::Error(err) => {
                    Alert::error("update-error", format!("Update check failed: {err}"))
                        .small()
                        .into_any_element()
                }
                _ => div().into_any_element(),
            })
            .into_any_element()
    }

    fn render_settings(
        &self,
        is_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.render_general_settings(is_dark, window, cx))
            .child(self.render_provider_settings(window, cx))
            .child(self.render_settings_footer())
            .child(div().h(px(20.0)))
            .into_any_element()
    }

    fn render_general_settings(
        &self,
        is_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let start_with_windows_app = cx.entity().downgrade();
        let theme_app = cx.entity().downgrade();
        let secondary_text = if is_dark {
            gpui::rgba(0xffffff99)
        } else {
            gpui::rgba(0x00000099)
        };
        let refresh_intervals = [30_u64, 60, 300, 1800, 3600];
        let refresh_index = refresh_intervals
            .iter()
            .enumerate()
            .min_by_key(|(_, value)| value.abs_diff(self.config.general.refresh_interval))
            .map(|(index, _)| index)
            .unwrap_or(0);
        let slider_app = cx.entity().downgrade();
        let refresh_slider = window.use_keyed_state("refresh-interval-slider", cx, move |_, cx| {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max((refresh_intervals.len() - 1) as f32)
                    .step(1.0)
                    .default_value(refresh_index as f32)
            });
            let _subscription = cx.subscribe(&slider, move |_, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(value) = event;
                let index = value
                    .end()
                    .round()
                    .clamp(0.0, (refresh_intervals.len() - 1) as f32)
                    as usize;
                let refresh_interval = refresh_intervals[index];
                slider_app
                    .update(cx, |this, cx| {
                        if this.config.general.refresh_interval != refresh_interval {
                            this.config.general.refresh_interval = refresh_interval;
                            this.save_config();
                            cx.notify();
                        }
                    })
                    .ok();
            });
            RefreshSliderFieldState {
                slider,
                _subscription,
            }
        });
        let refresh_slider = refresh_slider.read(cx).slider.clone();

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(13.0)).child("General Settings"))
            .child(
                GroupBox::new()
                    .fill()
                    .child(
                        // Theme Select
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.0)).child("Theme"))
                                    .child(div().text_size(px(10.0)).text_color(secondary_text).child("Configure app color palette"))
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .children(vec!["system", "dark", "light"].into_iter().enumerate().map(|(idx, t)| {
                                        let is_sel = self.config.general.theme == t;
                                        let theme_app = theme_app.clone();
                                        Button::new(("theme_btn", idx))
                                            .label(t)
                                            .small()
                                            .compact()
                                            .selected(is_sel)
                                            .when(is_sel, |button| button.primary())
                                            .on_click(move |_, window, cx| {
                                                theme_app.update(cx, |this, view_cx| {
                                                    this.config.general.theme = t.to_string();
                                                    this.save_config();
                                                    let dark = match t {
                                                        "dark" => true,
                                                        "light" => false,
                                                        _ => matches!(
                                                            window.appearance(),
                                                            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                                                        ),
                                                    };
                                                    crate::refresh_mica_backdrop(dark);
                                                    apply_component_theme(t, Some(window), view_cx);
                                                    view_cx.notify();
                                                }).ok();
                                            })
                                    }))
                            )
                    )
                    .child(Divider::horizontal())
                    .child(
                        // Start with Windows
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.0)).child("Start with Windows"))
                                    .child(div().text_size(px(10.0)).text_color(secondary_text).child("Launch Quotify when you sign in"))
                            )
                            .child(
                                Switch::new("start_with_windows")
                                    .checked(self.config.general.start_with_windows)
                                    .on_click(move |checked, _window, cx| {
                                        let checked = *checked;
                                        start_with_windows_app
                                            .update(cx, |this, cx| {
                                                if let Ok(()) = crate::startup::set_enabled(checked) {
                                                    this.config.general.start_with_windows = checked;
                                                    this.save_config();
                                                }
                                                cx.notify();
                                            })
                                            .ok();
                                    })
                            )
                    )
                    .child(Divider::horizontal())
                    .child(
                        // Refresh Interval
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.0)).child("Refresh Interval"))
                                    .child(div().text_size(px(11.0)).child(format!("{}s", self.config.general.refresh_interval)))
                            )
                            .child(
                                Slider::new(&refresh_slider)
                                    .horizontal()
                                    .w_full()
                                    .h(px(28.0))
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_size(px(9.0))
                                    .text_color(secondary_text)
                                    .child("30s")
                                    .child("3600s")
                            )
                    )
                    .child(Divider::horizontal())
                    .child(
                        // Network Proxy input field
                        self.render_input_field("proxy".into(), "Network Proxy".into(), "e.g. http://127.0.0.1:7890".into(), false, window, cx)
                    )
            )
            .into_any_element()
    }

    fn render_provider_settings(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let toggle_secrets_app = cx.entity().downgrade();
        let select_app = cx.entity().downgrade();

        let provider_names = provider_catalog()
            .iter()
            .map(|(_, display)| display.to_string())
            .collect::<Vec<_>>();
        let selected_index = provider_catalog()
            .iter()
            .position(|(id, _)| *id == self.selected_setting_provider)
            .unwrap_or(0);
        let provider_select =
            window.use_keyed_state("provider-select-state", cx, move |window, cx| {
                let select = cx.new(|cx| {
                    SelectState::new(
                        provider_names,
                        Some(IndexPath::default().row(selected_index)),
                        window,
                        cx,
                    )
                    .searchable(true)
                });
                let _subscription = cx.subscribe(
                    &select,
                    move |_, _, event: &SelectEvent<Vec<String>>, cx| {
                        let SelectEvent::Confirm(Some(display_name)) = event else {
                            return;
                        };
                        if let Some((provider_id, _)) = provider_catalog()
                            .iter()
                            .find(|(_, display)| *display == display_name)
                        {
                            let provider_id = provider_id.to_string();
                            select_app
                                .update(cx, |this, cx| {
                                    this.selected_setting_provider = provider_id;
                                    cx.notify();
                                })
                                .ok();
                        }
                    },
                );
                ProviderSelectFieldState {
                    select,
                    _subscription,
                }
            });
        let provider_select = provider_select.read(cx).select.clone();

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_size(px(13.0))
                            .child("Provider Settings"),
                    )
                    .child(
                        // Secrets masking toggler
                        Button::new("toggle_secrets_btn")
                            .ghost()
                            .xsmall()
                            .label(if self.show_secrets {
                                "Hide Secrets"
                            } else {
                                "Show Secrets"
                            })
                            .on_click(move |_, _, cx| {
                                toggle_secrets_app
                                    .update(cx, |this, cx| {
                                        this.show_secrets = !this.show_secrets;
                                        cx.notify();
                                    })
                                    .ok();
                            }),
                    ),
            )
            .child(
                GroupBox::new()
                    .fill()
                    .child(
                        // Provider ComboBox
                        Select::new(&provider_select)
                            .small()
                            .search_placeholder("Search providers")
                            .w_full(),
                    )
                    .child(Divider::horizontal())
                    .child(
                        // Provider fields list based on selection
                        self.render_selected_provider_fields(window, cx),
                    ),
            )
            .into_any_element()
    }

    fn render_settings_footer(&self) -> AnyElement {
        let config_path = self.config_path.clone();
        let report_config_path = self.config_path.clone();
        let report_history = self.history.clone();
        div()
            .flex()
            .gap_3()
            .justify_center()
            .child(
                Link::new("open_config_file_link")
                    .text_size(px(11.0))
                    .child("Open config file")
                    .on_click(move |_, _, _| {
                        let _ = open_config_file(config_path.as_ref());
                    }),
            )
            .child(Divider::vertical().h(px(12.0)))
            .child(
                Link::new("open_logs_link")
                    .text_size(px(11.0))
                    .child("Open logs")
                    .on_click(move |_, _, _| {
                        let _ = open_folder(&crate::diagnostics::log_dir());
                    }),
            )
            .child(Divider::vertical().h(px(12.0)))
            .child(
                Link::new("create_diagnostic_report_link")
                    .text_size(px(11.0))
                    .child("Create diagnostic report")
                    .on_click(move |_, _, _| {
                        let _ = crate::diagnostics::write_diagnostic_report(
                            report_config_path.as_deref(),
                            Some(&report_history.read()),
                        );
                    }),
            )
            .into_any_element()
    }

    fn render_input_field(
        &self,
        field_id: SharedString,
        label: SharedString,
        placeholder: SharedString,
        is_password: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let initial_value = config_field_value(&self.config, field_id.as_ref());
        let masked = is_password && !self.show_secrets;
        let app = cx.entity().downgrade();
        let subscription_field = field_id.to_string();
        let input_state = window.use_keyed_state(
            SharedString::from(format!("input-field-state-{field_id}")),
            cx,
            move |window, cx| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(initial_value)
                        .placeholder(placeholder)
                        .masked(masked)
                });
                let _subscription =
                    cx.subscribe(&input, move |_, input, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            let value = input.read(cx).value().to_string();
                            app.update(cx, |this, cx| {
                                if set_config_field_value(
                                    &mut this.config,
                                    &subscription_field,
                                    value.clone(),
                                ) {
                                    this.save_config();
                                }
                                cx.notify();
                            })
                            .ok();
                        }
                    });

                InputFieldState {
                    input,
                    masked,
                    _subscription,
                }
            },
        );

        let input = input_state.read(cx).input.clone();
        if input_state.read(cx).masked != masked {
            input_state.update(cx, |state, cx| {
                state.masked = masked;
                state
                    .input
                    .update(cx, |input, cx| input.set_masked(masked, window, cx));
            });
        }

        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_size(px(11.0))
                    .child(label),
            )
            .child(
                Input::new(&input)
                    .small()
                    .w_full()
                    .when(is_password, |input| input.mask_toggle()),
            )
            .into_any_element()
    }

    fn render_selected_provider_fields(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let provider_id = self.selected_setting_provider.clone();

        let mut widgets: Vec<AnyElement> = Vec::new();

        // 1. Enable toggle switch
        let enabled = match provider_id.as_str() {
            "deepseek" => self.config.deepseek.enabled.unwrap_or(false),
            "claude" => self.config.claude.enabled.unwrap_or(false),
            "codex" => self.config.codex.enabled.unwrap_or(false),
            "gemini" => self.config.gemini.enabled.unwrap_or(false),
            "antigravity" => self.config.antigravity.enabled.unwrap_or(false),
            "opencode" => self.config.opencode.enabled.unwrap_or(false),
            "mimo" => self.config.mimo.enabled.unwrap_or(false),
            _ => {
                if let Some(cfg) = api_key_provider_config(&self.config, &provider_id) {
                    cfg.enabled.unwrap_or(false)
                } else {
                    false
                }
            }
        };

        let provider_id_checkbox = provider_id.clone();
        let enable_provider_app = cx.entity().downgrade();
        widgets.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_size(px(11.0)).child("Enable Provider"))
                .child(
                    Switch::new("enable_provider_switch")
                        .checked(enabled)
                        .on_click(move |checked, _window, cx| {
                            let checked = *checked;
                            enable_provider_app
                                .update(cx, |this, cx| {
                                    match provider_id_checkbox.as_str() {
                                        "deepseek" => this.config.deepseek.enabled = Some(checked),
                                        "claude" => this.config.claude.enabled = Some(checked),
                                        "codex" => this.config.codex.enabled = Some(checked),
                                        "gemini" => this.config.gemini.enabled = Some(checked),
                                        "antigravity" => {
                                            this.config.antigravity.enabled = Some(checked)
                                        }
                                        "opencode" => this.config.opencode.enabled = Some(checked),
                                        "mimo" => this.config.mimo.enabled = Some(checked),
                                        _ => {
                                            if let Some(cfg) = api_key_provider_config_mut(
                                                &mut this.config,
                                                &provider_id_checkbox,
                                            ) {
                                                cfg.enabled = Some(checked);
                                            }
                                        }
                                    }
                                    this.save_config();
                                    cx.notify();
                                })
                                .ok();
                        }),
                )
                .into_any_element(),
        );

        // 2. Specific field editors
        match provider_id.as_str() {
            "deepseek" => {
                widgets.push(
                    self.render_input_field(
                        "deepseek_key".into(),
                        "API Key".into(),
                        "Paste DeepSeek Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            "claude" => {
                widgets.push(
                    self.render_input_field(
                        "claude_key".into(),
                        "API Key".into(),
                        "Claude Admin Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        "claude_session".into(),
                        "Session Key".into(),
                        "Claude Session Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        "claude_token".into(),
                        "Access Token".into(),
                        "Claude Access Token".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        "claude_auth".into(),
                        "Auth File Path".into(),
                        "e.g. C:\\Users\\Admin\\.claude\\session.toml".into(),
                        false,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            "codex" => {
                widgets.push(
                    self.render_input_field(
                        "codex_auth".into(),
                        "Auth File Path".into(),
                        "e.g. C:\\Users\\Admin\\.codex\\token.json".into(),
                        false,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            "gemini" => {
                widgets.push(
                    self.render_input_field(
                        "gemini_key".into(),
                        "API Key".into(),
                        "Paste Gemini Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            "antigravity" => {
                widgets.push(
                    self.render_input_field(
                        "antigravity_key".into(),
                        "API Key".into(),
                        "Paste Antigravity Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            "opencode" => {
                widgets.push(
                    self.render_input_field(
                        "opencode_key".into(),
                        "API Key".into(),
                        "OpenCode Workspaces Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        "opencode_workspace".into(),
                        "Workspace ID".into(),
                        "Paste Workspace ID".into(),
                        false,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        "opencode_auth".into(),
                        "Auth Cookie".into(),
                        "OpenCode Auth Cookie".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            "mimo" => {
                widgets.push(
                    self.render_input_field(
                        "mimo_key".into(),
                        "API Key".into(),
                        "MiMo Token Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        "mimo_token".into(),
                        "Service Token".into(),
                        "MiMo Service Token".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        "mimo_cookie".into(),
                        "Cookie Header".into(),
                        "MiMo Cookie Header".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            "opencodego" => {
                widgets.push(
                    div()
                        .text_size(px(11.0))
                        .text_color(gpui::rgba(0xffffff7f))
                        .child("OpenCode Go is configured using OpenCode settings.")
                        .into_any_element(),
                );
            }
            _ => {
                let key_field = SharedString::from(format!("{}_key", provider_id));
                let url_field = SharedString::from(format!("{}_url", provider_id));
                let dep_field = SharedString::from(format!("{}_dep", provider_id));
                widgets.push(
                    self.render_input_field(
                        key_field,
                        "API Key / Token".into(),
                        "Paste provider credential".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        url_field,
                        "Base URL".into(),
                        "e.g. http://127.0.0.1:8000/v1".into(),
                        false,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        dep_field,
                        "Deployment / Model Name".into(),
                        "e.g. gpui-4o-mini".into(),
                        false,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
        }

        // Test Controls & Status message block
        let status = self.provider_test_status.lock().clone();
        let testing_this = matches!(
            &status,
            ProviderTestStatus::Testing { provider } if *provider == provider_id
        );
        let testing_other = matches!(&status, ProviderTestStatus::Testing { .. }) && !testing_this;

        let provider_id_test = provider_id.clone();
        let test_provider_app = cx.entity().downgrade();
        widgets.push(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    Button::new("test_provider_btn")
                        .primary()
                        .small()
                        .w(px(110.0))
                        .label("Test Provider")
                        .loading(testing_this)
                        .disabled(testing_other)
                        .on_click(move |_, _, cx| {
                            if !testing_this && !testing_other {
                                test_provider_app
                                    .update(cx, |this, view_cx| {
                                        this.trigger_provider_test(
                                            provider_id_test.clone(),
                                            view_cx,
                                        );
                                    })
                                    .ok();
                            }
                        }),
                )
                .child(match status {
                    ProviderTestStatus::Success {
                        provider,
                        fetched_at,
                        summary,
                    } if provider == provider_id => Alert::success(
                        "provider-test-success",
                        format!(
                            "Test passed at {}. {summary}",
                            fetched_at.with_timezone(&chrono::Local).format("%H:%M:%S")
                        ),
                    )
                    .small()
                    .into_any_element(),
                    ProviderTestStatus::Error { provider, message } if provider == provider_id => {
                        Alert::error("provider-test-error", format!("Test failed: {message}"))
                            .small()
                            .into_any_element()
                    }
                    ProviderTestStatus::Testing { provider } if provider == provider_id => {
                        Alert::info(
                            "provider-test-running",
                            "Fetching usage with current provider settings...",
                        )
                        .small()
                        .into_any_element()
                    }
                    _ => div().into_any_element(),
                })
                .into_any_element(),
        );

        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(widgets)
            .into_any_element()
    }
}

fn provider_catalog() -> &'static [(&'static str, &'static str)] {
    &[
        ("codex", "Codex"),
        ("openai", "OpenAI"),
        ("opencode", "OpenCode"),
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

fn provider_icon(provider_name: &str, is_dark: bool) -> Option<&'static str> {
    match (provider_name, is_dark) {
        ("abacus", true) => Some("assets/provider-icons/abacus-ai-dark.svg"),
        ("abacus", false) => Some("assets/provider-icons/abacus-ai.png"),
        ("alibabatoken", _) => Some("assets/provider-icons/alibaba.svg"),
        ("amp", _) => Some("assets/provider-icons/amp.svg"),
        ("augment", _) => Some("assets/provider-icons/augment.svg"),
        ("codex", true) => Some("assets/provider-icons/codex-dark.svg"),
        ("codex", false) => Some("assets/provider-icons/codex.svg"),
        ("codebuff", true) => Some("assets/provider-icons/codebuff-dark.svg"),
        ("codebuff", false) => Some("assets/provider-icons/codebuff.svg"),
        ("copilot", _) => Some("assets/provider-icons/copilot.svg"),
        ("cursor", _) => Some("assets/provider-icons/cursor.svg"),
        ("droid", true) => Some("assets/provider-icons/droid-dark.svg"),
        ("droid", false) => Some("assets/provider-icons/droid.svg"),
        ("elevenlabs", _) => Some("assets/provider-icons/elevenlabs.svg"),
        ("jetbrains", _) => Some("assets/provider-icons/jetbrains-ai.svg"),
        ("kilo", _) => Some("assets/provider-icons/kilo.svg"),
        ("kimi", _) => Some("assets/provider-icons/kimi.svg"),
        ("kiro", true) => Some("assets/provider-icons/kiro-dark.svg"),
        ("kiro", false) => Some("assets/provider-icons/kiro.svg"),
        ("minimax", _) => Some("assets/provider-icons/minimax.svg"),
        ("mistral", _) => Some("assets/provider-icons/mistral.svg"),
        ("ollama", _) => Some("assets/provider-icons/ollama.svg"),
        ("opencode" | "opencodego", true) => Some("assets/provider-icons/opencode-dark.svg"),
        ("opencode" | "opencodego", false) => Some("assets/provider-icons/opencode.svg"),
        ("openrouter", _) => Some("assets/provider-icons/openrouter.svg"),
        ("claude", _) => Some("assets/provider-icons/claude.svg"),
        ("gemini", _) => Some("assets/provider-icons/gemini.svg"),
        ("antigravity", _) => Some("assets/provider-icons/antigravity.svg"),
        ("deepseek", _) => Some("assets/provider-icons/deepseek.svg"),
        ("synthetic", true) => Some("assets/provider-icons/synthetic-dark.svg"),
        ("synthetic", false) => Some("assets/provider-icons/synthetic.svg"),
        ("vertexai", _) => Some("assets/provider-icons/vertex-ai.svg"),
        ("warp", _) => Some("assets/provider-icons/warp.svg"),
        ("zai", true) => Some("assets/provider-icons/zai-dark.svg"),
        ("zai", false) => Some("assets/provider-icons/zai.svg"),
        ("mimo", _) => Some("assets/provider-icons/mimo.svg"),
        _ => None,
    }
}

fn config_field_value(config: &crate::config::AppConfig, field: &str) -> String {
    match field {
        "proxy" => return config.network.proxy.clone(),
        "deepseek_key" => return config.deepseek.api_key.clone(),
        "claude_key" => return config.claude.api_key.clone(),
        "claude_session" => return config.claude.session_key.clone(),
        "claude_token" => return config.claude.access_token.clone(),
        "claude_auth" => return config.claude.auth_file.clone(),
        "codex_auth" => return config.codex.auth_file.clone(),
        "gemini_key" => return config.gemini.api_key.clone(),
        "antigravity_key" => return config.antigravity.api_key.clone(),
        "opencode_key" => return config.opencode.api_key.clone(),
        "opencode_workspace" => return config.opencode.workspace_id.clone(),
        "opencode_auth" => return config.opencode.auth_cookie.clone(),
        "mimo_key" => return config.mimo.api_key.clone(),
        "mimo_token" => return config.mimo.service_token.clone(),
        "mimo_cookie" => return config.mimo.cookie_header.clone(),
        _ => {}
    }

    if let Some(provider) = field.strip_suffix("_key") {
        if let Some(config) = api_key_provider_config(config, provider) {
            return config.api_key.clone();
        }
    }
    if let Some(provider) = field.strip_suffix("_url") {
        if let Some(config) = api_key_provider_config(config, provider) {
            return config.base_url.clone();
        }
    }
    if let Some(provider) = field.strip_suffix("_dep") {
        if let Some(config) = api_key_provider_config(config, provider) {
            return config.deployment.clone();
        }
    }

    String::new()
}

fn set_config_field_value(
    config: &mut crate::config::AppConfig,
    field: &str,
    value: String,
) -> bool {
    let target = match field {
        "proxy" => Some(&mut config.network.proxy),
        "deepseek_key" => Some(&mut config.deepseek.api_key),
        "claude_key" => Some(&mut config.claude.api_key),
        "claude_session" => Some(&mut config.claude.session_key),
        "claude_token" => Some(&mut config.claude.access_token),
        "claude_auth" => Some(&mut config.claude.auth_file),
        "codex_auth" => Some(&mut config.codex.auth_file),
        "gemini_key" => Some(&mut config.gemini.api_key),
        "antigravity_key" => Some(&mut config.antigravity.api_key),
        "opencode_key" => Some(&mut config.opencode.api_key),
        "opencode_workspace" => Some(&mut config.opencode.workspace_id),
        "opencode_auth" => Some(&mut config.opencode.auth_cookie),
        "mimo_key" => Some(&mut config.mimo.api_key),
        "mimo_token" => Some(&mut config.mimo.service_token),
        "mimo_cookie" => Some(&mut config.mimo.cookie_header),
        _ => None,
    };

    if let Some(target) = target {
        *target = value;
        return true;
    }

    for suffix in ["_key", "_url", "_dep"] {
        if let Some(provider) = field.strip_suffix(suffix) {
            if let Some(provider_config) = api_key_provider_config_mut(config, provider) {
                match suffix {
                    "_key" => provider_config.api_key = value,
                    "_url" => provider_config.base_url = value,
                    "_dep" => provider_config.deployment = value,
                    _ => unreachable!(),
                }
                return true;
            }
        }
    }

    false
}

fn api_key_provider_config<'a>(
    config: &'a crate::config::AppConfig,
    provider: &str,
) -> Option<&'a crate::config::ApiKeyProviderConfig> {
    match provider {
        "openai" => Some(&config.openai),
        "openrouter" => Some(&config.openrouter),
        "moonshot" => Some(&config.moonshot),
        "elevenlabs" => Some(&config.elevenlabs),
        "doubao" => Some(&config.doubao),
        "zai" => Some(&config.zai),
        "venice" => Some(&config.venice),
        "crof" => Some(&config.crof),
        "synthetic" => Some(&config.synthetic),
        "warp" => Some(&config.warp),
        "groqcloud" => Some(&config.groqcloud),
        "deepgram" => Some(&config.deepgram),
        "llmproxy" => Some(&config.llmproxy),
        "codebuff" => Some(&config.codebuff),
        "kiro" => Some(&config.kiro),
        "copilot" => Some(&config.copilot),
        "azureopenai" => Some(&config.azureopenai),
        "ollama" => Some(&config.ollama),
        "minimax" => Some(&config.minimax),
        "jetbrains" => Some(&config.jetbrains),
        "kimi" => Some(&config.kimi),
        "kilo" => Some(&config.kilo),
        "augment" => Some(&config.augment),
        "bedrock" => Some(&config.bedrock),
        "vertexai" => Some(&config.vertexai),
        "stepfun" => Some(&config.stepfun),
        "abacus" => Some(&config.abacus),
        "alibabatoken" => Some(&config.alibabatoken),
        "t3chat" => Some(&config.t3chat),
        "amp" => Some(&config.amp),
        "mistral" => Some(&config.mistral),
        "grok" => Some(&config.grok),
        "cursor" => Some(&config.cursor),
        "droid" => Some(&config.droid),
        "windsurf" => Some(&config.windsurf),
        _ => None,
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

fn format_credits_balance(balance: f64) -> String {
    if balance.fract() == 0.0 {
        format!("{:.0}", balance)
    } else {
        format!("{:.2}", balance)
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

use crate::version::is_newer;

fn open_config_file(config_path: Option<&PathBuf>) -> anyhow::Result<()> {
    let path = if let Some(p) = config_path {
        p.clone()
    } else {
        crate::config::AppConfig::config_path()
    };
    std::process::Command::new("cmd")
        .args(["/C", "start", "", "notepad", &path.to_string_lossy()])
        .spawn()?;
    Ok(())
}

fn open_folder(path: &std::path::Path) -> anyhow::Result<()> {
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &path.to_string_lossy()])
        .spawn()?;
    Ok(())
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
