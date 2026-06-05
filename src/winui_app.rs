use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use parking_lot::RwLock;
use windows::Win32::Foundation::HWND;
use windows_core::Interface;
use windows_reactor::core::backend::{ControlKind, Prop, PropValue};
use windows_reactor::{
    App, AsyncSetState, Backdrop, BrushBinding, Color, Component, CustomElement,
    CustomElementHandle, Element, ElementExt, GridLength, ImageStretch, ProgressBar, RenderCx,
    ScrollBarVisibility, SetState, ThemeRef, Thickness, VerticalAlignment, border, button, grid,
    hstack, scroll_viewer, text_block, vstack,
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
static PROVIDER_SCROLL_OFFSET: AtomicUsize = AtomicUsize::new(0);

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

#[allow(non_snake_case)]
mod winui_svg {
    windows_core::imp::define_interface!(
        IImage,
        IImage_Vtbl,
        0x220d3d8d_66de_53a1_a215_ba9c165565ab
    );
    impl windows_core::RuntimeType for IImage {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    impl IImage {
        pub fn put_source<P0>(&self, value: P0) -> windows_core::Result<()>
        where
            P0: windows_core::Param<ImageSource>,
        {
            unsafe {
                (windows_core::Interface::vtable(self).put_Source)(
                    windows_core::Interface::as_raw(self),
                    value.param().abi(),
                )
                .ok()
            }
        }
    }

    #[repr(C)]
    #[doc(hidden)]
    pub struct IImage_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
        get_Source: usize,
        pub put_Source: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
        get_Stretch: usize,
        put_Stretch: usize,
        get_NineGrid: usize,
        put_NineGrid: usize,
    }

    windows_core::imp::define_interface!(
        IImageSource,
        IImageSource_Vtbl,
        0x6c2038f6_d6d5_55e9_9b9e_082f12dbff60
    );
    impl windows_core::RuntimeType for IImageSource {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct IImageSource_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
    }

    #[repr(transparent)]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ImageSource(windows_core::IUnknown);
    windows_core::imp::interface_hierarchy!(
        ImageSource,
        windows_core::IUnknown,
        windows_core::IInspectable
    );
    impl windows_core::RuntimeType for ImageSource {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_class::<Self, IImageSource>();
    }
    unsafe impl windows_core::Interface for ImageSource {
        type Vtable = <IImageSource as windows_core::Interface>::Vtable;
        const IID: windows_core::GUID = <IImageSource as windows_core::Interface>::IID;
    }
    impl core::ops::Deref for ImageSource {
        type Target = IImageSource;
        fn deref(&self) -> &Self::Target {
            unsafe { core::mem::transmute(self) }
        }
    }
    impl windows_core::RuntimeName for ImageSource {
        const NAME: &'static str = "Microsoft.UI.Xaml.Media.ImageSource";
    }

    windows_core::imp::define_interface!(
        IXamlReader,
        IXamlReader_Vtbl,
        0x54ce54c8_38c6_50d9_ac98_4b03eddbde9f
    );
    impl windows_core::RuntimeType for IXamlReader {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct IXamlReader_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
    }

    windows_core::imp::define_interface!(
        IXamlReaderStatics,
        IXamlReaderStatics_Vtbl,
        0x82a4cd9e_435e_5aeb_8c4f_300cece45cae
    );
    impl windows_core::RuntimeType for IXamlReaderStatics {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct IXamlReaderStatics_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
        pub Load: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
            *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
        LoadWithInitialTemplateValidation: usize,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct XamlReader(windows_core::IUnknown);
    windows_core::imp::interface_hierarchy!(
        XamlReader,
        windows_core::IUnknown,
        windows_core::IInspectable
    );
    impl XamlReader {
        pub fn load(xaml: &str) -> windows_core::Result<windows_core::IInspectable> {
            Self::statics(|this| unsafe {
                let mut result = core::mem::zeroed();
                (windows_core::Interface::vtable(this).Load)(
                    windows_core::Interface::as_raw(this),
                    core::mem::transmute_copy(&windows_core::HSTRING::from(xaml)),
                    &mut result,
                )
                .and_then(|| windows_core::Type::from_abi(result))
            })
        }

        fn statics<R, F: FnOnce(&IXamlReaderStatics) -> windows_core::Result<R>>(
            callback: F,
        ) -> windows_core::Result<R> {
            static SHARED: windows_core::imp::FactoryCache<XamlReader, IXamlReaderStatics> =
                windows_core::imp::FactoryCache::new();
            SHARED.call(callback)
        }
    }
    impl windows_core::RuntimeType for XamlReader {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_class::<Self, IXamlReader>();
    }
    unsafe impl windows_core::Interface for XamlReader {
        type Vtable = <IXamlReader as windows_core::Interface>::Vtable;
        const IID: windows_core::GUID = <IXamlReader as windows_core::Interface>::IID;
    }
    impl core::ops::Deref for XamlReader {
        type Target = IXamlReader;
        fn deref(&self) -> &Self::Target {
            unsafe { core::mem::transmute(self) }
        }
    }
    impl windows_core::RuntimeName for XamlReader {
        const NAME: &'static str = "Microsoft.UI.Xaml.Markup.XamlReader";
    }
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

pub fn scroll_provider_list(direction: i32) {
    if direction > 0 {
        PROVIDER_SCROLL_OFFSET.fetch_add(1, Ordering::SeqCst);
    } else if direction < 0 {
        let _ = PROVIDER_SCROLL_OFFSET.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |offset| {
            Some(offset.saturating_sub(1))
        });
    }
    request_rerender();
}

pub fn run_window(
    data: Arc<RwLock<Vec<UsageData>>>,
    last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    config: AppConfig,
    config_path: Option<PathBuf>,
    active_provider: Arc<RwLock<String>>,
) -> anyhow::Result<()> {
    let _bootstrap_handle = windows_reactor::bootstrap::initialize()?;
    let refresh_signal = Arc::new(WinUiRefreshSignal::new());
    let _ = GLOBAL_REFRESH_SIGNAL.set(refresh_signal.clone());
    App::new()
        .title("Quotify - WinUI Preview")
        .run_custom(move |_app| {
            let root = WinUiRoot {
                data,
                last_refresh,
                config,
                config_path,
                active_provider,
                refresh_signal: Some(refresh_signal),
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
            let hwnd = window_hwnd(host.window())?;
            unsafe {
                crate::install_winui_window_subclass(hwnd);
            }
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
            crate::install_winui_window_subclass(hwnd);
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
    let body: Element = if page == 1 {
        render_about_page()
    } else {
        let mut config = base_config.clone();
        config.general.provider_order = order.clone();
        let snapshot = data.read().clone();
        let ordered_providers = provider_display_order(&config);
        let scroll_offset = PROVIDER_SCROLL_OFFSET
            .load(Ordering::SeqCst)
            .min(ordered_providers.len().saturating_sub(1));
        let cards: Vec<Element> = ordered_providers
            .into_iter()
            .skip(scroll_offset)
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

        let controls = hstack((
            button("Up").subtle().on_click(|| scroll_provider_list(-1)),
            button("Down").subtle().on_click(|| scroll_provider_list(1)),
        ))
        .spacing(8.0);
        let list: Element = scroll_viewer(vstack(cards).spacing(8.0))
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
            .height(360.0)
            .into();

        vstack((controls, list)).spacing(8.0).into()
    };

    let header: Element = header.into();
    let content = grid((header.grid_row(0), body.grid_row(1)))
        .rows([GridLength::Auto, GridLength::Star(1.0)])
        .columns([GridLength::Star(1.0)])
        .row_spacing(12.0);

    border(content)
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
            provider_icon(&provider_id, display_name),
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

fn provider_icon(provider_id: &str, display_name: &str) -> Element {
    if let Some(path) = provider_icon_path(provider_id) {
        return Element::Custom(CustomElementHandle::new(SvgIconElement {
            uri: format!("ms-appx:///{path}"),
        }));
    }

    provider_initial(display_name)
}

#[derive(Clone, Debug, PartialEq)]
struct SvgIconElement {
    uri: String,
}

impl CustomElement for SvgIconElement {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn kind_name(&self) -> &'static str {
        "SvgIcon"
    }

    fn eq_dyn(&self, other: &dyn CustomElement) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn clone_dyn(&self) -> Box<dyn CustomElement> {
        Box::new(self.clone())
    }

    fn mount(&self, backend: &mut dyn windows_reactor::Backend) -> windows_reactor::ControlId {
        let id = backend.create(ControlKind::Image);
        backend.set_prop(id, Prop::Width, PropValue::F64(28.0));
        backend.set_prop(id, Prop::Height, PropValue::F64(28.0));
        backend.set_prop(
            id,
            Prop::ImageStretch,
            PropValue::ImageStretch(ImageStretch::Uniform),
        );
        set_native_svg_source(backend, id, &self.uri);
        id
    }

    fn update(
        &self,
        prev: &dyn CustomElement,
        id: windows_reactor::ControlId,
        backend: &mut dyn windows_reactor::Backend,
    ) {
        if prev.as_any().downcast_ref::<Self>() != Some(self) {
            set_native_svg_source(backend, id, &self.uri);
        }
    }
}

fn set_native_svg_source(
    backend: &mut dyn windows_reactor::Backend,
    id: windows_reactor::ControlId,
    uri: &str,
) {
    let Some(native) = backend.get_native_element(id) else {
        return;
    };
    let Ok(image) = native.cast::<winui_svg::IImage>() else {
        return;
    };
    let xaml = format!(
        r#"<SvgImageSource xmlns="using:Microsoft.UI.Xaml.Media.Imaging" UriSource="{uri}"/>"#
    );
    let Ok(source) =
        winui_svg::XamlReader::load(&xaml).and_then(|item| item.cast::<winui_svg::ImageSource>())
    else {
        return;
    };
    let _ = image.put_source(&source);
}

fn provider_icon_path(provider_id: &str) -> Option<&'static str> {
    let relative = match provider_id {
        "abacus" => "Assets/provider-icons/abacus-ai-dark.svg",
        "alibabatoken" => "Assets/provider-icons/alibaba.svg",
        "amp" => "Assets/provider-icons/amp.svg",
        "augment" => "Assets/provider-icons/augment.svg",
        "codex" => "Assets/provider-icons/codex.svg",
        "codebuff" => "Assets/provider-icons/codebuff.svg",
        "copilot" => "Assets/provider-icons/copilot.svg",
        "cursor" => "Assets/provider-icons/cursor.svg",
        "droid" => "Assets/provider-icons/droid.svg",
        "elevenlabs" => "Assets/provider-icons/elevenlabs.svg",
        "jetbrains" => "Assets/provider-icons/jetbrains-ai.svg",
        "kilo" => "Assets/provider-icons/kilo.svg",
        "kimi" => "Assets/provider-icons/kimi.svg",
        "kiro" => "Assets/provider-icons/kiro.svg",
        "minimax" => "Assets/provider-icons/minimax.svg",
        "mistral" => "Assets/provider-icons/mistral.svg",
        "ollama" => "Assets/provider-icons/ollama.svg",
        "opencode" | "opencodego" => "Assets/provider-icons/opencode.svg",
        "openrouter" => "Assets/provider-icons/openrouter.svg",
        "claude" => "Assets/provider-icons/claude.svg",
        "gemini" => "Assets/provider-icons/gemini.svg",
        "antigravity" => "Assets/provider-icons/antigravity.svg",
        "deepseek" => "Assets/provider-icons/deepseek.svg",
        "synthetic" => "Assets/provider-icons/synthetic.svg",
        "vertexai" => "Assets/provider-icons/vertex-ai.svg",
        "warp" => "Assets/provider-icons/warp.svg",
        "zai" => "Assets/provider-icons/zai.svg",
        _ => return None,
    };

    Some(relative)
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
