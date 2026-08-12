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
    select::{SearchableVec, Select, SelectEvent, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    tag::Tag,
};
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, OnceLock, atomic::Ordering},
    time::{Duration, Instant},
};

use crate::card_motion::{CardMotionElement, CardSlot, drop_target_index};
use crate::disclosure::DisclosureAnimation;
use crate::i18n::{self, Text};
use crate::provider::UsageData;

const CODEX_RESET_DISCLOSURE_KEY: &str = "codex-reset-credits";
const TREND_CAPTION_HEIGHT: f32 = 24.0;
const TREND_LEGEND_ROW_HEIGHT: f32 = 18.0;
const TREND_LEGEND_CHART_GAP: f32 = 6.0;
const TREND_CHART_HEIGHT: f32 = 112.0;
const TREND_PLOT_HEIGHT: f32 = 92.0;
const TREND_VALUE_LABEL_HEIGHT: f32 = 13.0;
const TREND_BAR_MAX_WIDTH: f32 = 24.0;
const PROGRESS_TRACK_HEIGHT: f32 = 8.0;

fn secondary_text_color(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0xa8a8a8)
    } else {
        gpui::rgb(0x666666)
    }
}

fn weak_text_color(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0x929292)
    } else {
        gpui::rgb(0x808080)
    }
}

#[derive(Clone, Copy)]
enum QuotifyTagTone {
    Info,
    Secondary,
    Success,
}

/// Keep the standard compact outline Tag appearance while drawing a
/// one-device-pixel border at every Windows DPI scale. A custom variant changes
/// the dark-mode semantic colors without adding a text style refinement, which
/// preserves Tag's built-in `text_xs` size and padding exactly.
fn quotify_tag(tone: QuotifyTagTone, is_dark: bool, scale_factor: f32) -> Tag {
    let tag = if is_dark {
        let color: Hsla = match tone {
            QuotifyTagTone::Info => gpui::rgb(0x60cdff),
            QuotifyTagTone::Secondary => gpui::rgb(0xc8c8c8),
            QuotifyTagTone::Success => gpui::rgb(0x6ccb5f),
        }
        .into();
        Tag::custom(color, color, color)
    } else {
        match tone {
            QuotifyTagTone::Info => Tag::info(),
            QuotifyTagTone::Secondary => Tag::secondary(),
            QuotifyTagTone::Success => Tag::success(),
        }
    };

    tag.small().border(px(1.0 / scale_factor.max(1.0)))
}

fn trend_disclosure_height(window_count: usize) -> f32 {
    TREND_CAPTION_HEIGHT
        + window_count as f32 * TREND_LEGEND_ROW_HEIGHT
        + TREND_LEGEND_CHART_GAP
        + TREND_CHART_HEIGHT
}

static COMPONENT_THEME_SETTING: OnceLock<RwLock<String>> = OnceLock::new();

fn component_theme_setting() -> &'static RwLock<String> {
    COMPONENT_THEME_SETTING.get_or_init(|| RwLock::new("system".to_string()))
}

pub fn current_component_theme_setting() -> String {
    component_theme_setting().read().clone()
}

/// Limits gpui-component's opaque surface color to a single Select subtree.
///
/// gpui-component 0.5.1 renders Select menus with `theme.background`, while
/// Quotify keeps that token transparent so the window backdrop remains visible.
/// Swapping the token only while this subtree is materialized gives the menu the
/// existing opaque popover color without changing the Root/DWM backdrop.
struct SelectPopoverSurface {
    child: AnyElement,
}

impl SelectPopoverSurface {
    fn new(child: impl IntoElement) -> Self {
        Self {
            child: child.into_any_element(),
        }
    }

    fn with_popover_theme<R>(cx: &mut App, render: impl FnOnce(&mut App) -> R) -> R {
        let (background, foreground, popover, popover_foreground) = {
            let theme = gpui_component::Theme::global(cx);
            (
                theme.background,
                theme.foreground,
                theme.popover,
                theme.popover_foreground,
            )
        };

        {
            let theme = gpui_component::Theme::global_mut(cx);
            theme.background = popover;
            theme.foreground = popover_foreground;
        }

        let result = render(cx);

        {
            let theme = gpui_component::Theme::global_mut(cx);
            theme.background = background;
            theme.foreground = foreground;
        }

        result
    }
}

impl IntoElement for SelectPopoverSurface {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectPopoverSurface {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = Self::with_popover_theme(cx, |cx| self.child.request_layout(window, cx));
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        Self::with_popover_theme(cx, |cx| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        Self::with_popover_theme(cx, |cx| self.child.paint(window, cx));
    }
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
    theme.primary = gpui::rgb(0x0067c0).into();
    theme.primary_hover = gpui::rgb(0x1975c5).into();
    theme.primary_active = gpui::rgb(0x005a9e).into();
    theme.ring = gpui::rgb(0x0067c0).into();
    theme.progress_bar = gpui::rgb(0x0067c0).into();

    if mode.is_dark() {
        // gpui-component 0.5.1 uses `background` for both Root and the Select menu.
        // Keep enough tint for Select readability without hiding the DWM Mica layer.
        theme.background = gpui::rgba(0x20202000).into();
        theme.group_box = gpui::rgba(0x2b2b2b78).into();
        theme.popover = gpui::rgba(0x2b2b2bff).into();
        theme.border = gpui::rgba(0xffffff26).into();
        theme.input = gpui::rgba(0xffffff3d).into();
        theme.muted = gpui::rgba(0xffffff24).into();
        theme.switch = gpui::rgba(0xffffff33).into();
    } else {
        theme.background = gpui::rgba(0xf3f3f300).into();
        theme.group_box = gpui::rgba(0xffffff85).into();
        theme.popover = gpui::rgba(0xf9f9f9ff).into();
        theme.border = gpui::rgba(0x0000001f).into();
        theme.input = gpui::rgba(0x0000003d).into();
        theme.muted = gpui::rgba(0x00000014).into();
        theme.switch = gpui::rgba(0x0000002e).into();
    }

    // Slider uses independent theme tokens even though Switch uses `primary`
    // directly. Keep their active track and thumb colors identical in both
    // light and dark modes.
    theme.slider_bar = theme.primary;
    theme.slider_thumb = theme.switch_thumb;
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum WebViewLoginStatus {
    LoggingIn,
    Error(String),
}

#[derive(Clone, Debug, Default)]
enum AgentScanStatus {
    #[default]
    Idle,
    Scanning,
    Complete(Vec<crate::agent_scan::DetectedAgent>),
}

#[derive(Default, Clone)]
pub struct ProviderDragState {
    held_provider: Option<String>,
    drag_start_pos: Option<Point<Pixels>>,
    dragging: bool,
    order_changed: bool,
}

#[derive(Clone, Copy)]
struct ProviderCardRenderContext {
    is_dark: bool,
    scale_factor: f32,
    animations_enabled: bool,
}

struct InputFieldState {
    input: Entity<InputState>,
    masked: bool,
    placeholder: SharedString,
    _subscription: Subscription,
}

struct ProviderSelectFieldState {
    select: Entity<SelectState<SearchableVec<String>>>,
    _subscription: Subscription,
}

struct LanguageSelectFieldState {
    select: Entity<SelectState<SearchableVec<String>>>,
    _subscription: Subscription,
}

struct SliderFieldState {
    slider: Entity<SliderState>,
    ready: bool,
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
    webview_login_status: Arc<parking_lot::Mutex<HashMap<String, WebViewLoginStatus>>>,
    budget_input_errors: HashMap<String, String>,
    pub selected_setting_provider: String,
    show_agent_scan_onboarding: bool,
    agent_scan_status: AgentScanStatus,
    disclosures: HashMap<String, DisclosureAnimation>,
    disclosure_frame_at: Option<Instant>,
    provider_card_slots: Arc<RwLock<Vec<CardSlot>>>,
    provider_order_revision: u64,
    trend_cache: crate::trend_cache::TrendCache,
    hovered_trend_series: Option<String>,
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
        i18n::set_current_language(&config.general.language);
        let show_agent_scan_onboarding = !config.general.agent_scan_onboarding_completed;
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
            webview_login_status: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            budget_input_errors: HashMap::new(),
            selected_setting_provider: "openai".to_string(),
            show_agent_scan_onboarding,
            agent_scan_status: AgentScanStatus::Idle,
            disclosures: HashMap::new(),
            disclosure_frame_at: None,
            provider_card_slots: Arc::new(RwLock::new(Vec::new())),
            provider_order_revision: 0,
            trend_cache: crate::trend_cache::TrendCache::new(),
            hovered_trend_series: None,
        }
    }

    fn save_config(&self) {
        i18n::set_current_language(&self.config.general.language);
        let mut config_to_save = self.config.clone();
        crate::secrets::store_and_scrub_config(&mut config_to_save);
        let res = if let Some(ref path) = self.config_path {
            config_to_save.save_to(path)
        } else {
            config_to_save.save()
        };
        match res {
            Ok(()) => {
                // The old Bedrock budget lived in Credential Manager under an
                // `api_key` name. Once the non-secret config has been saved,
                // remove that obsolete fallback so clearing the budget sticks.
                if let Err(err) = crate::secrets::delete("bedrock", "api_key") {
                    tracing::warn!("Failed to remove legacy Bedrock budget credential: {err}");
                }
            }
            Err(err) => {
                tracing::error!("Failed to save config: {err}");
            }
        }
    }

    fn decline_agent_scan_onboarding(&mut self) {
        self.config.general.local_agent_discovery = false;
        self.config.general.agent_scan_onboarding_completed = true;
        self.show_agent_scan_onboarding = false;
        self.agent_scan_status = AgentScanStatus::Idle;
        self.save_config();
    }

