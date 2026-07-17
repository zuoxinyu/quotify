use gpui::prelude::{FluentBuilder, InteractiveElement, ParentElement, Styled};
use gpui::*;
use parking_lot::RwLock;
use std::{
    path::PathBuf,
    sync::Arc,
    sync::atomic::Ordering,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderStatus {
    Active,
    Error,
    ErrorWithCache,
    Disabled,
}

pub struct QuotifyApp {
    pub data: Arc<RwLock<Vec<UsageData>>>,
    pub last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    pub history: Arc<RwLock<crate::usage_history::UsageHistory>>,
    pub config: crate::config::AppConfig,
    pub config_path: Option<PathBuf>,
    pub active_provider: Arc<RwLock<String>>,
    pub drag: ProviderDragState,
    pub last_config_reload: Instant,
    pub update_status: Arc<parking_lot::Mutex<UpdateStatus>>,
    pub provider_test_status: Arc<parking_lot::Mutex<ProviderTestStatus>>,
    pub selected_setting_provider: String,
    pub show_secrets: bool,
    pub focus_handles: std::collections::HashMap<String, FocusHandle>,
    pub show_provider_dropdown: bool,
}

impl QuotifyApp {
    pub fn new(
        data: Arc<RwLock<Vec<UsageData>>>,
        last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
        config: crate::config::AppConfig,
        config_path: Option<PathBuf>,
        active_provider: Arc<RwLock<String>>,
        history: Arc<RwLock<crate::usage_history::UsageHistory>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut focus_handles = std::collections::HashMap::new();
        // Pre-create focus handles for form inputs
        let fields = vec![
            "proxy",
            "openai_key",
            "openai_url",
            "openai_dep",
            "deepseek_key",
            "claude_key",
            "claude_session",
            "claude_token",
            "claude_auth",
            "codex_auth",
            "gemini_key",
            "antigravity_key",
            "opencode_key",
            "opencode_workspace",
            "opencode_auth",
            "mimo_key",
            "mimo_token",
            "mimo_cookie",
        ];
        for f in fields {
            focus_handles.insert(f.to_string(), cx.focus_handle());
        }

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
            focus_handles,
            show_provider_dropdown: false,
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

    // Handles key input into settings fields manually to support input replication 1:1
    fn handle_input_key(&mut self, field: &str, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let val = match field {
            "proxy" => &mut self.config.network.proxy,
            "openai_key" => &mut self.config.openai.api_key,
            "openai_url" => &mut self.config.openai.base_url,
            "openai_dep" => &mut self.config.openai.deployment,
            "deepseek_key" => &mut self.config.deepseek.api_key,
            "claude_key" => &mut self.config.claude.api_key,
            "claude_session" => &mut self.config.claude.session_key,
            "claude_token" => &mut self.config.claude.access_token,
            "claude_auth" => &mut self.config.claude.auth_file,
            "codex_auth" => &mut self.config.codex.auth_file,
            "gemini_key" => &mut self.config.gemini.api_key,
            "antigravity_key" => &mut self.config.antigravity.api_key,
            "opencode_key" => &mut self.config.opencode.api_key,
            "opencode_workspace" => &mut self.config.opencode.workspace_id,
            "opencode_auth" => &mut self.config.opencode.auth_cookie,
            "mimo_key" => &mut self.config.mimo.api_key,
            "mimo_token" => &mut self.config.mimo.service_token,
            "mimo_cookie" => &mut self.config.mimo.cookie_header,
            _ => return,
        };

        if event.keystroke.key == "backspace" {
            val.pop();
        } else if event.keystroke.modifiers.control && event.keystroke.key == "v" {
            if let Some(item) = cx.read_from_clipboard() {
                if let Some(text) = item.text() {
                    val.push_str(&text);
                }
            }
        } else if let Some(c) = &event.keystroke.key_char {
            val.push_str(c);
        }

        self.save_config();
        cx.notify();
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
            .font_family("Segoe UI Variable")
            .child(
                // Header block
                self.render_header(active_page, is_dark, cx),
            )
            .child(
                // Line separator
                div().h(px(1.0)).mt(px(4.0)).bg(border_color).w_full(),
            )
            .child(
                // Body View
                div()
                    .flex_1()
                    .w_full()
                    .px(px(12.0))
                    .py(px(10.0))
                    .child(match active_page {
                        1 => self.render_about(is_dark, cx).into_any_element(),
                        2 => self.render_settings(is_dark, window, cx).into_any_element(),
                        _ => self.render_dashboard(is_dark, cx).into_any_element(),
                    })
                    .id("body_view")
                    .overflow_y_scroll()
                    .scrollbar_width(px(0.0)),
            )
    }
}

impl QuotifyApp {
    fn winui_toggle_switch(enabled: bool, is_dark: bool) -> Div {
        let track = if enabled {
            gpui::rgb(0x0067c0)
        } else if is_dark {
            gpui::rgba(0xffffff33)
        } else {
            gpui::rgba(0x00000033)
        };
        let border = if enabled {
            gpui::rgb(0x0067c0)
        } else if is_dark {
            gpui::rgba(0xffffff66)
        } else {
            gpui::rgba(0x00000066)
        };

        div()
            .flex()
            .items_center()
            .when(enabled, |switch| switch.justify_end())
            .w(px(40.0))
            .h(px(20.0))
            .p(px(2.0))
            .bg(track)
            .border(px(1.0))
            .border_color(border)
            .rounded(px(10.0))
            .cursor(CursorStyle::PointingHand)
            .child(
                div()
                    .w(px(14.0))
                    .h(px(14.0))
                    .rounded(px(7.0))
                    .bg(if enabled {
                        gpui::rgb(0xffffff)
                    } else if is_dark {
                        gpui::rgb(0xd6d6d6)
                    } else {
                        gpui::rgb(0x5c5c5c)
                    })
                    .shadow_sm(),
            )
    }

    fn render_header(&self, active_page: u32, is_dark: bool, cx: &mut Context<Self>) -> AnyElement {
        let hover_bg = if is_dark {
            gpui::rgba(0xffffff1a)
        } else {
            gpui::rgba(0x0000000d)
        };
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
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(px(4.0))
                            .hover(|style| style.bg(hover_bg))
                            .font_family("Segoe MDL2 Assets")
                            .font_weight(gpui::FontWeight::THIN)
                            .text_size(px(12.0))
                            .child("\u{E72B}")
                            .id("back_btn")
                            .on_click(cx.listener(
                                |_this: &mut Self,
                                 _event: &gpui::ClickEvent,
                                 _window: &mut gpui::Window,
                                 cx: &mut gpui::Context<Self>| {
                                    crate::tray::ACTIVE_PAGE.store(0, Ordering::SeqCst);
                                    cx.notify();
                                },
                            ))
                    } else {
                        // App Logo
                        div()
                            .w(px(18.0))
                            .h(px(18.0))
                            .child(img("assets/icons/quotify.svg").w_full().h_full())
                            .id("app_logo")
                            .on_click(cx.listener(
                                |_this: &mut Self,
                                 _event: &gpui::ClickEvent,
                                 _window: &mut gpui::Window,
                                 cx: &mut gpui::Context<Self>| {
                                    crate::tray::ACTIVE_PAGE.store(1, Ordering::SeqCst);
                                    cx.notify();
                                },
                            ))
                    })
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_size(px(14.5))
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
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(px(4.0))
                            .hover(|style| style.bg(hover_bg))
                            .font_family("Segoe MDL2 Assets")
                            .font_weight(gpui::FontWeight::THIN)
                            .text_size(px(12.0))
                            .child("\u{E72C}")
                            .id("refresh_btn")
                            .on_click(move |_, _, _| {
                                crate::tray::request_refresh();
                            }),
                    )
                    .child(
                        // Settings button
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(px(4.0))
                            .hover(|style| style.bg(hover_bg))
                            .font_family("Segoe MDL2 Assets")
                            .font_weight(gpui::FontWeight::THIN)
                            .text_size(px(12.0))
                            .child("\u{E713}")
                            .id("settings_btn")
                            .on_click(cx.listener(
                                |_this: &mut Self,
                                 _event: &gpui::ClickEvent,
                                 _window: &mut gpui::Window,
                                 cx: &mut gpui::Context<Self>| {
                                    crate::tray::ACTIVE_PAGE.store(2, Ordering::SeqCst);
                                    cx.notify();
                                },
                            )),
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
        let card_bg = if is_dark {
            gpui::rgba(0x2d2d2dc8)
        } else {
            gpui::rgba(0xffffffb2)
        };

        let provider_name = name.to_string();
        let card_id = provider_name.clone();
        let card_elt_id = SharedString::from(format!("card_{name}"));

        let trend = self.history.read().trend_for(name, 7);

        div()
            .flex()
            .flex_col()
            .w_full()
            .bg(card_bg)
            .rounded(px(8.0))
            .px(px(10.0)) // Matches egui Margin::symmetric(10, 8)
            .py(px(8.0))
            .child(self.render_card_header(name, display_name, data, is_dark))
            .child(div().h(px(12.0))) // Faint vertical margin instead of line separator
            .child(self.render_card_body(data, trend, is_dark))
            .id(card_elt_id)
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
                            .font_weight(gpui::FontWeight::LIGHT)
                            .text_size(px(12.0))
                            .child(display_name),
                    )
                    .child(if is_primary {
                        div()
                            .px_1()
                            .rounded(px(4.0))
                            .bg(if is_dark {
                                gpui::rgb(0x222b42)
                            } else {
                                gpui::rgb(0xe5f2ff)
                            })
                            .border(px(1.0))
                            .border_color(if is_dark {
                                gpui::rgb(0x76b9ed)
                            } else {
                                gpui::rgb(0x0078d4)
                            })
                            .text_color(if is_dark {
                                gpui::rgb(0x76b9ed)
                            } else {
                                gpui::rgb(0x005ba1)
                            })
                            .text_size(px(8.0))
                            .font_weight(gpui::FontWeight::NORMAL)
                            .child("PRIMARY")
                    } else {
                        div()
                    }),
            )
            .child(if let Some(ref credits) = data.credits {
                let text = format!(
                    "{} {}",
                    format_credits_balance(credits.balance),
                    credits.currency
                );
                let border_color = if is_dark {
                    gpui::rgb(0x60cdff)
                } else {
                    gpui::rgb(0x0078d4)
                };
                div()
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.0))
                    .border(px(1.0))
                    .border_color(border_color)
                    .bg(if is_dark {
                        gpui::rgb(0x1c2e3c)
                    } else {
                        gpui::rgb(0xe0f4ff)
                    })
                    .text_color(if is_dark {
                        gpui::rgb(0x60cdff)
                    } else {
                        gpui::rgb(0x0078d4)
                    })
                    .text_size(px(10.0))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .child(text)
            } else {
                div()
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
            return div()
                .flex()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .bg(if is_dark {
                    gpui::rgb(0x3d2626)
                } else {
                    gpui::rgb(0xfde8e8)
                })
                .text_color(if is_dark {
                    gpui::rgb(0xff9999)
                } else {
                    gpui::rgb(0x991b1b)
                })
                .text_size(px(11.0))
                .child("⚠")
                .child(error.clone())
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
            .gap(px(8.0)) // 8px gap between progress rows matches egui's default spacing
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
                    .w(px(88.0))
                    .font_family("Segoe UI Variable")
                    .font_weight(gpui::FontWeight::LIGHT)
                    .text_size(px(10.0))
                    .child(w.label.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(8.0))
                    .bg(if is_dark {
                        gpui::rgb(0x202020)
                    } else {
                        gpui::rgb(0xe5e5e5)
                    })
                    .rounded(px(4.0))
                    .child(
                        div()
                            .w(gpui::relative(pct as f32 / 100.0))
                            .h_full()
                            .bg(fill_color)
                            .rounded(px(4.0)),
                    ),
            )
            .child(
                div()
                    .w(px(34.0))
                    .text_size(px(10.0))
                    .text_color(fill_color)
                    .font_family("Segoe UI Variable")
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

    fn render_about(&self, is_dark: bool, cx: &mut Context<Self>) -> AnyElement {
        let ver = env!("GIT_TAG");
        let card_bg = if is_dark {
            gpui::rgba(0x2d2d2dc8)
        } else {
            gpui::rgba(0xffffffb2)
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .px(px(16.0)) // Matches egui symmetric(16, 12) settings/about frame
            .py(px(12.0))
            .bg(card_bg)
            .rounded(px(8.0))
            .child(self.render_about_header(ver, is_dark))
            .child(div().h(px(1.0)).bg(if is_dark {
                gpui::rgba(0xffffff14)
            } else {
                gpui::rgba(0x00000014)
            }))
            .child(self.render_update_section(cx))
            .into_any_element()
    }

    fn render_about_header(&self, ver: &str, is_dark: bool) -> AnyElement {
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
                    div()
                        .text_color(gpui::rgb(0x0078d4))
                        .child("zuoxinyu/quotify")
                        .id("github_link")
                        .on_click(move |_, _, _| {
                            open_browser("https://github.com/zuoxinyu/quotify");
                        }),
                ),
            )
            .into_any_element()
    }

    fn render_update_section(&self, cx: &mut Context<Self>) -> AnyElement {
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
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(130.0))
                    .h(px(28.0))
                    .bg(gpui::rgb(0x005fa8))
                    .text_color(gpui::rgb(0xffffff))
                    .text_size(px(11.0))
                    .rounded(px(4.0))
                    .child("Check now")
                    .id("check_updates_btn")
                    .on_click(cx.listener(
                        |this: &mut Self,
                         _event: &gpui::ClickEvent,
                         _window: &mut gpui::Window,
                         cx: &mut gpui::Context<Self>| {
                            this.trigger_check_update(cx);
                        },
                    )),
            )
            .child(match self.update_status.lock().clone() {
                UpdateStatus::Checking => div().text_size(px(11.0)).child("Checking..."),
                UpdateStatus::UpToDate { .. } => div()
                    .text_size(px(11.0))
                    .text_color(gpui::rgb(0x2e7d32))
                    .child("App is up to date."),
                UpdateStatus::NewVersionAvailable {
                    latest_version,
                    release_url,
                } => div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(gpui::rgb(0xc62828))
                            .child(format!("New version {latest_version} available!")),
                    )
                    .child(
                        div()
                            .text_color(gpui::rgb(0x0078d4))
                            .text_size(px(11.0))
                            .child("View Release Page")
                            .id("view_release_page_link")
                            .on_click(move |_, _, _| {
                                open_browser(&release_url);
                            }),
                    ),
                UpdateStatus::Error(err) => div()
                    .text_size(px(11.0))
                    .text_color(gpui::rgb(0xc62828))
                    .child(format!("Update check failed: {err}")),
                _ => div(),
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
            .child(self.render_provider_settings(is_dark, window, cx))
            .child(self.render_settings_footer(is_dark, cx))
            .child(div().h(px(20.0)))
            .into_any_element()
    }

    fn render_general_settings(
        &self,
        is_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let card_bg = if is_dark {
            gpui::rgba(0x2d2d2dc8)
        } else {
            gpui::rgba(0xffffffb2)
        };
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
        let refresh_progress = refresh_index as f32 / (refresh_intervals.len() - 1) as f32;
        let slider_track = if is_dark {
            gpui::rgba(0xffffff4d)
        } else {
            gpui::rgba(0x00000042)
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(13.0)).child("General Settings"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .px(px(16.0)) // Matches egui Margin::symmetric(16, 12) settings card frame
                    .py(px(12.0))
                    .bg(card_bg)
                    .rounded(px(8.0))
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
                                        div()
                                            .px_2()
                                            .py_1()
                                            .rounded(px(4.0))
                                            .bg(if is_sel { gpui::rgb(0x0078d4) } else { gpui::rgba(0x0000001a) })
                                            .text_color(if is_sel { gpui::rgb(0xffffff) } else { if is_dark { gpui::rgb(0xffffff) } else { gpui::rgb(0x000000) } })
                                            .text_size(px(10.0))
                                            .child(t)
                                            .id(("theme_btn", idx))
                                            .on_click(cx.listener(move |this: &mut Self, _event: &gpui::ClickEvent, window: &mut gpui::Window, cx: &mut gpui::Context<Self>| {
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
                                                cx.notify();
                                            }))
                                    }))
                            )
                    )
                    .child(
                        div().h(px(1.0)).bg(if is_dark { gpui::rgba(0xffffff14) } else { gpui::rgba(0x00000014) })
                    )
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
                                Self::winui_toggle_switch(
                                    self.config.general.start_with_windows,
                                    is_dark,
                                )
                                .id("start_with_windows")
                                .on_click(cx.listener(|this: &mut Self, _event: &gpui::ClickEvent, _window: &mut gpui::Window, cx: &mut gpui::Context<Self>| {
                                    let n = !this.config.general.start_with_windows;
                                    if let Ok(()) = crate::startup::set_enabled(n) {
                                        this.config.general.start_with_windows = n;
                                        this.save_config();
                                    }
                                    cx.notify();
                                }))
                            )
                    )
                    .child(
                        div().h(px(1.0)).bg(if is_dark { gpui::rgba(0xffffff14) } else { gpui::rgba(0x00000014) })
                    )
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
                                div()
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .w_full()
                                    .h(px(28.0))
                                    .cursor(CursorStyle::PointingHand)
                                    .child(
                                        div()
                                            .absolute()
                                            .left(relative(0.1))
                                            .top(px(12.0))
                                            .w(relative(0.8))
                                            .h(px(4.0))
                                            .rounded(px(2.0))
                                            .bg(slider_track),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .left(relative(0.1))
                                            .top(px(12.0))
                                            .w(relative(refresh_progress * 0.8))
                                            .h(px(4.0))
                                            .rounded(px(2.0))
                                            .bg(gpui::rgb(0x0067c0)),
                                    )
                                    .children(refresh_intervals.into_iter().enumerate().map(|(idx, val)| {
                                        let selected = idx == refresh_index;
                                        div()
                                            .flex()
                                            .flex_1()
                                            .h_full()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                div()
                                                    .w(px(if selected { 16.0 } else { 6.0 }))
                                                    .h(px(if selected { 16.0 } else { 6.0 }))
                                                    .rounded(px(if selected { 8.0 } else { 3.0 }))
                                                    .border(px(if selected { 3.0 } else { 0.0 }))
                                                    .border_color(if selected {
                                                        if is_dark {
                                                            gpui::rgb(0x202020)
                                                        } else {
                                                            gpui::rgb(0xffffff)
                                                        }
                                                    } else {
                                                        slider_track
                                                    })
                                                    .bg(if selected {
                                                        gpui::rgb(0x0067c0)
                                                    } else {
                                                        slider_track
                                                    }),
                                            )
                                            .id(("interval_slider_stop", idx))
                                            .on_click(cx.listener(move |this: &mut Self, _event: &gpui::ClickEvent, _window: &mut gpui::Window, cx: &mut gpui::Context<Self>| {
                                                this.config.general.refresh_interval = val;
                                                this.save_config();
                                                cx.notify();
                                            }))
                                            .on_mouse_move(cx.listener(move |this: &mut Self, event: &gpui::MouseMoveEvent, _window: &mut gpui::Window, cx: &mut gpui::Context<Self>| {
                                                if event.pressed_button == Some(MouseButton::Left)
                                                    && this.config.general.refresh_interval != val
                                                {
                                                    this.config.general.refresh_interval = val;
                                                    this.save_config();
                                                    cx.notify();
                                                }
                                            }))
                                    }))
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
                    .child(
                        div().h(px(1.0)).bg(if is_dark { gpui::rgba(0xffffff14) } else { gpui::rgba(0x00000014) })
                    )
                    .child(
                        // Network Proxy input field
                        self.render_input_field("proxy".into(), "Network Proxy".into(), "e.g. http://127.0.0.1:7890".into(), false, window, cx)
                    )
            )
            .into_any_element()
    }

    fn render_provider_settings(
        &self,
        is_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let card_bg = if is_dark {
            gpui::rgba(0x2d2d2dc8)
        } else {
            gpui::rgba(0xffffffb2)
        };

        let dropdown_bg = if is_dark {
            gpui::rgb(0x2b2b2b)
        } else {
            gpui::rgb(0xffffff)
        };
        let dropdown_border = if is_dark {
            gpui::rgb(0x5c5c5c)
        } else {
            gpui::rgb(0x8a8a8a)
        };
        let dropdown_hover = if is_dark {
            gpui::rgb(0x3a3a3a)
        } else {
            gpui::rgb(0xf0f0f0)
        };
        let dropdown_selected = if is_dark {
            gpui::rgb(0x404b57)
        } else {
            gpui::rgb(0xe5f1fb)
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(13.0)).child("Provider Settings"))
                    .child(
                        // Secrets masking toggler
                        div()
                            .text_size(px(11.0))
                            .text_color(gpui::rgb(0x0078d4))
                            .child(if self.show_secrets { "Hide Secrets" } else { "Show Secrets" })
                            .id("toggle_secrets_btn")
                            .on_click(cx.listener(|this: &mut Self, _event: &gpui::ClickEvent, _window: &mut gpui::Window, cx: &mut gpui::Context<Self>| {
                                this.show_secrets = !this.show_secrets;
                                cx.notify();
                            }))
                    )
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .px(px(16.0)) // Matches egui Margin::symmetric(16, 12) settings card frame
                    .py(px(12.0))
                    .bg(card_bg)
                    .rounded(px(8.0))
                    .child(
                        // Provider ComboBox
                        div()
                            .relative()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .w_full()
                                    .h(px(32.0))
                                    .px_3()
                                    .bg(dropdown_bg)
                                    .border(px(1.0))
                                    .border_color(dropdown_border)
                                    .rounded(px(4.0))
                                    .text_size(px(11.0))
                                    .cursor(CursorStyle::PointingHand)
                                    .child(
                                        provider_catalog()
                                            .iter()
                                            .find(|(id, _)| *id == self.selected_setting_provider)
                                            .map(|(_, name)| *name)
                                            .unwrap_or(self.selected_setting_provider.as_str())
                                            .to_string()
                                    )
                                    .child(div().text_size(px(13.0)).child("⌄"))
                                    .id("provider_select_btn")
                                    .on_click(cx.listener(|this: &mut Self, _event: &gpui::ClickEvent, _window: &mut gpui::Window, cx: &mut gpui::Context<Self>| {
                                        this.show_provider_dropdown = !this.show_provider_dropdown;
                                        cx.notify();
                                    }))
                            )
                            .child(
                                if self.show_provider_dropdown {
                                    deferred(
                                        div()
                                            .absolute()
                                            .left_0()
                                        .top(px(36.0))
                                        .w_full()
                                        .h(px(156.0))
                                        .bg(dropdown_bg)
                                        .border(px(1.0))
                                        .border_color(dropdown_border)
                                        .rounded(px(6.0))
                                        .shadow_lg()
                                        .occlude()
                                        .children(
                                            provider_catalog().iter().enumerate().map(|(idx, (id, display))| {
                                                let pid = id.to_string();
                                                let selected = *id == self.selected_setting_provider;
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .h(px(28.0))
                                                    .px_3()
                                                    .bg(if selected { dropdown_selected } else { dropdown_bg })
                                                    .hover(move |style| style.bg(dropdown_hover))
                                                    .text_size(px(11.0))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .child(*display)
                                                    .child(if selected { "✓" } else { "" })
                                                    .id(("dropdown_item", idx))
                                                    .on_click(cx.listener(move |this: &mut Self, _event: &gpui::ClickEvent, _window: &mut gpui::Window, cx: &mut gpui::Context<Self>| {
                                                        this.selected_setting_provider = pid.clone();
                                                        this.show_provider_dropdown = false;
                                                        cx.notify();
                                                    }))
                                            })
                                        )
                                        .id("provider_dropdown_list")
                                            .overflow_y_scroll(),
                                    )
                                    .with_priority(100)
                                    .into_any_element()
                                } else {
                                    div()
                                        .id("provider_dropdown_placeholder")
                                        .into_any_element()
                                }
                            )
                    )
                    .child(
                        div().h(px(1.0)).bg(if is_dark { gpui::rgba(0xffffff14) } else { gpui::rgba(0x00000014) })
                    )
                    .child(
                        // Provider fields list based on selection
                        self.render_selected_provider_fields(window, cx)
                    )
            )
            .into_any_element()
    }

    fn render_settings_footer(&self, is_dark: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .gap_3()
            .justify_center()
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(gpui::rgb(0x0078d4))
                    .child("Open config file")
                    .id("open_config_file_link")
                    .on_click(cx.listener(
                        |this: &mut Self,
                         _event: &gpui::ClickEvent,
                         _window: &mut gpui::Window,
                         _| {
                            let _ = open_config_file(this.config_path.as_ref());
                        },
                    )),
            )
            .child(div().w(px(1.0)).h(px(12.0)).bg(if is_dark {
                gpui::rgba(0xffffff33)
            } else {
                gpui::rgba(0x00000033)
            }))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(gpui::rgb(0x0078d4))
                    .child("Open logs")
                    .id("open_logs_link")
                    .on_click(move |_, _, _| {
                        let _ = open_folder(&crate::diagnostics::log_dir());
                    }),
            )
            .child(div().w(px(1.0)).h(px(12.0)).bg(if is_dark {
                gpui::rgba(0xffffff33)
            } else {
                gpui::rgba(0x00000033)
            }))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(gpui::rgb(0x0078d4))
                    .child("Create diagnostic report")
                    .id("create_diagnostic_report_link")
                    .on_click(cx.listener(
                        |this: &mut Self,
                         _event: &gpui::ClickEvent,
                         _window: &mut gpui::Window,
                         _| {
                            let _ = crate::diagnostics::write_diagnostic_report(
                                this.config_path.as_deref(),
                                Some(&this.history.read()),
                            );
                        },
                    )),
            )
            .into_any_element()
    }

    fn render_input_field(
        &self,
        field_id: SharedString,
        label: SharedString,
        placeholder: SharedString,
        is_password: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_dark = match self.config.general.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                cx.window_appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };

        let focus_handle = self.focus_handles.get(field_id.as_ref()).cloned().unwrap();
        let is_focused = focus_handle.is_focused(window);

        let input_bg = if is_dark {
            gpui::rgb(0x202020)
        } else {
            gpui::rgb(0xf9f9f9)
        };
        let input_border = if is_focused {
            gpui::rgb(0x0067c0)
        } else if is_dark {
            gpui::rgb(0x5c5c5c)
        } else {
            gpui::rgb(0x8a8a8a)
        };
        let placeholder_color = if is_dark {
            gpui::rgba(0xffffff66)
        } else {
            gpui::rgba(0x00000066)
        };

        let val = match field_id.as_ref() {
            "proxy" => &self.config.network.proxy,
            "openai_key" => &self.config.openai.api_key,
            "openai_url" => &self.config.openai.base_url,
            "openai_dep" => &self.config.openai.deployment,
            "deepseek_key" => &self.config.deepseek.api_key,
            "claude_key" => &self.config.claude.api_key,
            "claude_session" => &self.config.claude.session_key,
            "claude_token" => &self.config.claude.access_token,
            "claude_auth" => &self.config.claude.auth_file,
            "codex_auth" => &self.config.codex.auth_file,
            "gemini_key" => &self.config.gemini.api_key,
            "antigravity_key" => &self.config.antigravity.api_key,
            "opencode_key" => &self.config.opencode.api_key,
            "opencode_workspace" => &self.config.opencode.workspace_id,
            "opencode_auth" => &self.config.opencode.auth_cookie,
            "mimo_key" => &self.config.mimo.api_key,
            "mimo_token" => &self.config.mimo.service_token,
            "mimo_cookie" => &self.config.mimo.cookie_header,
            _ => "",
        };

        let display_text = if is_password && !self.show_secrets {
            "•".repeat(val.len())
        } else {
            val.to_string()
        };

        let has_text = !display_text.is_empty();
        let caret = |visible: bool| {
            if visible {
                div()
                    .w(px(1.0))
                    .h(px(15.0))
                    .mx(px(1.0))
                    .bg(gpui::rgb(0x0067c0))
            } else {
                div().w(px(0.0)).h(px(15.0))
            }
        };
        let field_id_str = field_id.to_string();

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
                div()
                    .flex()
                    .items_center()
                    .w_full()
                    .h(px(32.0))
                    .px_2()
                    .bg(input_bg)
                    .border(px(1.0))
                    .border_color(input_border)
                    .rounded(px(4.0))
                    .track_focus(&focus_handle)
                    .cursor(CursorStyle::IBeam)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .w_full()
                            .overflow_hidden()
                            .child(caret(is_focused && !has_text))
                            .child(if has_text {
                                div().text_size(px(11.0)).child(display_text)
                            } else {
                                div()
                                    .text_color(placeholder_color)
                                    .text_size(px(11.0))
                                    .child(placeholder.to_string())
                            })
                            .child(caret(is_focused && has_text)),
                    )
                    .id(field_id)
                    .on_click(move |_, window, _| {
                        focus_handle.focus(window);
                        window.refresh();
                    })
                    .on_key_down(cx.listener(
                        move |this: &mut Self,
                              event: &gpui::KeyDownEvent,
                              _window: &mut gpui::Window,
                              cx: &mut gpui::Context<Self>| {
                            this.handle_input_key(&field_id_str, event, cx);
                        },
                    )),
            )
            .into_any_element()
    }

    fn render_selected_provider_fields(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_dark = match self.config.general.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                cx.window_appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };

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
        widgets.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_size(px(11.0)).child("Enable Provider"))
                .child(
                    Self::winui_toggle_switch(enabled, is_dark)
                        .id("enable_provider_switch")
                        .on_click(cx.listener(
                            move |this: &mut Self,
                                  _event: &gpui::ClickEvent,
                                  _window: &mut gpui::Window,
                                  cx: &mut gpui::Context<Self>| {
                                let n = !enabled;
                                match provider_id_checkbox.as_str() {
                                    "deepseek" => this.config.deepseek.enabled = Some(n),
                                    "claude" => this.config.claude.enabled = Some(n),
                                    "codex" => this.config.codex.enabled = Some(n),
                                    "gemini" => this.config.gemini.enabled = Some(n),
                                    "antigravity" => this.config.antigravity.enabled = Some(n),
                                    "opencode" => this.config.opencode.enabled = Some(n),
                                    "mimo" => this.config.mimo.enabled = Some(n),
                                    _ => {
                                        if let Some(cfg) = api_key_provider_config_mut(
                                            &mut this.config,
                                            &provider_id_checkbox,
                                        ) {
                                            cfg.enabled = Some(n);
                                        }
                                    }
                                }
                                this.save_config();
                                cx.notify();
                            },
                        )),
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
        widgets.push(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(110.0))
                        .h(px(28.0))
                        .bg(if testing_this || testing_other {
                            gpui::rgb(0x555555)
                        } else {
                            gpui::rgb(0x005fa8)
                        })
                        .text_color(gpui::rgb(0xffffff))
                        .text_size(px(11.0))
                        .rounded(px(4.0))
                        .child(if testing_this {
                            "Testing..."
                        } else {
                            "Test Provider"
                        })
                        .id("test_provider_btn")
                        .on_click(cx.listener(
                            move |this: &mut Self,
                                  _event: &gpui::ClickEvent,
                                  _window: &mut gpui::Window,
                                  cx: &mut gpui::Context<Self>| {
                                if !testing_this && !testing_other {
                                    this.trigger_provider_test(provider_id_test.clone(), cx);
                                }
                            },
                        )),
                )
                .child(match status {
                    ProviderTestStatus::Success {
                        provider,
                        fetched_at,
                        summary,
                    } if provider == provider_id => div()
                        .text_size(px(10.0))
                        .text_color(gpui::rgb(0x2e7d32))
                        .child(format!(
                            "Test passed at {}. {summary}",
                            fetched_at.with_timezone(&chrono::Local).format("%H:%M:%S")
                        )),
                    ProviderTestStatus::Error { provider, message } if provider == provider_id => {
                        div()
                            .text_size(px(10.0))
                            .text_color(gpui::rgb(0xc62828))
                            .child(format!("Test failed: {message}"))
                    }
                    ProviderTestStatus::Testing { provider } if provider == provider_id => div()
                        .text_size(px(10.0))
                        .child("Fetching usage with current provider settings..."),
                    _ => div(),
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
        _ => None,
    }
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

use crate::version::{is_newer, normalize_version};

fn open_browser(url: &str) {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

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
