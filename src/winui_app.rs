use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use parking_lot::RwLock;
use windows::Win32::Foundation::HWND;
use windows_core::Interface;
use windows_reactor::core::backend::{ControlKind, Event, EventHandler, Prop, PropValue};
use windows_reactor::core::callback::Callback;
use windows_reactor::{
    App, AsyncSetState, Backdrop, ButtonStyle, Color, ComboBox, Component, CustomElement,
    CustomElementHandle, Element, ElementExt, GridLength, ImageStretch, NumberBox, RenderCx,
    SetState, SymbolGlyph, ThemeRef, Thickness, ToggleSwitch, Tooltip, VerticalAlignment, border,
    button, grid, hstack, scroll_viewer, text_block, text_box, vstack,
};

use crate::{
    config::AppConfig,
    provider::UsageData,
    ui_model::{
        ProviderStatus, format_credits_balance, provider_catalog, provider_display_order,
        provider_status, reset_time_text,
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

#[allow(non_snake_case)]
mod winui_list {
    windows_core::imp::define_interface!(
        IItemsControl,
        IItemsControl_Vtbl,
        0xbf1ccb54_83e2_5b98_acbc_736f876c3d35
    );
    impl windows_core::RuntimeType for IItemsControl {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    impl IItemsControl {
        pub fn get_items(&self) -> windows_core::Result<windows_core::IInspectable> {
            unsafe {
                let mut result = core::mem::zeroed();
                (windows_core::Interface::vtable(self).get_Items)(
                    windows_core::Interface::as_raw(self),
                    &mut result,
                )
                .and_then(|| windows_core::Type::from_abi(result))
            }
        }
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct IItemsControl_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
        get_ItemsSource: usize,
        put_ItemsSource: usize,
        pub get_Items: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
        rest: [usize; 17],
    }

    windows_core::imp::define_interface!(
        IUIElement,
        IUIElement_Vtbl,
        0xc3c01020_320c_5cf6_9d24_d396bbfa4d8b
    );
    impl windows_core::RuntimeType for IUIElement {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    impl IUIElement {
        pub fn put_allow_drop(&self, value: bool) -> windows_core::Result<()> {
            unsafe {
                (windows_core::Interface::vtable(self).put_AllowDrop)(
                    windows_core::Interface::as_raw(self),
                    value,
                )
                .ok()
            }
        }
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct IUIElement_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
        get_DesiredSize: usize,
        get_AllowDrop: usize,
        pub put_AllowDrop:
            unsafe extern "system" fn(*mut core::ffi::c_void, bool) -> windows_core::HRESULT,
    }

    windows_core::imp::define_interface!(
        IFrameworkElement,
        IFrameworkElement_Vtbl,
        0xfe08f13d_dc6a_5495_ad44_c2d8d21863b0
    );
    impl windows_core::RuntimeType for IFrameworkElement {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    impl IFrameworkElement {
        pub fn get_name(&self) -> windows_core::Result<windows_core::HSTRING> {
            unsafe {
                let mut result = core::mem::zeroed();
                (windows_core::Interface::vtable(self).get_Name)(
                    windows_core::Interface::as_raw(self),
                    &mut result,
                )
                .map(|| result)
            }
        }
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct IFrameworkElement_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
        get_Triggers: usize,
        get_Resources: usize,
        put_Resources: usize,
        get_Tag: usize,
        put_Tag: usize,
        get_Language: usize,
        put_Language: usize,
        get_ActualWidth: usize,
        get_ActualHeight: usize,
        get_Width: usize,
        put_Width: usize,
        get_Height: usize,
        put_Height: usize,
        get_MinWidth: usize,
        put_MinWidth: usize,
        get_MaxWidth: usize,
        put_MaxWidth: usize,
        get_MinHeight: usize,
        put_MinHeight: usize,
        get_MaxHeight: usize,
        put_MaxHeight: usize,
        get_HorizontalAlignment: usize,
        put_HorizontalAlignment: usize,
        get_VerticalAlignment: usize,
        put_VerticalAlignment: usize,
        get_Margin: usize,
        put_Margin: usize,
        pub get_Name: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut windows_core::HSTRING,
        ) -> windows_core::HRESULT,
        put_Name: usize,
    }

    windows_core::imp::define_interface!(
        IListViewBase,
        IListViewBase_Vtbl,
        0x775c57ac_abce_5beb_8e34_3b8158aedd80
    );
    impl windows_core::RuntimeType for IListViewBase {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    impl IListViewBase {
        pub fn put_can_drag_items(&self, value: bool) -> windows_core::Result<()> {
            unsafe {
                (windows_core::Interface::vtable(self).put_CanDragItems)(
                    windows_core::Interface::as_raw(self),
                    value,
                )
                .ok()
            }
        }

        pub fn put_can_reorder_items(&self, value: bool) -> windows_core::Result<()> {
            unsafe {
                (windows_core::Interface::vtable(self).put_CanReorderItems)(
                    windows_core::Interface::as_raw(self),
                    value,
                )
                .ok()
            }
        }

        pub fn add_drag_items_completed<F>(&self, handler: F) -> windows_core::Result<i64>
        where
            F: Fn(windows_core::Ref<ListViewBase>, windows_core::Ref<DragItemsCompletedEventArgs>)
                + 'static,
        {
            unsafe {
                let handler: TypedEventHandler<ListViewBase, DragItemsCompletedEventArgs> = {
                    let com = Box::new(windows_core::imp::DelegateBox::<
                        TypedEventHandler<ListViewBase, DragItemsCompletedEventArgs>,
                        F,
                    >::new(
                        &TypedEventHandlerBox::<
                            ListViewBase,
                            DragItemsCompletedEventArgs,
                            F,
                        >::VTABLE,
                        handler,
                    ));
                    let raw = Box::into_raw(com);
                    windows_core::Type::from_abi(raw as *mut core::ffi::c_void)?
                };
                let mut token = 0i64;
                (windows_core::Interface::vtable(self).add_DragItemsCompleted)(
                    windows_core::Interface::as_raw(self),
                    core::mem::transmute_copy(&handler),
                    &mut token,
                )
                .map(|| token)
            }
        }
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct IListViewBase_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
        get_SelectedItems: usize,
        get_SelectionMode: usize,
        put_SelectionMode: usize,
        get_IsSwipeEnabled: usize,
        put_IsSwipeEnabled: usize,
        get_CanDragItems: usize,
        pub put_CanDragItems:
            unsafe extern "system" fn(*mut core::ffi::c_void, bool) -> windows_core::HRESULT,
        get_CanReorderItems: usize,
        pub put_CanReorderItems:
            unsafe extern "system" fn(*mut core::ffi::c_void, bool) -> windows_core::HRESULT,
        get_IsItemClickEnabled: usize,
        put_IsItemClickEnabled: usize,
        get_DataFetchSize: usize,
        put_DataFetchSize: usize,
        get_IncrementalLoadingThreshold: usize,
        put_IncrementalLoadingThreshold: usize,
        get_IncrementalLoadingTrigger: usize,
        put_IncrementalLoadingTrigger: usize,
        get_ShowsScrollingPlaceholders: usize,
        put_ShowsScrollingPlaceholders: usize,
        get_ReorderMode: usize,
        put_ReorderMode: usize,
        get_SelectedRanges: usize,
        get_IsMultiSelectCheckBoxEnabled: usize,
        put_IsMultiSelectCheckBoxEnabled: usize,
        get_SingleSelectionFollowsFocus: usize,
        put_SingleSelectionFollowsFocus: usize,
        add_ItemClick: usize,
        remove_ItemClick: usize,
        add_DragItemsStarting: usize,
        remove_DragItemsStarting: usize,
        pub add_DragItemsCompleted: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
            *mut i64,
        ) -> windows_core::HRESULT,
        remove_DragItemsCompleted: usize,
    }

    #[repr(transparent)]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ListViewBase(windows_core::IUnknown);
    windows_core::imp::interface_hierarchy!(
        ListViewBase,
        windows_core::IUnknown,
        windows_core::IInspectable
    );
    impl windows_core::RuntimeType for ListViewBase {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_class::<Self, IListViewBase>();
    }
    unsafe impl windows_core::Interface for ListViewBase {
        type Vtable = <IListViewBase as windows_core::Interface>::Vtable;
        const IID: windows_core::GUID = <IListViewBase as windows_core::Interface>::IID;
    }
    impl windows_core::RuntimeName for ListViewBase {
        const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ListViewBase";
    }

    windows_core::imp::define_interface!(
        IDragItemsCompletedEventArgs,
        IDragItemsCompletedEventArgs_Vtbl,
        0xc0138552_f467_5c3e_8af4_593607762844
    );
    impl windows_core::RuntimeType for IDragItemsCompletedEventArgs {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct IDragItemsCompletedEventArgs_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
    }

    #[repr(transparent)]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct DragItemsCompletedEventArgs(windows_core::IUnknown);
    windows_core::imp::interface_hierarchy!(
        DragItemsCompletedEventArgs,
        windows_core::IUnknown,
        windows_core::IInspectable
    );
    impl windows_core::RuntimeType for DragItemsCompletedEventArgs {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_class::<Self, IDragItemsCompletedEventArgs>();
    }
    unsafe impl windows_core::Interface for DragItemsCompletedEventArgs {
        type Vtable = <IDragItemsCompletedEventArgs as windows_core::Interface>::Vtable;
        const IID: windows_core::GUID =
            <IDragItemsCompletedEventArgs as windows_core::Interface>::IID;
    }
    impl windows_core::RuntimeName for DragItemsCompletedEventArgs {
        const NAME: &'static str = "Microsoft.UI.Xaml.Controls.DragItemsCompletedEventArgs";
    }

    #[repr(transparent)]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct TypedEventHandler<TSender, TResult>(
        windows_core::IUnknown,
        core::marker::PhantomData<TSender>,
        core::marker::PhantomData<TResult>,
    )
    where
        TSender: windows_core::RuntimeType + 'static,
        TResult: windows_core::RuntimeType + 'static;
    unsafe impl<
        TSender: windows_core::RuntimeType + 'static,
        TResult: windows_core::RuntimeType + 'static,
    > windows_core::Interface for TypedEventHandler<TSender, TResult>
    {
        type Vtable = TypedEventHandler_Vtbl<TSender, TResult>;
        const IID: windows_core::GUID =
            windows_core::GUID::from_signature(<Self as windows_core::RuntimeType>::SIGNATURE);
    }
    impl<TSender: windows_core::RuntimeType + 'static, TResult: windows_core::RuntimeType + 'static>
        windows_core::RuntimeType for TypedEventHandler<TSender, TResult>
    {
        const SIGNATURE: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::new()
            .push_slice(b"pinterface({9de1c534-6ae1-11e0-84e1-18a905bcc53f}")
            .push_slice(b";")
            .push_other(TSender::SIGNATURE)
            .push_slice(b";")
            .push_other(TResult::SIGNATURE)
            .push_slice(b")");
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct TypedEventHandler_Vtbl<TSender, TResult>
    where
        TSender: windows_core::RuntimeType + 'static,
        TResult: windows_core::RuntimeType + 'static,
    {
        base__: windows_core::IUnknown_Vtbl,
        Invoke: unsafe extern "system" fn(
            this: *mut core::ffi::c_void,
            sender: windows_core::AbiType<TSender>,
            args: windows_core::AbiType<TResult>,
        ) -> windows_core::HRESULT,
        TSender: core::marker::PhantomData<TSender>,
        TResult: core::marker::PhantomData<TResult>,
    }
    struct TypedEventHandlerBox<
        TSender,
        TResult,
        F: Fn(windows_core::Ref<TSender>, windows_core::Ref<TResult>) + 'static,
    >(core::marker::PhantomData<(TSender, TResult, fn() -> F)>)
    where
        TSender: windows_core::RuntimeType + 'static,
        TResult: windows_core::RuntimeType + 'static;
    impl<
        TSender: windows_core::RuntimeType + 'static,
        TResult: windows_core::RuntimeType + 'static,
        F: Fn(windows_core::Ref<TSender>, windows_core::Ref<TResult>) + 'static,
    > TypedEventHandlerBox<TSender, TResult, F>
    {
        const VTABLE: TypedEventHandler_Vtbl<TSender, TResult> =
            TypedEventHandler_Vtbl::<TSender, TResult> {
                base__:
                    windows_core::IUnknown_Vtbl {
                        QueryInterface: windows_core::imp::DelegateBox::<
                            TypedEventHandler<TSender, TResult>,
                            F,
                        >::QueryInterface,
                        AddRef: windows_core::imp::DelegateBox::<
                            TypedEventHandler<TSender, TResult>,
                            F,
                        >::AddRef,
                        Release: windows_core::imp::DelegateBox::<
                            TypedEventHandler<TSender, TResult>,
                            F,
                        >::Release,
                    },
                Invoke: Self::Invoke,
                TSender: core::marker::PhantomData::<TSender>,
                TResult: core::marker::PhantomData::<TResult>,
            };
        unsafe extern "system" fn Invoke(
            this: *mut core::ffi::c_void,
            sender: windows_core::AbiType<TSender>,
            args: windows_core::AbiType<TResult>,
        ) -> windows_core::HRESULT {
            unsafe {
                let this = &mut *(this as *mut *mut core::ffi::c_void
                    as *mut windows_core::imp::DelegateBox<TypedEventHandler<TSender, TResult>, F>);
                (this.invoke)(
                    core::mem::transmute_copy(&sender),
                    core::mem::transmute_copy(&args),
                );
                windows_core::HRESULT(0)
            }
        }
    }
}

#[allow(non_snake_case)]
mod winui_content {
    windows_core::imp::define_interface!(
        IContentControl,
        IContentControl_Vtbl,
        0x07e81761_11b2_52ae_8f8b_4d53d2b5900a
    );
    impl windows_core::RuntimeType for IContentControl {
        const SIGNATURE: windows_core::imp::ConstBuffer =
            windows_core::imp::ConstBuffer::for_interface::<Self>();
    }
    impl IContentControl {
        pub fn put_content<P0>(&self, value: P0) -> windows_core::Result<()>
        where
            P0: windows_core::Param<windows_core::IInspectable>,
        {
            unsafe {
                (windows_core::Interface::vtable(self).put_Content)(
                    windows_core::Interface::as_raw(self),
                    value.param().abi(),
                )
                .ok()
            }
        }
    }
    #[repr(C)]
    #[doc(hidden)]
    pub struct IContentControl_Vtbl {
        pub base__: windows_core::IInspectable_Vtbl,
        get_Content: usize,
        pub put_Content: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeaderAction {
    Providers,
    Settings,
    About,
    Refresh,
}

#[derive(Clone, Debug, PartialEq)]
struct FontIconButtonElement {
    glyph: &'static str,
    tooltip: String,
    action: HeaderAction,
}

impl CustomElement for FontIconButtonElement {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn kind_name(&self) -> &'static str {
        "FontIconButton"
    }

    fn eq_dyn(&self, other: &dyn CustomElement) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn clone_dyn(&self) -> Box<dyn CustomElement> {
        Box::new(self.clone())
    }

    fn mount(&self, backend: &mut dyn windows_reactor::Backend) -> windows_reactor::ControlId {
        let id = backend.create(ControlKind::Button);
        backend.set_prop(id, Prop::Width, PropValue::F64(34.0));
        backend.set_prop(id, Prop::Height, PropValue::F64(34.0));
        backend.set_prop(
            id,
            Prop::ButtonStyleVariant,
            PropValue::ButtonStyle(ButtonStyle::Subtle),
        );
        set_font_icon_button_tooltip(backend, id, &self.tooltip);
        set_font_icon_button_content(backend, id, self.glyph);
        let action = self.action;
        backend.attach_event(
            id,
            Event::Click,
            EventHandler::new(Callback::new(move |()| run_header_action(action))),
        );
        id
    }

    fn update(
        &self,
        prev: &dyn CustomElement,
        id: windows_reactor::ControlId,
        backend: &mut dyn windows_reactor::Backend,
    ) {
        let previous = prev.as_any().downcast_ref::<Self>();
        if previous != Some(self) {
            set_font_icon_button_content(backend, id, self.glyph);
        }
        if previous.is_none_or(|old| old.tooltip != self.tooltip) {
            set_font_icon_button_tooltip(backend, id, &self.tooltip);
        }
    }
}

fn font_icon_button(
    glyph: &'static str,
    tooltip: impl Into<String>,
    action: HeaderAction,
) -> Element {
    Element::Custom(CustomElementHandle::new(FontIconButtonElement {
        glyph,
        tooltip: tooltip.into(),
        action,
    }))
}

fn run_header_action(action: HeaderAction) {
    match action {
        HeaderAction::Providers => crate::tray::ACTIVE_PAGE.store(0, Ordering::SeqCst),
        HeaderAction::About => crate::tray::ACTIVE_PAGE.store(1, Ordering::SeqCst),
        HeaderAction::Settings => crate::tray::ACTIVE_PAGE.store(2, Ordering::SeqCst),
        HeaderAction::Refresh => crate::tray::request_refresh(),
    }
    request_rerender();
}

fn set_font_icon_button_tooltip(
    backend: &mut dyn windows_reactor::Backend,
    id: windows_reactor::ControlId,
    tooltip: &str,
) {
    if tooltip.is_empty() {
        backend.set_tooltip(id, None);
        return;
    }
    let tooltip = Tooltip::text(tooltip.to_string());
    backend.set_tooltip(id, Some(&tooltip));
}

fn set_font_icon_button_content(
    backend: &mut dyn windows_reactor::Backend,
    id: windows_reactor::ControlId,
    glyph: &str,
) {
    let Some(native) = backend.get_native_element(id) else {
        return;
    };
    let Ok(content_control) = native.cast::<winui_content::IContentControl>() else {
        return;
    };
    let xaml = format!(
        r#"<FontIcon xmlns="using:Microsoft.UI.Xaml.Controls" Glyph="{glyph}" FontSize="16"/>"#
    );
    if let Ok(icon) = winui_svg::XamlReader::load(&xaml) {
        let _ = content_control.put_content(&icon);
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
                crate::configure_native_winui_window(hwnd);
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
        unsafe {
            crate::configure_native_winui_window(hwnd);
        }
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
    let (order, _set_order) = cx.use_state(base_config.general.provider_order.clone());
    let (active, _set_active) = cx.use_state(active_provider.read().clone());
    let (settings_config, set_settings_config) = cx.use_state(base_config.clone());
    if let Some(signal) = refresh_signal.as_ref() {
        let (_tick, set_tick) = cx.use_async_state(signal.current());
        signal.bind(set_tick);
    }

    let last = *last_refresh.read();
    let elapsed = (chrono::Utc::now() - last).num_seconds();
    let refresh_age = if elapsed < 60 {
        format!("{}s ago", elapsed.max(0))
    } else {
        format!("{}m ago", elapsed / 60)
    };
    let page = crate::tray::ACTIVE_PAGE.load(Ordering::SeqCst);

    let navigation = hstack((
        font_icon_button("\u{E80F}", "Main", HeaderAction::Providers),
        font_icon_button("\u{E713}", "Settings", HeaderAction::Settings),
        font_icon_button("\u{E897}", "About", HeaderAction::About),
    ))
    .spacing(4.0)
    .vertical_alignment(VerticalAlignment::Center);

    let header = border(
        hstack((
            text_block("Quotify")
                .font_size(22.0)
                .bold()
                .vertical_alignment(VerticalAlignment::Center),
            navigation,
            font_icon_button(
                "\u{E72C}",
                format!("Refreshed {refresh_age}"),
                HeaderAction::Refresh,
            ),
        ))
        .spacing(10.0)
        .vertical_alignment(VerticalAlignment::Center),
    )
    .height(40.0);

    let body: Element = if page == 1 {
        render_about_page()
    } else if page == 2 {
        render_settings_page(
            settings_config.clone(),
            set_settings_config.clone(),
            config_path.clone(),
            refresh_signal.clone(),
        )
    } else {
        let mut config = base_config.clone();
        config.general.provider_order = order.clone();
        let snapshot = data.read().clone();
        let ordered_providers: Vec<_> = provider_display_order(&config)
            .into_iter()
            .filter(|(provider_id, _)| provider_enabled(&config, provider_id))
            .collect();
        let rows: Vec<ProviderListRow> = ordered_providers
            .into_iter()
            .map(|(provider_id, display_name)| {
                let provider_data = snapshot
                    .iter()
                    .find(|d| d.provider.eq_ignore_ascii_case(&provider_id));
                provider_list_row(&provider_id, display_name, provider_data, &active)
            })
            .collect();

        let list: Element = Element::Custom(CustomElementHandle::new(ProviderListElement {
            rows,
            config: base_config.clone(),
            config_path: config_path.clone(),
        }))
        .into();

        border(list).height(360.0).into()
    };

    let content = vstack((header, body)).spacing(10.0);

    border(content)
        .padding(Thickness::uniform(14.0))
        .background(ThemeRef::LayerFill)
        .into()
}

#[derive(Clone, Debug, PartialEq)]
struct ProviderListRow {
    id: String,
    name: &'static str,
    status: ProviderStatus,
    active: bool,
    summary: String,
    details: Vec<String>,
    progress: Vec<ProviderProgressRow>,
}

#[derive(Clone, Debug, PartialEq)]
struct ProviderProgressRow {
    label: String,
    value: f64,
    reset: String,
}

fn provider_list_row(
    provider_id: &str,
    display_name: &'static str,
    data: Option<&UsageData>,
    active: &str,
) -> ProviderListRow {
    let status = provider_status(data);
    let mut details = Vec::new();
    if let Some(credits) = data.and_then(|d| d.credits.as_ref()) {
        details.push(format!(
            "Credits: {} {}",
            format_credits_balance(credits.balance),
            credits.currency
        ));
    }
    if let Some(error) = data.and_then(|d| d.error.clone()) {
        details.push(error);
    }
    let progress = data
        .map(|usage| {
            usage
                .windows
                .iter()
                .map(|window| ProviderProgressRow {
                    label: window.label.clone(),
                    value: window.used_percent.clamp(0.0, 100.0),
                    reset: reset_time_text(window.resets_at),
                })
                .collect()
        })
        .unwrap_or_default();

    ProviderListRow {
        id: provider_id.to_string(),
        name: display_name,
        status,
        active: active.eq_ignore_ascii_case(provider_id),
        summary: match status {
            ProviderStatus::Active => "ACTIVE",
            ProviderStatus::Error => "ERROR",
            ProviderStatus::Disabled => "OFFLINE",
        }
        .to_string(),
        details,
        progress,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ProviderListElement {
    rows: Vec<ProviderListRow>,
    config: AppConfig,
    config_path: Option<PathBuf>,
}

impl CustomElement for ProviderListElement {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn kind_name(&self) -> &'static str {
        "ProviderListView"
    }

    fn eq_dyn(&self, other: &dyn CustomElement) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn clone_dyn(&self) -> Box<dyn CustomElement> {
        Box::new(self.clone())
    }

    fn mount(&self, backend: &mut dyn windows_reactor::Backend) -> windows_reactor::ControlId {
        let id = backend.create(ControlKind::ListView);
        backend.set_prop(id, Prop::Height, PropValue::F64(360.0));
        configure_provider_list(backend, id, &self.rows);
        attach_provider_reorder_handler(backend, id, self.config.clone(), self.config_path.clone());
        id
    }

    fn update(
        &self,
        prev: &dyn CustomElement,
        id: windows_reactor::ControlId,
        backend: &mut dyn windows_reactor::Backend,
    ) {
        if prev.as_any().downcast_ref::<Self>() != Some(self) {
            configure_provider_list(backend, id, &self.rows);
        }
    }
}

fn configure_provider_list(
    backend: &mut dyn windows_reactor::Backend,
    id: windows_reactor::ControlId,
    rows: &[ProviderListRow],
) {
    let Some(native) = backend.get_native_element(id) else {
        return;
    };
    if let Ok(list) = native.cast::<winui_list::IListViewBase>() {
        let _ = list.put_can_drag_items(true);
        let _ = list.put_can_reorder_items(true);
    }
    if let Ok(ui) = native.cast::<winui_list::IUIElement>() {
        let _ = ui.put_allow_drop(true);
    }
    let Ok(items_control) = native.cast::<winui_list::IItemsControl>() else {
        return;
    };
    let Ok(items) = items_control
        .get_items()
        .and_then(|items| items.cast::<windows_collections::IVector<windows_core::IInspectable>>())
    else {
        return;
    };
    let _ = items.Clear();
    for row in rows {
        if let Ok(item) = load_provider_list_item(row) {
            let _ = items.Append(&item);
        }
    }
}

fn attach_provider_reorder_handler(
    backend: &mut dyn windows_reactor::Backend,
    id: windows_reactor::ControlId,
    config: AppConfig,
    config_path: Option<PathBuf>,
) {
    let Some(native) = backend.get_native_element(id) else {
        return;
    };
    let Ok(list) = native.cast::<winui_list::IListViewBase>() else {
        return;
    };
    let native_for_order = native.clone();
    let _ = list.add_drag_items_completed(move |_sender, _args| {
        let Some(order) = provider_order_from_native_list(&native_for_order) else {
            return;
        };
        if order.is_empty() {
            return;
        }
        save_visible_provider_order(&order, config.clone(), config_path.clone());
    });
}

fn provider_order_from_native_list(native: &windows_core::IInspectable) -> Option<Vec<String>> {
    let items_control = native.cast::<winui_list::IItemsControl>().ok()?;
    let items = items_control
        .get_items()
        .ok()?
        .cast::<windows_collections::IVector<windows_core::IInspectable>>()
        .ok()?;
    let size = items.Size().ok()?;
    let mut order = Vec::with_capacity(size as usize);
    for index in 0..size {
        let item = items.GetAt(index).ok()?;
        let framework = item.cast::<winui_list::IFrameworkElement>().ok()?;
        let name = framework.get_name().ok()?.to_string_lossy();
        if !name.is_empty() {
            order.push(name);
        }
    }
    Some(order)
}

fn load_provider_list_item(
    row: &ProviderListRow,
) -> windows_core::Result<windows_core::IInspectable> {
    winui_svg::XamlReader::load(&provider_list_item_xaml(row))
}

fn provider_list_item_xaml(row: &ProviderListRow) -> String {
    let (tag_bg, tag_fg) = match row.status {
        ProviderStatus::Active => ("#E1F4E5", "#107C41"),
        ProviderStatus::Error => ("#FDE8E8", "#C42B1C"),
        ProviderStatus::Disabled => ("#F3F3F3", "#767676"),
    };
    let active = if row.active {
        r##"<Border Background="#E8F0FF" CornerRadius="4" Padding="6,0" Height="20" MinWidth="48">
                    <TextBlock Text="Primary" FontSize="9" FontWeight="SemiBold" HorizontalAlignment="Center" VerticalAlignment="Center" TextAlignment="Center"/>
                </Border>"##
    } else {
        ""
    };
    let icon = if let Some(path) = provider_icon_path(&row.id) {
        format!(
            r#"<Grid Width="28" Height="28"><Image Source="ms-appx:///{path}" Width="28" Height="28" Stretch="Uniform" HorizontalAlignment="Center" VerticalAlignment="Center"/></Grid>"#
        )
    } else {
        format!(
            r##"<Border Width="28" Height="28" CornerRadius="14" Background="#E8F0FF"><TextBlock Text="{}" FontSize="13" FontWeight="SemiBold" HorizontalAlignment="Center" VerticalAlignment="Center"/></Border>"##,
            xaml_escape(&row.name.chars().next().unwrap_or('?').to_string())
        )
    };
    let details = row
        .details
        .iter()
        .take(2)
        .map(|detail| {
            format!(
                r#"<TextBlock Text="{}" FontSize="12" Opacity="0.76" TextTrimming="CharacterEllipsis"/>"#,
                xaml_escape(detail)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let progress = if row.progress.is_empty() {
        if row.status == ProviderStatus::Error {
            String::new()
        } else {
            r#"<TextBlock Text="No active usage windows." FontSize="12" Opacity="0.62"/>"#
                .to_string()
        }
    } else {
        row.progress
            .iter()
            .take(4)
            .map(provider_progress_xaml)
            .collect::<Vec<_>>()
            .join("")
    };

    format!(
        r##"<ListViewItem
    xmlns="using:Microsoft.UI.Xaml.Controls"
    xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
    Name="{}"
    AllowDrop="True">
    <Border Background="#80FFFFFF" BorderBrush="#26000000" BorderThickness="1" CornerRadius="8" Padding="12" Margin="0,4">
        <StackPanel Spacing="8">
            <StackPanel Orientation="Horizontal" Spacing="8" VerticalAlignment="Center">
                {icon}
                <TextBlock Text="{}" FontSize="15" FontWeight="SemiBold" VerticalAlignment="Center"/>
                <Border Background="{tag_bg}" CornerRadius="4" Padding="6,0" Height="20" MinWidth="46">
                    <TextBlock Text="{}" Foreground="{tag_fg}" FontSize="9" FontWeight="SemiBold" HorizontalAlignment="Center" VerticalAlignment="Center" TextAlignment="Center"/>
                </Border>
                {active}
            </StackPanel>
            <StackPanel Spacing="4">{details}{progress}</StackPanel>
        </StackPanel>
    </Border>
</ListViewItem>"##,
        xaml_escape(&row.id),
        xaml_escape(row.name),
        xaml_escape(&row.summary),
    )
}

fn provider_progress_xaml(row: &ProviderProgressRow) -> String {
    let value = row.value.clamp(0.0, 100.0);
    format!(
        r#"<Grid ColumnSpacing="8">
            <Grid.ColumnDefinitions>
                <ColumnDefinition Width="96"/>
                <ColumnDefinition Width="*"/>
                <ColumnDefinition Width="44"/>
                <ColumnDefinition Width="72"/>
            </Grid.ColumnDefinitions>
            <TextBlock Grid.Column="0" Text="{}" FontSize="12" FontWeight="SemiBold" TextTrimming="CharacterEllipsis"/>
            <ProgressBar Grid.Column="1" Minimum="0" Maximum="100" Value="{value:.0}" Height="4" VerticalAlignment="Center"/>
            <TextBlock Grid.Column="2" Text="{value:.0}%" FontSize="12" FontWeight="SemiBold"/>
            <TextBlock Grid.Column="3" Text="{}" FontSize="12" Opacity="0.65" TextTrimming="CharacterEllipsis"/>
        </Grid>"#,
        xaml_escape(&row.label),
        xaml_escape(&row.reset)
    )
}

fn xaml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
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

fn render_settings_page(
    config: AppConfig,
    set_config: SetState<AppConfig>,
    config_path: Option<PathBuf>,
    refresh_signal: Option<Arc<WinUiRefreshSignal>>,
) -> Element {
    let theme_items = vec![
        "System".to_string(),
        "Light".to_string(),
        "Dark".to_string(),
    ];
    let selected_theme = match config.general.theme.to_ascii_lowercase().as_str() {
        "light" => 1,
        "dark" => 2,
        _ => 0,
    };
    let mut provider_rows = Vec::new();
    for (id, display_name) in provider_catalog() {
        let enabled = provider_enabled(&config, id);
        let id_string = (*id).to_string();
        let update_config = config.clone();
        let update_path = config_path.clone();
        let update_set = set_config.clone();
        let update_signal = refresh_signal.clone();
        let toggle: Element = ToggleSwitch::new(enabled)
            .on_content("On")
            .off_content("Off")
            .on_changed(move |value| {
                let mut next = update_config.clone();
                set_provider_enabled(&mut next, &id_string, value);
                save_config(next.clone(), update_path.clone());
                update_set.call(next);
                if let Some(signal) = update_signal.as_ref() {
                    signal.notify();
                }
            })
            .into();
        provider_rows.push(
            hstack((
                provider_icon(id, display_name),
                text_block(*display_name).font_size(13.0).bold(),
                toggle,
            ))
            .spacing(8.0)
            .vertical_alignment(VerticalAlignment::Center)
            .into(),
        );
    }

    let interval_config = config.clone();
    let interval_path = config_path.clone();
    let interval_set = set_config.clone();
    let interval_signal = refresh_signal.clone();
    let proxy_config = config.clone();
    let proxy_path = config_path.clone();
    let proxy_set = set_config.clone();
    let proxy_signal = refresh_signal.clone();
    let theme_config = config.clone();
    let theme_path = config_path.clone();
    let theme_set = set_config.clone();
    let theme_signal = refresh_signal.clone();
    let open_path = config_path.clone();

    let provider_list: Element = scroll_viewer(vstack(provider_rows).spacing(6.0))
        .height(210.0)
        .into();

    border(
        vstack((
            text_block("Settings").font_size(20.0).bold(),
            grid((
                NumberBox::new(config.general.refresh_interval as f64)
                    .range(30.0, 86400.0)
                    .header("Refresh interval seconds")
                    .on_value_changed(move |value| {
                        let mut next = interval_config.clone();
                        next.general.refresh_interval = value.round().max(30.0) as u64;
                        save_config(next.clone(), interval_path.clone());
                        interval_set.call(next);
                        if let Some(signal) = interval_signal.as_ref() {
                            signal.notify();
                        }
                    })
                    .grid_column(0),
                ComboBox::new(theme_items)
                    .selected_index(selected_theme)
                    .header("Theme")
                    .on_selection_changed(move |index| {
                        let mut next = theme_config.clone();
                        next.general.theme = match index {
                            1 => "light",
                            2 => "dark",
                            _ => "",
                        }
                        .to_string();
                        save_config(next.clone(), theme_path.clone());
                        theme_set.call(next);
                        if let Some(signal) = theme_signal.as_ref() {
                            signal.notify();
                        }
                    })
                    .grid_column(1),
            ))
            .columns([GridLength::Star(1.0), GridLength::Star(1.0)])
            .column_spacing(10.0),
            text_box(config.network.proxy.clone())
                .header("Network proxy")
                .placeholder("http://127.0.0.1:7890")
                .on_changed(move |value| {
                    let mut next = proxy_config.clone();
                    next.network.proxy = value;
                    save_config(next.clone(), proxy_path.clone());
                    proxy_set.call(next);
                    if let Some(signal) = proxy_signal.as_ref() {
                        signal.notify();
                    }
                }),
            text_block("Providers").font_size(14.0).bold(),
            provider_list,
            button("Open config file")
                .icon(SymbolGlyph::Setting)
                .subtle()
                .on_click(move || {
                    if let Some(path) = open_path.as_ref() {
                        let _ = std::process::Command::new("notepad").arg(path).spawn();
                    }
                }),
        ))
        .spacing(10.0),
    )
    .background(ThemeRef::CardBackground)
    .border_brush(ThemeRef::CardStroke)
    .border_thickness(Thickness::uniform(1.0))
    .corner_radius(8.0)
    .padding(Thickness::uniform(14.0))
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

fn save_provider_order(order: &[String], mut config: AppConfig, config_path: Option<PathBuf>) {
    config.general.provider_order = order.to_vec();
    save_config(config, config_path);
}

fn save_visible_provider_order(
    visible_order: &[String],
    config: AppConfig,
    config_path: Option<PathBuf>,
) {
    let mut merged = visible_order.to_vec();
    for (id, _) in provider_display_order(&config) {
        if !merged
            .iter()
            .any(|visible| visible.eq_ignore_ascii_case(&id))
        {
            merged.push(id);
        }
    }
    save_provider_order(&merged, config, config_path);
}

fn provider_enabled(config: &AppConfig, id: &str) -> bool {
    match id {
        "deepseek" => config.deepseek.enabled,
        "claude" => config.claude.enabled,
        "codex" => config.codex.enabled,
        "gemini" => config.gemini.enabled,
        "opencode" | "opencodego" => config.opencode.enabled,
        "mimo" => config.mimo.enabled,
        "antigravity" => config.antigravity.enabled,
        "openrouter" => config.openrouter.enabled,
        "openai" => config.openai.enabled,
        "moonshot" => config.moonshot.enabled,
        "elevenlabs" => config.elevenlabs.enabled,
        "doubao" => config.doubao.enabled,
        "zai" => config.zai.enabled,
        "venice" => config.venice.enabled,
        "crof" => config.crof.enabled,
        "synthetic" => config.synthetic.enabled,
        "warp" => config.warp.enabled,
        "groqcloud" => config.groqcloud.enabled,
        "deepgram" => config.deepgram.enabled,
        "llmproxy" => config.llmproxy.enabled,
        "codebuff" => config.codebuff.enabled,
        "kiro" => config.kiro.enabled,
        "copilot" => config.copilot.enabled,
        "azureopenai" => config.azureopenai.enabled,
        "ollama" => config.ollama.enabled,
        "minimax" => config.minimax.enabled,
        "jetbrains" => config.jetbrains.enabled,
        "kimi" => config.kimi.enabled,
        "kilo" => config.kilo.enabled,
        "augment" => config.augment.enabled,
        "bedrock" => config.bedrock.enabled,
        "vertexai" => config.vertexai.enabled,
        "stepfun" => config.stepfun.enabled,
        "abacus" => config.abacus.enabled,
        "alibabatoken" => config.alibabatoken.enabled,
        "t3chat" => config.t3chat.enabled,
        "amp" => config.amp.enabled,
        "mistral" => config.mistral.enabled,
        "grok" => config.grok.enabled,
        "cursor" => config.cursor.enabled,
        "droid" => config.droid.enabled,
        "windsurf" => config.windsurf.enabled,
        _ => false,
    }
}

fn set_provider_enabled(config: &mut AppConfig, id: &str, enabled: bool) {
    match id {
        "deepseek" => config.deepseek.enabled = enabled,
        "claude" => config.claude.enabled = enabled,
        "codex" => config.codex.enabled = enabled,
        "gemini" => config.gemini.enabled = enabled,
        "opencode" | "opencodego" => config.opencode.enabled = enabled,
        "mimo" => config.mimo.enabled = enabled,
        "antigravity" => config.antigravity.enabled = enabled,
        "openrouter" => config.openrouter.enabled = enabled,
        "openai" => config.openai.enabled = enabled,
        "moonshot" => config.moonshot.enabled = enabled,
        "elevenlabs" => config.elevenlabs.enabled = enabled,
        "doubao" => config.doubao.enabled = enabled,
        "zai" => config.zai.enabled = enabled,
        "venice" => config.venice.enabled = enabled,
        "crof" => config.crof.enabled = enabled,
        "synthetic" => config.synthetic.enabled = enabled,
        "warp" => config.warp.enabled = enabled,
        "groqcloud" => config.groqcloud.enabled = enabled,
        "deepgram" => config.deepgram.enabled = enabled,
        "llmproxy" => config.llmproxy.enabled = enabled,
        "codebuff" => config.codebuff.enabled = enabled,
        "kiro" => config.kiro.enabled = enabled,
        "copilot" => config.copilot.enabled = enabled,
        "azureopenai" => config.azureopenai.enabled = enabled,
        "ollama" => config.ollama.enabled = enabled,
        "minimax" => config.minimax.enabled = enabled,
        "jetbrains" => config.jetbrains.enabled = enabled,
        "kimi" => config.kimi.enabled = enabled,
        "kilo" => config.kilo.enabled = enabled,
        "augment" => config.augment.enabled = enabled,
        "bedrock" => config.bedrock.enabled = enabled,
        "vertexai" => config.vertexai.enabled = enabled,
        "stepfun" => config.stepfun.enabled = enabled,
        "abacus" => config.abacus.enabled = enabled,
        "alibabatoken" => config.alibabatoken.enabled = enabled,
        "t3chat" => config.t3chat.enabled = enabled,
        "amp" => config.amp.enabled = enabled,
        "mistral" => config.mistral.enabled = enabled,
        "grok" => config.grok.enabled = enabled,
        "cursor" => config.cursor.enabled = enabled,
        "droid" => config.droid.enabled = enabled,
        "windsurf" => config.windsurf.enabled = enabled,
        _ => {}
    }
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