    fn trigger_agent_scan(&mut self, cx: &mut Context<Self>) {
        if matches!(self.agent_scan_status, AgentScanStatus::Scanning) {
            return;
        }

        let mut scan_config = self.config.clone();
        for provider_id in crate::agent_scan::AGENT_PROVIDER_IDS {
            set_provider_enabled_value(&mut scan_config, provider_id, None);
        }

        self.agent_scan_status = AgentScanStatus::Scanning;
        cx.notify();

        cx.spawn(|this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                let detected = cx
                    .background_executor()
                    .spawn(async move { crate::agent_scan::scan_available_agents(&scan_config) })
                    .await;

                cx.update(|cx| {
                    this.update(cx, |view, cx| {
                        view.config.general.local_agent_discovery = true;
                        view.config.general.agent_scan_onboarding_completed = true;
                        for agent in &detected {
                            set_provider_enabled_value(
                                &mut view.config,
                                agent.provider_id,
                                Some(true),
                            );
                        }

                        if view.config.general.active_provider.trim().is_empty()
                            && let Some(first) = detected.first()
                        {
                            view.config.general.active_provider = first.provider_id.to_string();
                            *view.active_provider.write() = first.provider_id.to_string();
                        }

                        view.agent_scan_status = AgentScanStatus::Complete(detected);
                        view.save_config();
                        crate::tray::request_refresh();
                        cx.notify();
                    })
                    .ok();
                })
                .ok();
            }
        })
        .detach();
    }

    fn set_primary_provider(&mut self, provider_name: &str) {
        *self.active_provider.write() = provider_name.to_string();
        self.config.general.active_provider = provider_name.to_string();
        self.save_config();

        update_tray_icon_for_active_provider(provider_name, &self.data);
        crate::tray::request_refresh();
    }

    fn move_dragged_provider_to(&mut self, provider_name: &str, target_index: usize) -> bool {
        let visible_provider_names = self
            .data
            .read()
            .iter()
            .map(|data| data.provider.clone())
            .collect::<Vec<_>>();
        let mut full_order = provider_display_order(&self.config)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        let visible_slots = full_order
            .iter()
            .enumerate()
            .filter(|(_, name)| {
                visible_provider_names
                    .iter()
                    .any(|visible| visible.eq_ignore_ascii_case(name))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let mut visible_order = visible_slots
            .iter()
            .map(|index| full_order[*index].clone())
            .collect::<Vec<_>>();

        let Some(source_index) = visible_order
            .iter()
            .position(|name| name.eq_ignore_ascii_case(provider_name))
        else {
            return false;
        };
        let target_index = target_index.min(visible_order.len().saturating_sub(1));
        if source_index == target_index {
            return false;
        }

        let provider = visible_order.remove(source_index);
        visible_order.insert(target_index, provider);
        for (slot, provider) in visible_slots.into_iter().zip(visible_order) {
            full_order[slot] = provider;
        }
        self.config.general.provider_order = full_order;
        self.provider_order_revision = self.provider_order_revision.wrapping_add(1);
        true
    }

    fn finish_provider_drag(&mut self) -> bool {
        if self.drag.order_changed {
            self.save_config();
        }
        if self.drag.held_provider.is_none() {
            return false;
        }

        self.drag = ProviderDragState::default();
        true
    }

    fn handle_provider_drag_move(
        &mut self,
        position: Point<Pixels>,
        left_button_pressed: bool,
    ) -> bool {
        if !left_button_pressed {
            return self.finish_provider_drag();
        }

        let Some(held_provider) = self.drag.held_provider.clone() else {
            return false;
        };

        let mut needs_render = false;
        if !self.drag.dragging
            && let Some(start_pos) = self.drag.drag_start_pos
        {
            let dx = (position.x - start_pos.x) / px(1.0);
            let dy = (position.y - start_pos.y) / px(1.0);
            if dx.abs() + dy.abs() > 6.0 {
                self.finish_disclosure_animations();
                self.drag.dragging = true;
                needs_render = true;
            }
        }

        if !self.drag.dragging {
            return needs_render;
        }

        let target_index = {
            let slots = self.provider_card_slots.read();
            drop_target_index(&slots, position.x / px(1.0), position.y / px(1.0))
        };

        if let Some(target_index) = target_index
            && self.move_dragged_provider_to(&held_provider, target_index)
        {
            self.drag.order_changed = true;
            needs_render = true;
        }

        needs_render
    }

    fn disclosure(&self, key: &str) -> DisclosureAnimation {
        self.disclosures.get(key).copied().unwrap_or_default()
    }

    fn toggle_disclosure(&mut self, key: String) {
        let now = Instant::now();
        if self
            .disclosures
            .values()
            .any(DisclosureAnimation::is_animating)
        {
            self.advance_running_disclosures_to(now);
        }
        self.disclosures.entry(key).or_default().toggle();

        if client_area_animations_enabled() {
            self.disclosure_frame_at = Some(now);
        } else {
            self.finish_disclosure_animations();
        }
    }

    fn advance_disclosure_animations(&mut self, window: &mut Window) {
        if !self
            .disclosures
            .values()
            .any(DisclosureAnimation::is_animating)
        {
            self.disclosures
                .retain(|_, animation| animation.should_render_content());
            self.disclosure_frame_at = None;
            return;
        }

        let now = Instant::now();
        self.advance_running_disclosures_to(now);

        if self
            .disclosures
            .values()
            .any(DisclosureAnimation::is_animating)
        {
            window.request_animation_frame();
        } else {
            self.disclosure_frame_at = None;
        }
    }

    fn advance_running_disclosures_to(&mut self, now: Instant) {
        let delta = self
            .disclosure_frame_at
            .replace(now)
            .map(|previous| now.saturating_duration_since(previous))
            .unwrap_or_default();

        for animation in self.disclosures.values_mut() {
            animation.advance(delta);
        }
        self.disclosures
            .retain(|_, animation| animation.should_render_content());
    }

    fn finish_disclosure_animations(&mut self) {
        for animation in self.disclosures.values_mut() {
            animation.finish();
        }
        self.disclosures
            .retain(|_, animation| animation.should_render_content());
        self.disclosure_frame_at = None;
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
                                anyhow::anyhow!(i18n::text(Text::ProviderCouldNotCreate))
                            })?;
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()?;
                        let mut data = rt.block_on(provider.fetch_usage())?;
                        let budget_unavailable = crate::apply_provider_budget(&config, &mut data);
                        if budget_unavailable {
                            anyhow::bail!(i18n::text(Text::BudgetWindowMissing));
                        }
                        Ok::<UsageData, anyhow::Error>(data)
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

    fn trigger_webview_login(&self, provider: String, cx: &mut Context<Self>) {
        if !crate::webview_login::supports_provider(&provider) {
            return;
        }

        {
            let mut statuses = self.webview_login_status.lock();
            if matches!(statuses.get(&provider), Some(WebViewLoginStatus::LoggingIn)) {
                return;
            }
            statuses.insert(provider.clone(), WebViewLoginStatus::LoggingIn);
        }
        cx.notify();

        let statuses = self.webview_login_status.clone();
        let initial_url = if matches!(provider.as_str(), "opencode" | "opencodego") {
            crate::webview_login::opencode_workspace_go_url(&self.config.opencode.workspace_id)
        } else {
            None
        };
        std::thread::spawn(move || {
            let result =
                crate::webview_login::login_and_store_for_provider_at(&provider, true, initial_url);
            match result {
                Ok(_) => {
                    statuses.lock().remove(&provider);
                    crate::tray::request_refresh();
                }
                Err(err) => {
                    statuses
                        .lock()
                        .insert(provider, WebViewLoginStatus::Error(err.to_string()));
                }
            }
            crate::trigger_gui_update();
        });
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
                        // GPUI's background executor is not a Tokio executor. Polling a
                        // reqwest request on it panics because no Tokio reactor is active.
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()?;
                        rt.block_on(async {
                            let client = reqwest::Client::builder()
                                .timeout(Duration::from_secs(10))
                                .build()?;
                            let resp = client
                                .get(
                                    "https://api.github.com/repos/zuoxinyu/quotify/releases/latest",
                                )
                                .header("User-Agent", "Quotify-App")
                                .send()
                                .await?
                                .error_for_status()?
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
                            let result: anyhow::Result<(String, String)> =
                                Ok((latest_tag, release_url));
                            result
                        })
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

fn client_area_animations_enabled() -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        };

        let mut enabled = windows::core::BOOL::from(true);
        let result = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some(std::ptr::from_mut(&mut enabled).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
            )
        };
        match result {
            Ok(()) => enabled.as_bool(),
            Err(err) => {
                tracing::debug!("Failed to read Windows client-area animation setting: {err}");
                true
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    true
}

impl Render for QuotifyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.advance_disclosure_animations(window);
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
        let backdrop_active = crate::IS_BACKDROP_ACTIVE.load(Ordering::SeqCst);
        let bg_fill = if backdrop_active {
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

        if self.show_agent_scan_onboarding {
            return div()
                .flex()
                .flex_col()
                .w_full()
                .h_full()
                .p(px(24.0))
                .bg(bg_fill)
                .text_color(text_color)
                .font_family("Segoe UI")
                .child(self.render_agent_scan_onboarding(is_dark, cx));
        }

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
                        _ => self
                            .render_dashboard(is_dark, window, cx)
                            .into_any_element(),
                    })
                    .id("body_view")
                    .overflow_y_scrollbar(),
            )
            .on_mouse_move(cx.listener(
                |this: &mut Self,
                 event: &MouseMoveEvent,
                 _window: &mut Window,
                 cx: &mut Context<Self>| {
                    if this.handle_provider_drag_move(event.position, event.dragging()) {
                        cx.notify();
                    }
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(
                    |this: &mut Self,
                     _event: &MouseUpEvent,
                     _window: &mut Window,
                     cx: &mut Context<Self>| {
                        if this.finish_provider_drag() {
                            cx.notify();
                        }
                    },
                ),
            )
    }
}

impl QuotifyApp {
    fn render_agent_scan_onboarding(&self, is_dark: bool, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let secondary_text = secondary_text_color(is_dark);

        let status_content = match &self.agent_scan_status {
            AgentScanStatus::Idle => {
                let decline_app = app.clone();
                let scan_app = app.clone();
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap_4()
                    .child(
                        GroupBox::new()
                            .fill()
                            .child(
                                div()
                                    .flex()
                                    .w_full()
                                    .gap_3()
                                    .child(div().flex_none().child(fluent_icon("\u{E721}", 15.0)))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_size(px(12.0))
                                                    .child(i18n::text(Text::LocalOnlyDiscovery)),
                                            )
                                            .child(
                                                div()
                                                    .w_full()
                                                    .text_size(px(10.0))
                                                    .text_color(secondary_text)
                                                    .whitespace_normal()
                                                    .child(i18n::text(
                                                        Text::LocalOnlyDiscoveryDescription,
                                                    )),
                                            ),
                                    ),
                            )
                            .child(Divider::horizontal())
                            .child(
                                div()
                                    .flex()
                                    .w_full()
                                    .gap_3()
                                    .child(div().flex_none().child(fluent_icon("\u{E8D7}", 15.0)))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_size(px(12.0))
                                                    .child(i18n::text(Text::YouStayInControl)),
                                            )
                                            .child(
                                                div()
                                                    .w_full()
                                                    .text_size(px(10.0))
                                                    .text_color(secondary_text)
                                                    .whitespace_normal()
                                                    .child(i18n::text(
                                                        Text::YouStayInControlDescription,
                                                    )),
                                            ),
                                    ),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("onboarding_skip_agent_scan")
                                    .label(i18n::text(Text::NotNow))
                                    .on_click(move |_, _, cx| {
                                        decline_app
                                            .update(cx, |this, cx| {
                                                this.decline_agent_scan_onboarding();
                                                cx.notify();
                                            })
                                            .ok();
                                    }),
                            )
                            .child(
                                Button::new("onboarding_allow_agent_scan")
                                    .primary()
                                    .child(fluent_icon("\u{E721}", 11.0))
                                    .child(i18n::text(Text::AllowAndScan))
                                    .on_click(move |_, _, cx| {
                                        scan_app
                                            .update(cx, |this, cx| {
                                                this.trigger_agent_scan(cx);
                                            })
                                            .ok();
                                    }),
                            ),
                    )
                    .into_any_element()
            }
            AgentScanStatus::Scanning => div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .child(fluent_icon("\u{E721}", 24.0))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_size(px(13.0))
                        .child(i18n::text(Text::ScanningAgents)),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(secondary_text)
                        .child(i18n::text(Text::ScanFewSeconds)),
                )
                .into_any_element(),
            AgentScanStatus::Complete(detected) => {
                let continue_app = app.clone();
                let detected_count = detected.len();
                let result_message = if detected.is_empty() {
                    i18n::text(Text::NoReadyAgentsOnboarding).to_string()
                } else {
                    i18n::scan_complete_message(detected_count)
                };

                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap_3()
                    .child(
                        Alert::success("agent-scan-complete", result_message)
                            .title(i18n::text(Text::ScanComplete)),
                    )
                    .when(!detected.is_empty(), |view| {
                        view.child(
                            GroupBox::new().fill().child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .max_h(px(230.0))
                                    .overflow_y_scrollbar()
                                    .children(detected.iter().map(|agent| {
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .py_1()
                                            .child(fluent_icon("\u{E73E}", 10.0))
                                            .child(
                                                div().text_size(px(11.0)).child(agent.display_name),
                                            )
                                    })),
                            ),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div().flex().justify_end().child(
                            Button::new("onboarding_finish_agent_scan")
                                .primary()
                                .label(i18n::text(Text::Continue))
                                .on_click(move |_, _, cx| {
                                    continue_app
                                        .update(cx, |this, cx| {
                                            this.show_agent_scan_onboarding = false;
                                            cx.notify();
                                        })
                                        .ok();
                                }),
                        ),
                    )
                    .into_any_element()
            }
        };

        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(img("assets/icons/quotify.svg").w(px(32.0)).h(px(32.0)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_size(px(18.0))
                                    .child(i18n::text(Text::WelcomeTitle)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(secondary_text)
                                    .child(i18n::text(Text::WelcomeSubtitle)),
                            ),
                    ),
            )
            .child(Divider::horizontal())
            .child(status_content)
            .into_any_element()
    }

    fn render_header(&self, active_page: u32, is_dark: bool, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let weak_text = secondary_text_color(is_dark);

        let refresh_age = {
            let last = *self.last_refresh.read();
            let elapsed = chrono::Utc::now() - last;
            let secs = elapsed.num_seconds();
            i18n::refresh_age(secs)
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
                            .child(fluent_icon("\u{E72B}", 12.0))
                            .tooltip(i18n::text(Text::Back))
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
                            .tooltip(i18n::text(Text::AboutQuotify))
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
                                1 => i18n::text(Text::About),
                                2 => i18n::text(Text::Settings),
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
                            .child(fluent_icon("\u{E72C}", 12.0))
                            .tooltip(i18n::text(Text::RefreshUsage))
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
                            .child(fluent_icon("\u{E713}", 12.0))
                            .tooltip(i18n::text(Text::Settings))
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

    fn render_dashboard(
        &mut self,
        is_dark: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let data = self.data.read().clone();
        let all_providers = provider_display_order(&self.config);
        let visible_providers = all_providers
            .into_iter()
            .filter(|(name, _)| data.iter().any(|d| d.provider == *name))
            .collect::<Vec<_>>();

        if visible_providers.is_empty() {
            self.provider_card_slots.write().clear();
            return div()
                .flex()
                .flex_col()
                .items_center()
                .w_full()
                .pt_8()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(weak_text_color(is_dark))
                        .child(i18n::text(Text::NoEnabledProviders)),
                )
                .into_any_element();
        }

        let card_context = ProviderCardRenderContext {
            is_dark,
            scale_factor: window.scale_factor(),
            animations_enabled: client_area_animations_enabled(),
        };
        let mut cards = Vec::new();
        for (idx, (name, _)) in visible_providers.iter().enumerate() {
            if let Some(pdata) = data.iter().find(|d| d.provider == *name) {
                cards.push(self.render_provider_card(name, pdata, idx, card_context, cx));
            }
        }
        let provider_card_slots = self.provider_card_slots.clone();

        div()
            .flex()
            .flex_col()
            .justify_between()
            .w_full()
            .pb(px(20.0))
            .gap_5()
            .children(cards)
            .on_children_prepainted(move |bounds, _, _| {
                *provider_card_slots.write() =
                    bounds.into_iter().map(CardSlot::from_bounds).collect();
            })
            .into_any_element()
    }

    fn render_provider_card(
        &mut self,
        name: &str,
        data: &UsageData,
        row_idx: usize,
        card_context: ProviderCardRenderContext,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ProviderCardRenderContext {
            is_dark,
            scale_factor,
            animations_enabled,
        } = card_context;
        let provider_name = name.to_string();
        let mouse_down_provider = provider_name.clone();
        let card_elt_id = SharedString::from(format!("card_{name}"));

        let effective_budget = self
            .config
            .provider_budget(name)
            .or_else(|| crate::legacy_bedrock_budget(&self.config, name));
        let trend = {
            let history = self.history.read();
            self.trend_cache
                .get_or_compute(&history, data, 7, chrono::Utc::now(), effective_budget)
        };
        let reset_credits = crate::provider::codex::reset_credits(data);
        let reset_animation = self.disclosure(CODEX_RESET_DISCLOSURE_KEY);
        let show_reset_credits = reset_credits.is_some() && reset_animation.target_is_open();

        let card = div()
            .w_full()
            .id(card_elt_id)
            .child(
                GroupBox::new()
                    .fill()
                    .child(self.render_card_header(
                        name,
                        SharedString::from(crate::provider::display_name(name).to_owned()),
                        data,
                        reset_credits.as_ref(),
                        show_reset_credits,
                        row_idx,
                        is_dark,
                        scale_factor,
                        cx,
                    ))
                    .child(self.render_card_body(name, data, trend, is_dark, cx))
                    .when_some(reset_credits, |card, resets| {
                        let (details, height) =
                            Self::render_codex_reset_details(&resets, is_dark, scale_factor);
                        card.child(Self::render_animated_disclosure(
                            reset_animation,
                            height,
                            details,
                        ))
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(
                    move |this: &mut Self,
                          event: &MouseDownEvent,
                          _window: &mut gpui::Window,
                          cx: &mut gpui::Context<Self>| {
                        this.drag = ProviderDragState {
                            held_provider: Some(mouse_down_provider.clone()),
                            drag_start_pos: Some(event.position),
                            ..ProviderDragState::default()
                        };
                        cx.notify();
                    },
                ),
            );

        CardMotionElement::new(
            provider_name,
            self.provider_order_revision,
            animations_enabled,
            card,
        )
        .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_card_header(
        &self,
        name: &str,
        display_name: SharedString,
        data: &UsageData,
        reset_credits: Option<&crate::provider::CodexResetCredits>,
        show_reset_credits: bool,
        row_idx: usize,
        is_dark: bool,
        scale_factor: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_primary = self.active_provider.read().eq_ignore_ascii_case(name);
        let provider_name = name.to_string();
        let provider_icon_element = if let Some(icon_path) = provider_icon(name, is_dark) {
            div()
                .w(px(16.0))
                .h(px(16.0))
                .child(img(icon_path).w_full().h_full())
                .into_any_element()
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
                .into_any_element()
        };
        let provider_icon_hitbox = div()
            .id(SharedString::from(format!(
                "provider-icon-{name}-{row_idx}"
            )))
            .w(px(18.0))
            .h(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(
                    move |this: &mut Self,
                          event: &MouseDownEvent,
                          _window: &mut Window,
                          cx: &mut Context<Self>| {
                        cx.stop_propagation();
                        if event.click_count == 2 {
                            this.set_primary_provider(&provider_name);
                            cx.notify();
                        }
                    },
                ),
            )
            .child(provider_icon_element);
        let reset_tag = reset_credits.map(|resets| {
            let app = cx.entity().downgrade();
            div()
                .id(SharedString::from(format!(
                    "codex-reset-credits-hitbox-{row_idx}"
                )))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Button::new(SharedString::from(format!("codex-reset-credits-{row_idx}")))
                        .ghost()
                        .xsmall()
                        .p_0()
                        .h_auto()
                        .child(
                            quotify_tag(QuotifyTagTone::Info, is_dark, scale_factor)
                                .outline()
                                .gap_1()
                                .child(i18n::reset_count(resets.available_count))
                                .child(if show_reset_credits { "▴" } else { "▾" }),
                        )
                        .tooltip(if show_reset_credits {
                            i18n::text(Text::HideResetDetails)
                        } else {
                            i18n::text(Text::ShowResetDetails)
                        })
                        .on_click(move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                this.toggle_disclosure(CODEX_RESET_DISCLOSURE_KEY.to_string());
                                cx.notify();
                            })
                            .ok();
                        }),
                )
                .into_any_element()
        });
        let budget_tag = data
            .windows
            .iter()
            .find(|window| {
                crate::provider::is_budget_spend_window(
                    &data.provider,
                    &window.label,
                    window.unit.as_deref(),
                )
            })
            .and_then(|window| window.used.zip(window.limit))
            .filter(|(used, limit)| {
                used.is_finite() && *used >= 0.0 && limit.is_finite() && *limit > 0.0
            })
            .map(|(used, limit)| {
                quotify_tag(QuotifyTagTone::Info, is_dark, scale_factor)
                    .outline()
                    .child(i18n::budget_left(&format_credits_balance(
                        (limit - used).max(0.0),
                    )))
                    .into_any_element()
            });
        let budget_configured = self.config.provider_budget(name).is_some();
        let budget_unavailable_tag =
            (budget_configured && budget_tag.is_none() && data.error.is_none()).then(|| {
                quotify_tag(QuotifyTagTone::Secondary, is_dark, scale_factor)
                    .outline()
                    .child(i18n::text(Text::BudgetUnavailable))
                    .into_any_element()
            });
        let credits_tag = data.credits.as_ref().map(|credits| {
            let text = format!(
                "{} {}",
                format_credits_balance(credits.balance),
                credits.currency
            );
            quotify_tag(QuotifyTagTone::Info, is_dark, scale_factor)
                .outline()
                .child(text)
                .into_any_element()
        });
        let mut status_tags = Vec::new();
        if let Some(tier) = data
            .subscription_tier
            .as_deref()
            .map(str::trim)
            .filter(|tier| !tier.is_empty())
        {
            status_tags.push(
                quotify_tag(QuotifyTagTone::Secondary, is_dark, scale_factor)
                    .outline()
                    .child(tier.to_string())
                    .into_any_element(),
            );
        }
        if let Some(reset_tag) = reset_tag {
            status_tags.push(reset_tag);
        } else {
            if let Some(credits_tag) = credits_tag {
                status_tags.push(credits_tag);
            }
            if let Some(budget_tag) = budget_tag {
                status_tags.push(budget_tag);
            } else if let Some(budget_unavailable_tag) = budget_unavailable_tag {
                status_tags.push(budget_unavailable_tag);
            }
        }

        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(provider_icon_hitbox)
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_size(px(12.0))
                            .child(display_name),
                    )
                    .child(if is_primary {
                        div()
                            .text_color(if is_dark {
                                gpui::rgb(0x60cdff)
                            } else {
                                gpui::rgb(0x0067c0)
                            })
                            .child(fluent_icon("\u{E735}", 12.0))
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(div().flex().items_center().gap_1().children(status_tags))
            .into_any_element()
    }

    fn render_card_body(
        &self,
        provider: &str,
        data: &UsageData,
        trend: Option<Arc<crate::trend_cache::CachedProviderTrends>>,
        is_dark: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(ref error) = data.error {
            let login_message = crate::webview_login::login_required_message(error);
            if let Some(login_message) = login_message
                && crate::webview_login::supports_provider(provider)
            {
                let login_status = self.webview_login_status.lock().get(provider).cloned();
                let logging_in = matches!(login_status, Some(WebViewLoginStatus::LoggingIn));
                let login_error = match login_status {
                    Some(WebViewLoginStatus::Error(error)) => Some(error),
                    _ => None,
                };
                let provider = provider.to_string();
                let app = cx.entity().downgrade();

                return div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        Alert::error(
                            SharedString::from(format!("provider-login-required-{provider}")),
                            login_message,
                        )
                        .small(),
                    )
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                Button::new(SharedString::from(format!(
                                    "provider-webview-login-{provider}"
                                )))
                                .primary()
                                .small()
                                .label(if logging_in {
                                    i18n::text(Text::WaitingForLogin)
                                } else if login_error.is_some() {
                                    i18n::text(Text::RetryLogin)
                                } else {
                                    i18n::text(Text::LogIn)
                                })
                                .loading(logging_in)
                                .disabled(logging_in)
                                .on_click(move |_, _, cx| {
                                    let provider = provider.clone();
                                    app.update(cx, |this, cx| {
                                        this.trigger_webview_login(provider, cx);
                                    })
                                    .ok();
                                }),
                            ),
                    )
                    .when_some(login_error, |body, error| {
                        body.child(
                            Alert::error(
                                SharedString::from(format!(
                                    "provider-webview-login-error-{}",
                                    data.provider
                                )),
                                format!("{}: {error}", i18n::text(Text::LoginFailed)),
                            )
                            .small(),
                        )
                    })
                    .into_any_element();
            }

            return Alert::error(
                SharedString::from(format!("provider-error-{provider}")),
                error.clone(),
            )
            .small()
            .into_any_element();
        }

        let mut children = data
            .windows
            .iter()
            .filter(|window| {
                !data.provider.eq_ignore_ascii_case("codex")
                    || !crate::provider::codex::is_reset_credits_window(window)
            })
            .map(|w| Self::render_progress_row(provider, w, is_dark))
            .collect::<Vec<_>>();

        if let Some(trend_val) = trend {
            children.push(self.render_trend_section(provider, trend_val.as_ref(), is_dark, cx));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(12.0)) // 8px gap between progress rows matches egui's default spacing
            .children(children)
            .into_any_element()
    }

    fn render_animated_disclosure(
        animation: DisclosureAnimation,
        expanded_height: f32,
        content: AnyElement,
    ) -> AnyElement {
        let progress = animation.progress().clamp(0.0, 1.0);

        div()
            .w_full()
            .h(px(expanded_height * progress))
            .flex_none()
            .overflow_hidden()
            .when(animation.should_render_content(), |disclosure| {
                disclosure.child(
                    div()
                        .w_full()
                        .h(px(expanded_height))
                        .flex_none()
                        .opacity(progress)
                        .child(content),
                )
            })
            .into_any_element()
    }

    fn render_trend_section(
        &self,
        provider: &str,
        trends: &crate::trend_cache::CachedProviderTrends,
        is_dark: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let provider_key = provider.to_ascii_lowercase();
        let disclosure_key = format!("trend:{provider_key}");
        let animation = self.disclosure(&disclosure_key);
        let expanded = animation.target_is_open();
        let window_count = trends.windows.len();
        let trend_text = i18n::trend_count(window_count);
        let app = cx.entity().downgrade();
        let toggle_key = disclosure_key.clone();
        let weak_text = weak_text_color(is_dark);
        let (histograms, histogram_height) =
            self.render_trend_histograms(&provider_key, &trends.windows, is_dark, cx);

        div()
            .mt_2()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new(SharedString::from(format!("trend-toggle-{provider_key}")))
                    .ghost()
                    .xsmall()
                    .p_0()
                    .h_auto()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .w_full()
                            .text_color(weak_text)
                            .child(div().text_size(px(10.0)).child(trend_text))
                            .child(fluent_icon(
                                if expanded { "\u{E70E}" } else { "\u{E70D}" },
                                10.0,
                            )),
                    )
                    .tooltip(if expanded {
                        i18n::text(Text::HideUsageHistogram)
                    } else {
                        i18n::text(Text::ShowUsageHistogram)
                    })
                    .on_click(move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.toggle_disclosure(toggle_key.clone());
                            cx.notify();
                        })
                        .ok();
                    }),
            )
            .child(Self::render_animated_disclosure(
                animation,
                histogram_height,
                histograms,
            ))
            .into_any_element()
    }

    fn render_trend_histograms(
        &self,
        provider: &str,
        trends: &[crate::trend_cache::CachedWindowTrend],
        is_dark: bool,
        cx: &mut Context<Self>,
    ) -> (AnyElement, f32) {
        let weak_text = weak_text_color(is_dark);
        let bucket_count = trends
            .iter()
            .map(|trend| trend.histogram_buckets.len())
            .max()
            .unwrap_or(0);
        let latest_values = trends
            .iter()
            .flat_map(|trend| &trend.histogram_buckets)
            .filter_map(|bucket| bucket.latest_percent)
            .filter(|value| value.is_finite())
            .map(|value| value.max(0.0))
            .collect::<Vec<_>>();
        let series_states = trends
            .iter()
            .map(|trend| crate::usage_history::classify_histogram_buckets(&trend.histogram_buckets))
            .collect::<Vec<_>>();
        let chart_state =
            if series_states.contains(&crate::usage_history::TrendHistogramState::Chart) {
                crate::usage_history::TrendHistogramState::Chart
            } else if series_states
                .contains(&crate::usage_history::TrendHistogramState::AllRoundedToZero)
            {
                crate::usage_history::TrendHistogramState::AllRoundedToZero
            } else {
                crate::usage_history::TrendHistogramState::LatestUnavailable
            };
        let max_value = latest_values.iter().copied().fold(1.0_f64, f64::max);
        let grid_color = if is_dark {
            gpui::rgba(0xffffff26)
        } else {
            gpui::rgba(0x0000001f)
        };
        let active_series = self.hovered_trend_series.as_deref();
        let mut legend_rows = Vec::with_capacity(trends.len());

        for (series_index, window_trend) in trends.iter().enumerate() {
            let series_id = format!("{provider}:{}", window_trend.key.as_str());
            let dimmed = active_series.is_some_and(|active| active != series_id);
            let hover_id = series_id.clone();
            let series_color = trend_series_color(series_index, is_dark);
            legend_rows.push(
                div()
                    .id(SharedString::from(format!("trend-legend-{series_id}")))
                    .w_full()
                    .h(px(TREND_LEGEND_ROW_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .cursor_pointer()
                    .opacity(if dimmed { 0.28 } else { 1.0 })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_hover(cx.listener(
                        move |this: &mut Self,
                              hovered: &bool,
                              _window: &mut gpui::Window,
                              cx: &mut gpui::Context<Self>| {
                            if *hovered {
                                this.hovered_trend_series = Some(hover_id.clone());
                            } else if this.hovered_trend_series.as_deref()
                                == Some(hover_id.as_str())
                            {
                                this.hovered_trend_series = None;
                            }
                            cx.notify();
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(series_color))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(i18n::window_label(&window_trend.label).into_owned()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(9.0))
                            .text_color(weak_text)
                            .child(format_trend_metrics(&window_trend.trend)),
                    ),
            );
        }

        let chart = match chart_state {
            crate::usage_history::TrendHistogramState::Chart => {
                let mut bucket_groups = Vec::with_capacity(bucket_count);
                for bucket_index in 0..bucket_count {
                    let days_ago = bucket_count.saturating_sub(bucket_index + 1);
                    let bucket_label: SharedString = i18n::histogram_day_label(days_ago).into();
                    let mut series_bars = Vec::with_capacity(trends.len());

                    for (series_index, window_trend) in trends.iter().enumerate() {
                        let series_id = format!("{provider}:{}", window_trend.key.as_str());
                        let dimmed = active_series.is_some_and(|active| active != series_id);
                        let highlighted =
                            active_series == Some(series_id.as_str()) || trends.len() == 1;
                        let value = window_trend
                            .histogram_buckets
                            .get(bucket_index)
                            .and_then(|bucket| bucket.latest_percent)
                            .filter(|value| value.is_finite())
                            .map(|value| value.max(0.0));
                        let bar_height = value.map_or(0.0, |value| {
                            ((value / max_value) as f32
                                * (TREND_PLOT_HEIGHT - TREND_VALUE_LABEL_HEIGHT))
                                .max(1.0)
                        });
                        let hover_id = series_id.clone();
                        let series_color = trend_series_color(series_index, is_dark);

                        series_bars.push(
                            div()
                                .id(SharedString::from(format!(
                                    "trend-bar-{series_id}-{bucket_index}"
                                )))
                                .relative()
                                .flex_1()
                                .min_w(px(0.0))
                                .h_full()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_end()
                                .cursor_pointer()
                                .opacity(if dimmed { 0.22 } else { 1.0 })
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .on_hover(cx.listener(
                                    move |this: &mut Self,
                                          hovered: &bool,
                                          _window: &mut gpui::Window,
                                          cx: &mut gpui::Context<Self>| {
                                        if *hovered {
                                            this.hovered_trend_series =
                                                Some(hover_id.clone());
                                        } else if this.hovered_trend_series.as_deref()
                                            == Some(hover_id.as_str())
                                        {
                                            this.hovered_trend_series = None;
                                        }
                                        cx.notify();
                                    },
                                ))
                                .when(highlighted && value.is_some(), |bar| {
                                    bar.child(
                                        div()
                                            .absolute()
                                            .bottom(px(bar_height))
                                            .left_0()
                                            .w_full()
                                            .h(px(TREND_VALUE_LABEL_HEIGHT))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(8.0))
                                            .child(format_histogram_percent(value.unwrap())),
                                    )
                                })
                                .child(
                                    div()
                                        .w_full()
                                        .max_w(px(TREND_BAR_MAX_WIDTH))
                                        .h(px(bar_height))
                                        .flex_none()
                                        .rounded(px(2.0))
                                        .bg(if value.is_some() {
                                            series_color
                                        } else {
                                            gpui::rgba(0x00000000)
                                        }),
                                ),
                        );
                    }

                    bucket_groups.push(
                        div()
                            .flex_1()
                            .h(px(TREND_CHART_HEIGHT))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .w_full()
                                    .h(px(TREND_PLOT_HEIGHT))
                                    .flex_none()
                                    .flex()
                                    .items_end()
                                    .gap(px(2.0))
                                    .children(series_bars),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(9.0))
                                    .text_color(weak_text)
                                    .child(bucket_label),
                            ),
                    );
                }

                let grid_lines = (1..=4)
                    .map(|index| {
                        div()
                            .absolute()
                            .top(px(TREND_PLOT_HEIGHT * index as f32 / 4.0))
                            .left_0()
                            .w_full()
                            .h(px(1.0))
                            .bg(grid_color)
                    })
                    .collect::<Vec<_>>();

                div()
                    .relative()
                    .w_full()
                    .h(px(TREND_CHART_HEIGHT))
                    .flex_none()
                    .children(grid_lines)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .w_full()
                            .h_full()
                            .flex()
                            .gap_2()
                            .children(bucket_groups),
                    )
                    .into_any_element()
            }
            crate::usage_history::TrendHistogramState::AllRoundedToZero => div()
                .flex()
                .items_center()
                .justify_center()
                .w_full()
                .h(px(TREND_CHART_HEIGHT))
                .text_size(px(10.0))
                .text_color(weak_text)
                .child(i18n::text(Text::HistogramZero))
                .into_any_element(),
            crate::usage_history::TrendHistogramState::LatestUnavailable => div()
                .flex()
                .items_center()
                .justify_center()
                .w_full()
                .h(px(TREND_CHART_HEIGHT))
                .text_size(px(10.0))
                .text_color(weak_text)
                .child(i18n::text(Text::HistogramUnavailable))
                .into_any_element(),
        };
        let disclosure_height = trend_disclosure_height(trends.len());

        (
            div()
                .w_full()
                .h(px(disclosure_height))
                .flex()
                .flex_col()
                .child(div().w_full().flex().flex_col().children(legend_rows))
                .child(div().w_full().h(px(TREND_LEGEND_CHART_GAP)).flex_none())
                .child(chart)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .w_full()
                        .h(px(TREND_CAPTION_HEIGHT))
                        .flex_none()
                        .text_size(px(9.0))
                        .text_color(weak_text)
                        .text_align(TextAlign::Center)
                        .child(i18n::text(Text::HistogramCaption)),
                )
                .into_any_element(),
            disclosure_height,
        )
    }

    fn render_codex_reset_details(
        resets: &crate::provider::CodexResetCredits,
        is_dark: bool,
        scale_factor: f32,
    ) -> (AnyElement, f32) {
        let weak_text = weak_text_color(is_dark);

        let rows = resets
            .credits
            .iter()
            .enumerate()
            .map(|(index, credit)| {
                let status = credit.status.trim();
                let status_text = i18n::credit_status(status).into_owned();
                let status_tag = if status.eq_ignore_ascii_case("available") {
                    quotify_tag(QuotifyTagTone::Success, is_dark, scale_factor)
                        .outline()
                        .child(status_text)
                        .into_any_element()
                } else {
                    quotify_tag(QuotifyTagTone::Secondary, is_dark, scale_factor)
                        .outline()
                        .child(status_text)
                        .into_any_element()
                };

                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .h(px(28.0))
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(20.0))
                                    .text_size(px(10.0))
                                    .text_color(weak_text)
                                    .child(format!("#{}", index + 1)),
                            )
                            .child(status_tag),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(weak_text)
                            .child(format_reset_credit_expiry(credit.expires_at)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let details_height = 29.0
            + if rows.is_empty() {
                36.0
            } else {
                rows.len() as f32 * 28.0
            };

        let details = div()
            .w_full()
            .h(px(details_height))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_end()
                    .w_full()
                    .h(px(9.0))
                    .child(Divider::horizontal()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .h(px(20.0))
                    .text_size(px(10.0))
                    .text_color(weak_text)
                    .child(i18n::text(Text::ResetCredit))
                    .child(i18n::text(Text::Expires)),
            )
            .when(rows.is_empty(), |details| {
                details.child(
                    div()
                        .flex()
                        .items_center()
                        .w_full()
                        .h(px(36.0))
                        .text_size(px(10.0))
                        .text_color(weak_text)
                        .child(i18n::text(Text::NoResetCreditDetails)),
                )
            })
            .children(rows)
            .into_any_element();

        (details, details_height)
    }

    fn render_progress_row(
        provider: &str,
        w: &crate::provider::UsageWindow,
        is_dark: bool,
    ) -> AnyElement {
        let pct = w.used_percent.clamp(0.0, 100.0);
        let fill_color = usage_percent_color(pct, is_dark);
        let detail = usage_window_detail_text(provider, w);

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
                    .child(i18n::window_label(&w.label).into_owned()),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h(px(PROGRESS_TRACK_HEIGHT))
                    .rounded_full()
                    .overflow_hidden()
                    .child(Progress::new().value(0.0).bg(fill_color).w_full().h_full())
                    .when(pct > 0.0, |track| {
                        track.child(
                            Progress::new()
                                .value(100.0)
                                .bg(fill_color)
                                .absolute()
                                .top_0()
                                .left_0()
                                .w(relative((pct / 100.0) as f32))
                                .min_w(px(PROGRESS_TRACK_HEIGHT))
                                .h_full(),
                        )
                    }),
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
                    .w(px(52.0))
                    .flex()
                    .justify_end()
                    .text_size(px(10.0))
                    .text_color(weak_text_color(is_dark))
                    .child(detail),
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
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .child(format!("{}: {ver}", i18n::text(Text::Version))),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .child(format!("{}: zuoxinyu", i18n::text(Text::Author))),
            )
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
                    .child(i18n::text(Text::CheckForUpdates)),
            )
            .child(
                Button::new("check_updates_btn")
                    .primary()
                    .small()
                    .w(px(130.0))
                    .label(i18n::text(Text::CheckNow))
                    .loading(checking)
                    .disabled(checking)
                    .on_click(move |_, _, cx| {
                        app.update(cx, |this, cx| this.trigger_check_update(cx))
                            .ok();
                    }),
            )
            .child(match status {
                UpdateStatus::Checking => {
                    Alert::info("update-checking", i18n::text(Text::CheckingUpdates))
                        .small()
                        .into_any_element()
                }
                UpdateStatus::UpToDate { .. } => {
                    Alert::success("update-current", i18n::text(Text::AppUpToDate))
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
                            i18n::new_version_available(&latest_version),
                        )
                        .small(),
                    )
                    .child(
                        Link::new("view_release_page_link")
                            .href(release_url)
                            .text_size(px(11.0))
                            .child(i18n::text(Text::ViewReleasePage)),
                    )
                    .into_any_element(),
                UpdateStatus::Error(err) => Alert::error(
                    "update-error",
                    format!("{}: {err}", i18n::text(Text::UpdateCheckFailed)),
                )
                .small()
                .into_any_element(),
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
            .child(self.render_notification_settings(is_dark, window, cx))
            .child(self.render_provider_settings(is_dark, window, cx))
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
        let language_app = cx.entity().downgrade();
        let theme_app = cx.entity().downgrade();
        let backdrop_app = cx.entity().downgrade();
        let auto_webview_app = cx.entity().downgrade();
        let agent_discovery_app = cx.entity().downgrade();
        let scan_agents_app = cx.entity().downgrade();
        let agent_scan_status = self.agent_scan_status.clone();
        let secondary_text = secondary_text_color(is_dark);
        let language_options = i18n::LanguageSetting::ALL
            .iter()
            .map(|setting| setting.display_name().to_string())
            .collect::<Vec<_>>();
        let configured_language = i18n::LanguageSetting::from_config(&self.config.general.language);
        let language_index = i18n::LanguageSetting::ALL
            .iter()
            .position(|setting| *setting == configured_language)
            .unwrap_or(0);
        let language_select =
            window.use_keyed_state("language-select-state", cx, move |window, cx| {
                let select = cx.new(|cx| {
                    SelectState::new(
                        SearchableVec::new(language_options),
                        Some(IndexPath::default().row(language_index)),
                        window,
                        cx,
                    )
                    .searchable(false)
                });
                let _subscription = cx.subscribe(
                    &select,
                    move |_, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                        let SelectEvent::Confirm(Some(display_name)) = event else {
                            return;
                        };
                        let Some(setting) = i18n::LanguageSetting::ALL
                            .iter()
                            .copied()
                            .find(|setting| setting.display_name() == display_name)
                        else {
                            return;
                        };
                        language_app
                            .update(cx, |this, cx| {
                                this.config.general.language = setting.config_value().to_string();
                                i18n::set_current_language(&this.config.general.language);
                                *this.provider_test_status.lock() = ProviderTestStatus::Idle;
                                this.budget_input_errors.clear();
                                this.save_config();
                                let active_provider = this.active_provider.read().clone();
                                update_tray_icon_for_active_provider(&active_provider, &this.data);
                                cx.notify();
                            })
                            .ok();
                    },
                );
                LanguageSelectFieldState {
                    select,
                    _subscription,
                }
            });
        let language_select = language_select.read(cx).select.clone();
        let select_bg = if is_dark {
            gpui::rgb(0x202020)
        } else {
            gpui::rgb(0xf3f3f3)
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
            SliderFieldState {
                slider,
                ready: false,
                _subscription,
            }
        });
        let refresh_slider_ready = refresh_slider.read(cx).ready;
        let refresh_slider_state = refresh_slider.clone();
        let refresh_slider = refresh_slider.read(cx).slider.clone();
        let expected_refresh_value = refresh_index as f32;
        if (refresh_slider.read(cx).value().end() - expected_refresh_value).abs() > f32::EPSILON {
            refresh_slider.update(cx, |state, cx| {
                state.set_value(expected_refresh_value, window, cx);
            });
        }
        if !refresh_slider_ready {
            let app = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                refresh_slider_state.update(cx, |state, _| state.ready = true);
                app.update(cx, |_, cx| cx.notify()).ok();
            });
            window.request_animation_frame();
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(13.0)).child(i18n::text(Text::GeneralSettings)))
            .child(
                GroupBox::new()
                    .fill()
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .items_center()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.0)).child(i18n::text(Text::Language)))
                                    .child(div().text_size(px(10.0)).text_color(secondary_text).child(i18n::text(Text::LanguageDescription)))
                            )
                            .child(
                                div().w(px(140.0)).flex_none().child(
                                    SelectPopoverSurface::new(
                                        Select::new(&language_select)
                                            .bg(select_bg)
                                            .w(px(140.0)),
                                    ),
                                ),
                            ),
                    )
                    .child(Divider::horizontal())
                    .child(
                        // Theme Select
                        div()
                            .flex()
                            .w_full()
                            .overflow_hidden()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.0)).child(i18n::text(Text::Theme)))
                                    .child(div().text_size(px(10.0)).text_color(secondary_text).child(i18n::text(Text::ThemeDescription)))
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .children(vec!["system", "dark", "light"].into_iter().enumerate().map(|(idx, t)| {
                                        let is_sel = self.config.general.theme == t;
                                        let theme_app = theme_app.clone();
                                        let label = match t {
                                            "dark" => i18n::text(Text::Dark),
                                            "light" => i18n::text(Text::Light),
                                            _ => i18n::text(Text::System),
                                        };
                                        Button::new(("theme_btn", idx))
                                            .label(label)
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
                                                    let backdrop = this.config.general.backdrop.clone();
                                                    crate::refresh_backdrop(dark, &backdrop);
                                                    apply_component_theme(t, Some(window), view_cx);
                                                    view_cx.notify();
                                                }).ok();
                                            })
                                    }))
                            )
                    )
                    .child(Divider::horizontal())
                    .child(
                        // Windows backdrop material
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.0)).child(i18n::text(Text::Backdrop)))
                                    .child(div().text_size(px(10.0)).text_color(secondary_text).child(i18n::text(Text::BackdropDescription)))
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .children([
                                        ("mica", "Mica"),
                                        ("mica_alt", "Mica Alt"),
                                        ("acrylic", "Acrylic"),
                                        ("none", i18n::text(Text::None)),
                                    ].into_iter().enumerate().map(|(idx, (value, label))| {
                                        let configured = match self.config.general.backdrop.as_str() {
                                            "mica" | "mica_alt" | "acrylic" | "none" => self.config.general.backdrop.as_str(),
                                            _ => "mica_alt",
                                        };
                                        let is_sel = configured == value;
                                        let backdrop_app = backdrop_app.clone();
                                        Button::new(("backdrop_btn", idx))
                                            .label(label)
                                            .small()
                                            .compact()
                                            .selected(is_sel)
                                            .when(is_sel, |button| button.primary())
                                            .on_click(move |_, window, cx| {
                                                backdrop_app.update(cx, |this, view_cx| {
                                                    this.config.general.backdrop = value.to_string();
                                                    this.save_config();
                                                    let dark = match this.config.general.theme.as_str() {
                                                        "dark" => true,
                                                        "light" => false,
                                                        _ => matches!(
                                                            window.appearance(),
                                                            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                                                        ),
                                                    };
                                                    crate::refresh_backdrop(dark, value);
                                                    view_cx.notify();
                                                }).ok();
                                            })
                                    }))
                            )
                    )
                    .child(Divider::horizontal())
                    .child(
                        // WebView authentication policy
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.0)).child(i18n::text(Text::AutomaticWebViewLogin)))
                                    .child(div().text_size(px(10.0)).text_color(secondary_text).child(i18n::text(Text::AutomaticWebViewLoginDescription)))
                            )
                            .child(
                                Switch::new("auto_webview_login")
                                    .checked(self.config.general.auto_webview_login)
                                    .on_click(move |checked, _window, cx| {
                                        let checked = *checked;
                                        auto_webview_app
                                            .update(cx, |this, cx| {
                                                this.config.general.auto_webview_login = checked;
                                                this.save_config();
                                                cx.notify();
                                            })
                                            .ok();
                                    })
                            )
                    )
                    .child(Divider::horizontal())
                    .child(
                        // Local agent discovery consent and manual scan.
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_size(px(12.0))
                                                    .child(i18n::text(Text::LocalAgentDiscovery)),
                                            )
                                            .child(
                                                div()
                                                    .w_full()
                                                    .truncate()
                                                    .text_size(px(10.0))
                                                    .text_color(secondary_text)
                                                    .child(i18n::text(
                                                        Text::LocalAgentDiscoveryDescription,
                                                    )),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_none()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                Button::new("scan_local_agents_btn")
                                                    .small()
                                                    .compact()
                                                    .label(i18n::text(Text::ScanNow))
                                                    .loading(matches!(
                                                        &agent_scan_status,
                                                        AgentScanStatus::Scanning
                                                    ))
                                                    .disabled(
                                                        !self.config.general.local_agent_discovery
                                                            || matches!(
                                                                &agent_scan_status,
                                                                AgentScanStatus::Scanning
                                                            ),
                                                    )
                                                    .on_click(move |_, _, cx| {
                                                        scan_agents_app
                                                            .update(cx, |this, cx| {
                                                                this.trigger_agent_scan(cx);
                                                            })
                                                            .ok();
                                                    }),
                                            )
                                            .child(
                                                Switch::new("local_agent_discovery")
                                                    .checked(
                                                        self.config
                                                            .general
                                                            .local_agent_discovery,
                                                    )
                                                    .on_click(move |checked, _window, cx| {
                                                        let checked = *checked;
                                                        agent_discovery_app
                                                            .update(cx, |this, cx| {
                                                                this.config
                                                                    .general
                                                                    .local_agent_discovery =
                                                                    checked;
                                                                this.config
                                                                    .general
                                                                    .agent_scan_onboarding_completed =
                                                                    true;
                                                                this.agent_scan_status =
                                                                    AgentScanStatus::Idle;
                                                                this.save_config();
                                                                if checked {
                                                                    this.trigger_agent_scan(cx);
                                                                } else {
                                                                    crate::tray::request_refresh();
                                                                    cx.notify();
                                                                }
                                                            })
                                                            .ok();
                                                    }),
                                            ),
                                    ),
                            )
                            .child(match agent_scan_status {
                                AgentScanStatus::Complete(detected) if detected.is_empty() => {
                                    Alert::info(
                                        "settings-agent-scan-empty",
                                        i18n::text(Text::NoReadyLocalAgents),
                                    )
                                    .small()
                                    .into_any_element()
                                }
                                AgentScanStatus::Complete(detected) => Alert::success(
                                    "settings-agent-scan-complete",
                                    i18n::enabled_agents(
                                        &detected
                                            .iter()
                                            .map(|agent| agent.display_name)
                                            .collect::<Vec<_>>()
                                            .join(", "),
                                    ),
                                )
                                .small()
                                .into_any_element(),
                                _ => div().into_any_element(),
                            }),
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
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.0)).child(i18n::text(Text::StartWithWindows)))
                                    .child(div().text_size(px(10.0)).text_color(secondary_text).child(i18n::text(Text::StartWithWindowsDescription)))
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
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.0)).child(i18n::text(Text::RefreshInterval)))
                                    .child(div().text_size(px(11.0)).child(format!("{}s", self.config.general.refresh_interval)))
                            )
                            .child(
                                Slider::new(&refresh_slider)
                                    .horizontal()
                                    .w_full()
                                    .h(px(28.0))
                                    .opacity(if refresh_slider_ready { 1.0 } else { 0.0 })
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
                        self.render_input_field(is_dark, "proxy".into(), i18n::text(Text::NetworkProxy).into(), "e.g. http://127.0.0.1:7890".into(), false, window, cx)
                    )
            )
            .into_any_element()
    }

    fn render_notification_settings(
        &self,
        is_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = &self.config.notifications;
        let enabled = settings.enabled;
        let secondary_text = secondary_text_color(is_dark);

        let master_app = cx.entity().downgrade();
        let monthly_app = cx.entity().downgrade();
        let weekly_app = cx.entity().downgrade();
        let five_hour_app = cx.entity().downgrade();
        let threshold_toggle_app = cx.entity().downgrade();
        let failure_app = cx.entity().downgrade();

        let threshold_value = settings.threshold_percent() as f32;
        let threshold_slider_app = cx.entity().downgrade();
        let threshold_slider =
            window.use_keyed_state("notification-threshold-slider", cx, move |_, cx| {
                let slider = cx.new(|_| {
                    SliderState::new()
                        .min(1.0)
                        .max(100.0)
                        .step(1.0)
                        .default_value(threshold_value)
                });
                let _subscription = cx.subscribe(&slider, move |_, _, event: &SliderEvent, cx| {
                    let SliderEvent::Change(value) = event;
                    let threshold = value.end().round().clamp(1.0, 100.0) as f64;
                    threshold_slider_app
                        .update(cx, |this, cx| {
                            if this.config.notifications.usage_threshold_percent != threshold {
                                this.config.notifications.usage_threshold_percent = threshold;
                                this.save_config();
                                cx.notify();
                            }
                        })
                        .ok();
                });
                SliderFieldState {
                    slider,
                    ready: false,
                    _subscription,
                }
            });
        let threshold_slider_ready = threshold_slider.read(cx).ready;
        let threshold_slider_state = threshold_slider.clone();
        let threshold_slider = threshold_slider.read(cx).slider.clone();
        if (threshold_slider.read(cx).value().end() - threshold_value).abs() > f32::EPSILON {
            threshold_slider.update(cx, |state, cx| {
                state.set_value(threshold_value, window, cx);
            });
        }
        if !threshold_slider_ready {
            let app = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                threshold_slider_state.update(cx, |state, _| state.ready = true);
                app.update(cx, |_, cx| cx.notify()).ok();
            });
            window.request_animation_frame();
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_size(px(13.0))
                    .child(i18n::text(Text::Notifications)),
            )
            .child(
                GroupBox::new()
                    .fill()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .w(px(205.0))
                                    .max_w(px(205.0))
                                    .flex_none()
                                    .overflow_hidden()
                                    .pr_3()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_size(px(12.0))
                                            .child(i18n::text(Text::WindowsNotifications)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.0))
                                            .text_color(secondary_text)
                                            .whitespace_normal()
                                            .child(i18n::text(
                                                Text::WindowsNotificationsDescription,
                                            )),
                                    ),
                            )
                            .child(
                                Switch::new("notifications-enabled")
                                    .flex_none()
                                    .checked(enabled)
                                    .on_click(move |checked, _, cx| {
                                        let checked = *checked;
                                        master_app
                                            .update(cx, |this, cx| {
                                                this.config.notifications.enabled = checked;
                                                this.save_config();
                                                cx.notify();
                                            })
                                            .ok();
                                    }),
                            ),
                    )
                    .child(Divider::horizontal())
                    .child(notification_switch_row(
                        i18n::text(Text::MonthlyQuotaReset),
                        i18n::text(Text::MonthlyQuotaResetDescription),
                        "notify-monthly-reset",
                        weak_text_color(is_dark),
                        settings.monthly_resets,
                        !enabled,
                        move |checked, cx| {
                            monthly_app
                                .update(cx, |this, cx| {
                                    this.config.notifications.monthly_resets = checked;
                                    this.save_config();
                                    cx.notify();
                                })
                                .ok();
                        },
                    ))
                    .child(notification_switch_row(
                        i18n::text(Text::WeeklyQuotaReset),
                        i18n::text(Text::WeeklyQuotaResetDescription),
                        "notify-weekly-reset",
                        weak_text_color(is_dark),
                        settings.weekly_resets,
                        !enabled,
                        move |checked, cx| {
                            weekly_app
                                .update(cx, |this, cx| {
                                    this.config.notifications.weekly_resets = checked;
                                    this.save_config();
                                    cx.notify();
                                })
                                .ok();
                        },
                    ))
                    .child(notification_switch_row(
                        i18n::text(Text::FiveHourQuotaReset),
                        i18n::text(Text::FiveHourQuotaResetDescription),
                        "notify-five-hour-reset",
                        weak_text_color(is_dark),
                        settings.five_hour_resets,
                        !enabled,
                        move |checked, cx| {
                            five_hour_app
                                .update(cx, |this, cx| {
                                    this.config.notifications.five_hour_resets = checked;
                                    this.save_config();
                                    cx.notify();
                                })
                                .ok();
                        },
                    ))
                    .child(Divider::horizontal())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .w_full()
                                    .overflow_hidden()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .w(px(205.0))
                                            .max_w(px(205.0))
                                            .flex_none()
                                            .overflow_hidden()
                                            .pr_3()
                                            .child(
                                                div()
                                                    .text_size(px(11.0))
                                                    .child(i18n::text(Text::UsageThreshold)),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(10.0))
                                                    .text_color(secondary_text)
                                                    .whitespace_normal()
                                                    .child(i18n::text(
                                                        Text::UsageThresholdDescription,
                                                    )),
                                            ),
                                    )
                                    .child(
                                        Switch::new("notify-usage-threshold")
                                            .flex_none()
                                            .checked(settings.usage_threshold_enabled)
                                            .disabled(!enabled)
                                            .on_click(move |checked, _, cx| {
                                                let checked = *checked;
                                                threshold_toggle_app
                                                    .update(cx, |this, cx| {
                                                        this.config
                                                            .notifications
                                                            .usage_threshold_enabled = checked;
                                                        this.save_config();
                                                        cx.notify();
                                                    })
                                                    .ok();
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        Slider::new(&threshold_slider)
                                            .horizontal()
                                            .disabled(!enabled || !settings.usage_threshold_enabled)
                                            .flex_1()
                                            .h(px(28.0))
                                            .opacity(if threshold_slider_ready {
                                                1.0
                                            } else {
                                                0.0
                                            }),
                                    )
                                    .child(
                                        div()
                                            .w(px(42.0))
                                            .text_align(TextAlign::Right)
                                            .text_size(px(11.0))
                                            .child(format!("{}%", settings.threshold_percent())),
                                    ),
                            ),
                    )
                    .child(Divider::horizontal())
                    .child(notification_switch_row(
                        i18n::text(Text::SilentRefreshFailures),
                        i18n::text(Text::SilentRefreshFailuresDescription),
                        "notify-silent-refresh-failure",
                        weak_text_color(is_dark),
                        settings.silent_refresh_failures,
                        !enabled,
                        move |checked, cx| {
                            failure_app
                                .update(cx, |this, cx| {
                                    this.config.notifications.silent_refresh_failures = checked;
                                    this.save_config();
                                    cx.notify();
                                })
                                .ok();
                        },
                    )),
            )
            .into_any_element()
    }

    fn render_provider_settings(
        &self,
        is_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let select_app = cx.entity().downgrade();

        let settings_catalog = provider_settings_catalog();
        let provider_names = settings_catalog
            .iter()
            .map(|(_, display)| display.to_string())
            .collect::<Vec<_>>();
        let selected_index = settings_catalog
            .iter()
            .position(|(id, _)| *id == self.selected_setting_provider)
            .unwrap_or(0);
        let provider_select =
            window.use_keyed_state("provider-select-state", cx, move |window, cx| {
                let select = cx.new(|cx| {
                    SelectState::new(
                        SearchableVec::new(provider_names),
                        Some(IndexPath::default().row(selected_index)),
                        window,
                        cx,
                    )
                    .searchable(true)
                });
                let _subscription = cx.subscribe(
                    &select,
                    move |_, _, event: &SelectEvent<SearchableVec<String>>, cx| {
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
        let bg_fill = if is_dark {
            gpui::rgb(0x202020)
        } else {
            gpui::rgb(0xf3f3f3)
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_size(px(13.0))
                    .child(i18n::text(Text::ProviderSettings)),
            )
            .child(
                GroupBox::new()
                    .fill()
                    .child(
                        // Provider ComboBox
                        SelectPopoverSurface::new(
                            Select::new(&provider_select)
                                .bg(bg_fill)
                                .search_placeholder(i18n::text(Text::SearchProviders))
                                .w_full(),
                        ),
                    )
                    .child(Divider::horizontal())
                    .child(
                        // Provider fields list based on selection
                        self.render_selected_provider_fields(is_dark, window, cx),
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
                    .child(i18n::text(Text::OpenConfigFile))
                    .on_click(move |_, _, _| {
                        let _ = open_config_file(config_path.as_ref());
                    }),
            )
            .child(Divider::vertical().h(px(12.0)))
            .child(
                Link::new("open_logs_link")
                    .text_size(px(11.0))
                    .child(i18n::text(Text::OpenLogs))
                    .on_click(move |_, _, _| {
                        let _ = open_folder(&crate::diagnostics::log_dir());
                    }),
            )
            .child(Divider::vertical().h(px(12.0)))
            .child(
                Link::new("create_diagnostic_report_link")
                    .text_size(px(11.0))
                    .child(i18n::text(Text::CreateDiagnosticReport))
                    .on_click(move |_, _, _| {
                        let _ = crate::diagnostics::write_diagnostic_report(
                            report_config_path.as_deref(),
                            Some(&report_history.read()),
                        );
                    }),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_input_field(
        &self,
        is_dark: bool,
        field_id: SharedString,
        label: SharedString,
        placeholder: SharedString,
        is_password: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let initial_value = config_field_value(&self.config, field_id.as_ref());
        let masked = is_password;
        let initial_placeholder = placeholder.clone();
        let app = cx.entity().downgrade();
        let subscription_field = field_id.to_string();
        let input_state = window.use_keyed_state(
            SharedString::from(format!("input-field-state-{field_id}")),
            cx,
            move |window, cx| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(initial_value)
                        .placeholder(initial_placeholder.clone())
                        .masked(masked)
                });
                let _subscription =
                    cx.subscribe(&input, move |_, input, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            let value = input.read(cx).value().to_string();
                            app.update(cx, |this, cx| {
                                let saved = set_config_field_value(
                                    &mut this.config,
                                    &subscription_field,
                                    value.clone(),
                                );
                                if saved {
                                    this.budget_input_errors.remove(&subscription_field);
                                    if let Some(provider) =
                                        subscription_field.strip_suffix("_budget")
                                        && let Some(data) =
                                            this.data.write().iter_mut().find(|data| {
                                                data.provider.eq_ignore_ascii_case(provider)
                                            })
                                    {
                                        crate::apply_provider_budget(&this.config, data);
                                    }
                                    this.save_config();
                                } else if subscription_field.ends_with("_budget") {
                                    this.budget_input_errors.insert(
                                        subscription_field.clone(),
                                        i18n::text(Text::BudgetPositive).to_string(),
                                    );
                                }
                                cx.notify();
                            })
                            .ok();
                        }
                    });

                InputFieldState {
                    input,
                    masked,
                    placeholder: initial_placeholder,
                    _subscription,
                }
            },
        );

        let input = input_state.read(cx).input.clone();
        let masked_changed = input_state.read(cx).masked != masked;
        let placeholder_changed = input_state.read(cx).placeholder != placeholder;
        if masked_changed || placeholder_changed {
            input_state.update(cx, |state, cx| {
                if masked_changed {
                    state.masked = masked;
                    state
                        .input
                        .update(cx, |input, cx| input.set_masked(masked, window, cx));
                }
                if placeholder_changed {
                    state.placeholder = placeholder.clone();
                    state.input.update(cx, |input, cx| {
                        input.set_placeholder(placeholder.clone(), window, cx);
                    });
                }
            });
        }
        let bg_fill = if is_dark {
            gpui::rgb(0x202020)
        } else {
            gpui::rgb(0xf3f3f3)
        };

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
                    .bg(bg_fill)
                    .w_full()
                    .when(is_password, |input| input.mask_toggle()),
            )
            .into_any_element()
    }

    fn render_selected_provider_fields(
        &self,
        is_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let provider_id = self.selected_setting_provider.clone();

        let mut widgets: Vec<AnyElement> = Vec::new();

        // 1. Primary provider action
        let is_primary = self
            .active_provider
            .read()
            .eq_ignore_ascii_case(&provider_id);
        let primary_provider_id = provider_id.clone();
        let primary_provider_app = cx.entity().downgrade();
        widgets.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(11.0))
                        .child(i18n::text(Text::PrimaryProvider)),
                )
                .child(
                    Button::new("set_primary_provider_btn")
                        .primary()
                        .small()
                        .disabled(is_primary)
                        .child(fluent_icon("\u{E735}", 11.0))
                        .child(if is_primary {
                            i18n::text(Text::Primary)
                        } else {
                            i18n::text(Text::SetAsPrimary)
                        })
                        .on_click(move |_, _, cx| {
                            primary_provider_app
                                .update(cx, |this, cx| {
                                    this.set_primary_provider(&primary_provider_id);
                                    cx.notify();
                                })
                                .ok();
                        }),
                )
                .into_any_element(),
        );

        // 2. Enable toggle switch
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
                .child(
                    div()
                        .text_size(px(11.0))
                        .child(i18n::text(Text::EnableProvider)),
                )
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

        // 3. Specific field editors
        match provider_id.as_str() {
            "deepseek" => {
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        "deepseek_key".into(),
                        i18n::text(Text::ApiKey).into(),
                        i18n::paste_named("DeepSeek Key").into(),
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
                        is_dark,
                        "claude_key".into(),
                        i18n::text(Text::ApiKey).into(),
                        "Claude Admin Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        "claude_session".into(),
                        i18n::text(Text::SessionKey).into(),
                        "Claude Session Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        "claude_token".into(),
                        i18n::text(Text::AccessToken).into(),
                        "Claude Access Token".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        "claude_auth".into(),
                        i18n::text(Text::AuthFilePath).into(),
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
                        is_dark,
                        "codex_auth".into(),
                        i18n::text(Text::AuthFilePath).into(),
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
                        is_dark,
                        "gemini_key".into(),
                        i18n::text(Text::ApiKey).into(),
                        i18n::paste_named("Gemini Key").into(),
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
                        is_dark,
                        "antigravity_key".into(),
                        i18n::text(Text::ApiKey).into(),
                        i18n::paste_named("Antigravity Key").into(),
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
                        is_dark,
                        "opencode_key".into(),
                        i18n::text(Text::ApiKey).into(),
                        "OpenCode Workspaces Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        "opencode_workspace".into(),
                        i18n::text(Text::WorkspaceId).into(),
                        i18n::paste_named("Workspace ID").into(),
                        false,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        "opencode_auth".into(),
                        i18n::text(Text::AuthCookie).into(),
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
                        is_dark,
                        "mimo_key".into(),
                        i18n::text(Text::ApiKey).into(),
                        "MiMo Token Key".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        "mimo_token".into(),
                        i18n::text(Text::ServiceToken).into(),
                        "MiMo Service Token".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        "mimo_cookie".into(),
                        i18n::text(Text::CookieHeader).into(),
                        "MiMo Cookie Header".into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            "bedrock" => {
                widgets.push(
                    div()
                        .text_size(px(10.0))
                        .text_color(secondary_text_color(is_dark))
                        .child(i18n::text(Text::BedrockDescription))
                        .into_any_element(),
                );
            }
            _ => {
                let key_field = SharedString::from(format!("{}_key", provider_id));
                let url_field = SharedString::from(format!("{}_url", provider_id));
                let dep_field = SharedString::from(format!("{}_dep", provider_id));
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        key_field,
                        i18n::text(Text::ApiKeyToken).into(),
                        i18n::text(Text::PasteProviderCredential).into(),
                        true,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        url_field,
                        i18n::text(Text::BaseUrl).into(),
                        "e.g. http://127.0.0.1:8000/v1".into(),
                        false,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
                widgets.push(
                    self.render_input_field(
                        is_dark,
                        dep_field,
                        i18n::text(Text::DeploymentModelName).into(),
                        "e.g. gpui-4o-mini".into(),
                        false,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
        }

        if crate::provider::supports_api_budget(&provider_id) {
            let budget_field = format!("{provider_id}_budget");
            widgets.push(
                self.render_input_field(
                    is_dark,
                    SharedString::from(budget_field.clone()),
                    i18n::text(Text::ApiBudget30d).into(),
                    i18n::text(Text::ApiBudgetPlaceholder).into(),
                    false,
                    window,
                    cx,
                )
                .into_any_element(),
            );
            widgets.push(
                div()
                    .text_size(px(10.0))
                    .text_color(secondary_text_color(is_dark))
                    .child(i18n::text(Text::BudgetDescription))
                    .into_any_element(),
            );
            if provider_id == "bedrock"
                && std::env::var("CODEXBAR_BEDROCK_BUDGET")
                    .ok()
                    .is_some_and(|value| !value.trim().is_empty())
            {
                widgets.push(
                    Alert::info(
                        "bedrock-budget-environment-override",
                        i18n::text(Text::BedrockEnvironmentOverride),
                    )
                    .small()
                    .into_any_element(),
                );
            }
            if let Some(message) = self.budget_input_errors.get(&budget_field) {
                widgets.push(
                    Alert::error(
                        SharedString::from(format!("{provider_id}-budget-error")),
                        message.clone(),
                    )
                    .small()
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
                        .label(i18n::text(Text::TestProvider))
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
                            "{} {}. {summary}",
                            i18n::text(Text::TestPassed),
                            fetched_at.with_timezone(&chrono::Local).format("%H:%M:%S")
                        ),
                    )
                    .small()
                    .into_any_element(),
                    ProviderTestStatus::Error { provider, message } if provider == provider_id => {
                        Alert::error(
                            "provider-test-error",
                            format!("{}: {message}", i18n::text(Text::TestFailed)),
                        )
                        .small()
                        .into_any_element()
                    }
                    ProviderTestStatus::Testing { provider } if provider == provider_id => {
                        Alert::info("provider-test-running", i18n::text(Text::FetchingUsage))
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

fn notification_switch_row(
    title: &'static str,
    description: &'static str,
    id: &'static str,
    description_color: Rgba,
    checked: bool,
    disabled: bool,
    on_change: impl Fn(bool, &mut App) + 'static,
) -> AnyElement {
    div()
        .flex()
        .w_full()
        .overflow_hidden()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .w(px(205.0))
                .max_w(px(205.0))
                .flex_none()
                .overflow_hidden()
                .pr_3()
                .child(div().text_size(px(11.0)).child(title))
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(description_color)
                        .whitespace_normal()
                        .child(description),
                ),
        )
        .child(
            Switch::new(id)
                .flex_none()
                .checked(checked)
                .disabled(disabled)
                .on_click(move |checked, _, cx| on_change(*checked, cx)),
        )
        .into_any_element()
}

fn fluent_icon(glyph: &'static str, size: f32) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .font_family("Segoe Fluent Icons")
        .font_weight(gpui::FontWeight::THIN)
        .text_size(px(size))
        .line_height(relative(1.0))
        .child(glyph)
        .into_any_element()
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

fn provider_catalog() -> &'static [(&'static str, &'static str)] {
    crate::provider::PROVIDER_CATALOG
}

fn provider_settings_catalog() -> Vec<(&'static str, &'static str)> {
    let mut catalog = provider_catalog().to_vec();
    catalog.sort_by_cached_key(|(id, display)| (display.to_lowercase(), *display, *id));
    catalog
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
        ("alibabatoken" | "alibaba" | "qwencloud", _) => Some("assets/provider-icons/alibaba.svg"),
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
        ("devin", _) => Some("assets/provider-icons/devin.svg"),
        ("perplexity", _) => Some("assets/provider-icons/perplexity.svg"),
        ("poe", _) => Some("assets/provider-icons/poe.svg"),
        ("chutes", _) => Some("assets/provider-icons/chutes.svg"),
        ("zed", _) => Some("assets/provider-icons/zed.svg"),
        ("litellm", _) => Some("assets/provider-icons/litellm.svg"),
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

    if let Some(provider) = field.strip_suffix("_budget") {
        return config
            .provider_budget(provider)
            .map(|budget| budget.to_string())
            .unwrap_or_default();
    }

    if let Some(provider) = field.strip_suffix("_key")
        && let Some(config) = api_key_provider_config(config, provider)
    {
        return config.api_key.clone();
    }
    if let Some(provider) = field.strip_suffix("_url")
        && let Some(config) = api_key_provider_config(config, provider)
    {
        return config.base_url.clone();
    }
    if let Some(provider) = field.strip_suffix("_dep")
        && let Some(config) = api_key_provider_config(config, provider)
    {
        return config.deployment.clone();
    }

    String::new()
}

fn set_config_field_value(
    config: &mut crate::config::AppConfig,
    field: &str,
    value: String,
) -> bool {
    if let Some(provider) = field.strip_suffix("_budget") {
        let value = value.trim();
        if value.is_empty() {
            config.set_provider_budget(provider, None);
            return true;
        }
        if let Ok(budget) = value.parse::<f64>()
            && budget.is_finite()
            && budget > 0.0
        {
            config.set_provider_budget(provider, Some(budget));
            return true;
        }
        return false;
    }

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
        if let Some(provider) = field.strip_suffix(suffix)
            && let Some(provider_config) = api_key_provider_config_mut(config, provider)
        {
            match suffix {
                "_key" => provider_config.api_key = value,
                "_url" => provider_config.base_url = value,
                "_dep" => provider_config.deployment = value,
                _ => unreachable!(),
            }
            return true;
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
        "clinepass" => Some(&config.clinepass),
        "alibaba" => Some(&config.alibaba),
        "qwencloud" => Some(&config.qwencloud),
        "devin" => Some(&config.devin),
        "manus" => Some(&config.manus),
        "zed" => Some(&config.zed),
        "perplexity" => Some(&config.perplexity),
        "sakana" => Some(&config.sakana),
        "deepinfra" => Some(&config.deepinfra),
        "commandcode" => Some(&config.commandcode),
        "qoder" => Some(&config.qoder),
        "litellm" => Some(&config.litellm),
        "poe" => Some(&config.poe),
        "chutes" => Some(&config.chutes),
        "neuralwatt" => Some(&config.neuralwatt),
        "clawrouter" => Some(&config.clawrouter),
        "longcat" => Some(&config.longcat),
        "sub2api" => Some(&config.sub2api),
        "wayfinder" => Some(&config.wayfinder),
        "zenmux" => Some(&config.zenmux),
        "aiand" => Some(&config.aiand),
        "zoommate" => Some(&config.zoommate),
        "xai" => Some(&config.xai),
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
        "clinepass" => Some(&mut config.clinepass),
        "alibaba" => Some(&mut config.alibaba),
        "qwencloud" => Some(&mut config.qwencloud),
        "devin" => Some(&mut config.devin),
        "manus" => Some(&mut config.manus),
        "zed" => Some(&mut config.zed),
        "perplexity" => Some(&mut config.perplexity),
        "sakana" => Some(&mut config.sakana),
        "deepinfra" => Some(&mut config.deepinfra),
        "commandcode" => Some(&mut config.commandcode),
        "qoder" => Some(&mut config.qoder),
        "litellm" => Some(&mut config.litellm),
        "poe" => Some(&mut config.poe),
        "chutes" => Some(&mut config.chutes),
        "neuralwatt" => Some(&mut config.neuralwatt),
        "clawrouter" => Some(&mut config.clawrouter),
        "longcat" => Some(&mut config.longcat),
        "sub2api" => Some(&mut config.sub2api),
        "wayfinder" => Some(&mut config.wayfinder),
        "zenmux" => Some(&mut config.zenmux),
        "aiand" => Some(&mut config.aiand),
        "zoommate" => Some(&mut config.zoommate),
        "xai" => Some(&mut config.xai),
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
    set_provider_enabled_value(config, provider, Some(true));
}

fn set_provider_enabled_value(
    config: &mut crate::config::AppConfig,
    provider: &str,
    enabled: Option<bool>,
) -> bool {
    match provider {
        "deepseek" => config.deepseek.enabled = enabled,
        "claude" => config.claude.enabled = enabled,
        "codex" => config.codex.enabled = enabled,
        "gemini" => config.gemini.enabled = enabled,
        "antigravity" => config.antigravity.enabled = enabled,
        "opencode" | "opencodego" => config.opencode.enabled = enabled,
        "mimo" => config.mimo.enabled = enabled,
        _ => {
            if let Some(cfg) = api_key_provider_config_mut(config, provider) {
                cfg.enabled = enabled;
            } else {
                return false;
            }
        }
    }
    true
}

fn summarize_provider_test(data: &UsageData) -> String {
    let max_percent = data.max_used_percent();
    let windows = data.windows.len();
    let credits = data.credits.as_ref().map(|credits| {
        (
            format_credits_balance(credits.balance),
            credits.currency.as_str(),
        )
    });

    if windows == 0 && data.credits.is_none() {
        i18n::text(Text::ProviderNoUsage).to_string()
    } else {
        i18n::provider_test_summary(
            windows,
            max_percent,
            credits
                .as_ref()
                .map(|(balance, currency)| (balance.as_str(), *currency)),
        )
    }
}

fn format_credits_balance(balance: f64) -> String {
    if balance.fract() == 0.0 {
        format!("{:.0}", balance)
    } else {
        format!("{:.2}", balance)
    }
}

fn usage_window_detail_text(provider: &str, window: &crate::provider::UsageWindow) -> String {
    if crate::usage_history::is_deepseek_calendar_spend_window(provider, &window.label)
        && let Some(spent) = window
            .used
            .filter(|spent| spent.is_finite() && *spent >= 0.0)
    {
        let amount = format_credits_balance(spent);
        return match window.unit.as_deref().map(str::trim) {
            Some(unit) if unit.eq_ignore_ascii_case("CNY") => format!("¥{amount}"),
            Some(unit) if unit.eq_ignore_ascii_case("USD") => format!("${amount}"),
            Some(unit) if unit.eq_ignore_ascii_case("EUR") => format!("€{amount}"),
            Some(unit) if !unit.is_empty() => format!("{amount} {unit}"),
            _ => amount,
        };
    }

    reset_time_text(window.resets_at)
}

fn format_reset_credit_expiry(expires_at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    expires_at
        .map(|expires| {
            expires
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| i18n::text(Text::NoExpiration).to_string())
}

fn reset_time_text(resets_at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    let Some(resets) = resets_at else {
        return "-".to_string();
    };

    let remaining = resets - chrono::Utc::now();
    if remaining.num_seconds() <= 0 {
        return i18n::text(Text::Resetting).to_string();
    }

    let days = remaining.num_days();
    let hours = remaining.num_hours() % 24;
    let minutes = remaining.num_minutes() % 60;

    i18n::reset_duration(days, hours, minutes)
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

fn trend_series_color(index: usize, is_dark: bool) -> gpui::Rgba {
    const LIGHT: [u32; 6] = [
        0x0078d4, // Fluent blue
        0x744da9, // Purple
        0x008272, // Teal
        0xca5010, // Orange
        0xc239b3, // Magenta
        0x498205, // Green
    ];
    const DARK: [u32; 6] = [
        0x60cdff, // Light blue
        0xb4a0ff, // Light purple
        0x4cc2a8, // Light teal
        0xffa86b, // Light orange
        0xf08bcb, // Light magenta
        0x92c353, // Light green
    ];
    let palette = if is_dark { &DARK } else { &LIGHT };
    gpui::rgb(palette[index % palette.len()])
}

fn format_histogram_percent(value: f64) -> String {
    if value < 1.0 {
        format!("{value:.1}%")
    } else {
        format!("{value:.0}%")
    }
}

fn usage_percent_color(percent: f64, is_dark: bool) -> gpui::Rgba {
    if percent >= 80.0 {
        if is_dark {
            gpui::rgb(0xf1707a)
        } else {
            gpui::rgb(0xc42b1c)
        }
    } else if percent >= 50.0 {
        if is_dark {
            gpui::rgb(0xffc800)
        } else {
            gpui::rgb(0xb37b00)
        }
    } else if is_dark {
        gpui::rgb(0x60cdff)
    } else {
        gpui::rgb(0x0078d4)
    }
}

fn format_trend_metrics(trend: &crate::usage_history::ProviderTrend) -> String {
    i18n::trend_metrics(
        trend.average_percent,
        trend.peak_percent,
        trend
            .previous_percent
            .map(|previous| trend.latest_percent - previous),
        trend.samples,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        format_histogram_percent, provider_catalog, provider_settings_catalog,
        trend_disclosure_height, trend_series_color,
    };

    #[test]
    fn provider_settings_catalog_is_sorted_by_display_name() {
        let actual = provider_settings_catalog();
        let mut expected = provider_catalog().to_vec();
        expected.sort_by_cached_key(|(id, display)| (display.to_lowercase(), *display, *id));

        assert_eq!(actual, expected);
    }

    #[test]
    fn combined_trend_height_adds_only_legend_rows_per_window() {
        let one_window = trend_disclosure_height(1);
        let three_windows = trend_disclosure_height(3);

        assert_eq!(
            three_windows - one_window,
            2.0 * super::TREND_LEGEND_ROW_HEIGHT
        );
    }

    #[test]
    fn trend_palette_is_distinct_and_wraps_stably() {
        let colors = (0..6)
            .map(|index| trend_series_color(index, false))
            .collect::<Vec<_>>();

        assert!(colors.windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(trend_series_color(0, false), trend_series_color(6, false));
        assert_ne!(trend_series_color(0, false), trend_series_color(0, true));
    }

    #[test]
    fn histogram_percent_labels_preserve_sub_percent_values() {
        assert_eq!(format_histogram_percent(0.04), "0.0%");
        assert_eq!(format_histogram_percent(0.6), "0.6%");
        assert_eq!(format_histogram_percent(11.4), "11%");
    }
}
