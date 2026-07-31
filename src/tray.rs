use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use windows::Win32::Foundation::{ERROR_INVALID_WINDOW_HANDLE, POINT};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_RESPECT_QUIET_TIME,
    NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIN_BALLOONUSERCLICK, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyMenu, DestroyWindow, GetCursorPos, HICON, MF_SEPARATOR, MF_STRING,
    PostMessageW, RegisterClassW, SetForegroundWindow, TPM_LEFTALIGN, TPM_RIGHTBUTTON,
    TrackPopupMenu, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_LBUTTONUP,
    WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::w;

pub const WM_TRAYICON: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;
pub const WM_APP_SHOW: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 2;
pub const WM_APP_UPDATE_DATA: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 3;
pub const WM_APP_QUIT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 4;
pub const WM_APP_TOGGLE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 5;
pub const WM_APP_HIDE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 6;
pub const WM_APP_NOTIFY: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 7;

pub const IDM_SHOW: usize = 1;
pub const IDM_REFRESH: usize = 2;
pub const IDM_QUIT: usize = 3;
pub const IDM_ABOUT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeNotification {
    pub title: String,
    pub body: String,
    pub severity: NotificationSeverity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RefreshRequestOrigin {
    Automatic = 1,
    UserInitiated = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendHWND(HWND);
unsafe impl Send for SendHWND {}
unsafe impl Sync for SendHWND {}

impl SendHWND {
    pub fn new(hwnd: HWND) -> Self {
        Self(hwnd)
    }

    pub fn raw(&self) -> HWND {
        self.0
    }

    pub fn post_message(
        &self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows::core::Result<()> {
        unsafe { PostMessageW(Some(self.0), msg, wparam, lparam) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendHICON(pub HICON);
unsafe impl Send for SendHICON {}
unsafe impl Sync for SendHICON {}

pub static MAIN_HWND: OnceLock<SendHWND> = OnceLock::new();
pub static TRAY_HWND: OnceLock<SendHWND> = OnceLock::new();
static REFRESH_REQUESTED: AtomicU8 = AtomicU8::new(0);
pub static WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);
pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE_PAGE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static CURRENT_HICON: Mutex<Option<SendHICON>> = Mutex::new(None);
static CURRENT_TOOLTIP: Mutex<String> = Mutex::new(String::new());
static PENDING_NOTIFICATIONS: Mutex<VecDeque<NativeNotification>> = Mutex::new(VecDeque::new());
static REFRESH_SIGNAL: OnceLock<(Mutex<()>, Condvar)> = OnceLock::new();

fn refresh_signal() -> &'static (Mutex<()>, Condvar) {
    REFRESH_SIGNAL.get_or_init(|| (Mutex::new(()), Condvar::new()))
}

pub fn request_refresh() {
    request_refresh_with_origin(RefreshRequestOrigin::UserInitiated);
}

pub fn request_automatic_refresh() {
    request_refresh_with_origin(RefreshRequestOrigin::Automatic);
}

fn request_refresh_with_origin(origin: RefreshRequestOrigin) {
    let (lock, cvar) = refresh_signal();
    let _guard = lock.lock();
    REFRESH_REQUESTED.fetch_max(origin as u8, Ordering::SeqCst);
    cvar.notify_one();
}

pub fn take_refresh_request() -> Option<RefreshRequestOrigin> {
    match REFRESH_REQUESTED.swap(0, Ordering::SeqCst) {
        value if value == RefreshRequestOrigin::Automatic as u8 => {
            Some(RefreshRequestOrigin::Automatic)
        }
        value if value == RefreshRequestOrigin::UserInitiated as u8 => {
            Some(RefreshRequestOrigin::UserInitiated)
        }
        _ => None,
    }
}

pub fn wait_for_refresh_or_timeout(timeout: std::time::Duration) {
    let (lock, cvar) = refresh_signal();
    let mut guard = lock.lock();
    if REFRESH_REQUESTED.load(Ordering::SeqCst) == 0 {
        cvar.wait_for(&mut guard, timeout);
    }
}

pub fn send_native_notification(
    title: impl Into<String>,
    body: impl Into<String>,
    severity: NotificationSeverity,
) -> windows::core::Result<()> {
    let tray_hwnd = TRAY_HWND.get().copied().ok_or_else(|| {
        windows::core::Error::new(
            windows::core::HRESULT::from_win32(ERROR_INVALID_WINDOW_HANDLE.0),
            "Tray window is not initialized",
        )
    })?;

    PENDING_NOTIFICATIONS.lock().push_back(NativeNotification {
        title: title.into(),
        body: body.into(),
        severity,
    });
    tray_hwnd.post_message(WM_APP_NOTIFY, WPARAM(0), LPARAM(0))
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        static TASKBAR_CREATED_MSG: OnceLock<u32> = OnceLock::new();
        let taskbar_created = *TASKBAR_CREATED_MSG.get_or_init(|| {
            windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(w!("TaskbarCreated"))
        });

        if msg == taskbar_created {
            if let Some(shicon) = *CURRENT_HICON.lock() {
                let tooltip = CURRENT_TOOLTIP.lock().clone();
                if let Err(err) = register_tray_icon(hwnd, shicon.0, &tooltip) {
                    tracing::error!(
                        "Failed to re-register tray icon after Explorer restart: {err}"
                    );
                }
            }
            return LRESULT(1);
        }

        match msg {
            WM_CREATE => LRESULT(0),
            WM_TRAYICON => {
                let event = (lparam.0 as u32) & 0xFFFF;
                match event {
                    WM_LBUTTONUP | NIN_BALLOONUSERCLICK => {
                        if let Some(&shwnd) = MAIN_HWND.get() {
                            let message = if event == NIN_BALLOONUSERCLICK {
                                WM_APP_SHOW
                            } else {
                                WM_APP_TOGGLE
                            };
                            let _ = shwnd.post_message(message, WPARAM(0), LPARAM(0));
                        }
                    }
                    WM_RBUTTONUP => {
                        if let Some(&shwnd) = MAIN_HWND.get() {
                            let _ = shwnd.post_message(WM_APP_HIDE, WPARAM(0), LPARAM(0));
                        }

                        let mut pt = POINT { x: 0, y: 0 };
                        let _ = GetCursorPos(&mut pt);

                        let _ = SetForegroundWindow(hwnd);

                        if let Ok(hmenu) = CreatePopupMenu() {
                            let _ = AppendMenuW(hmenu, MF_STRING, IDM_SHOW, w!("Show Details"));
                            let _ = AppendMenuW(hmenu, MF_STRING, IDM_REFRESH, w!("Refresh Now"));
                            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, None);
                            let _ = AppendMenuW(hmenu, MF_STRING, IDM_ABOUT, w!("About"));
                            let _ = AppendMenuW(hmenu, MF_STRING, IDM_QUIT, w!("Quit"));

                            let _ = TrackPopupMenu(
                                hmenu,
                                TPM_LEFTALIGN | TPM_RIGHTBUTTON,
                                pt.x,
                                pt.y,
                                Some(0),
                                hwnd,
                                None,
                            );
                            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                            let _ = DestroyMenu(hmenu);
                        }
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_APP_NOTIFY => {
                loop {
                    let notification = PENDING_NOTIFICATIONS.lock().pop_front();
                    let Some(notification) = notification else {
                        break;
                    };
                    if let Err(err) = show_native_notification(hwnd, &notification) {
                        tracing::warn!("Failed to show native notification: {err}");
                    }
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wparam.0 & 0xFFFF;
                match id {
                    IDM_SHOW => {
                        if let Some(&shwnd) = MAIN_HWND.get() {
                            let _ = shwnd.post_message(WM_APP_SHOW, WPARAM(0), LPARAM(0));
                        }
                    }
                    IDM_REFRESH => {
                        request_refresh();
                    }
                    IDM_ABOUT => {
                        if let Some(&shwnd) = MAIN_HWND.get() {
                            let _ = shwnd.post_message(WM_APP_SHOW, WPARAM(1), LPARAM(0));
                        }
                    }
                    IDM_QUIT => {
                        QUIT_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
                        if let Some(&shwnd) = MAIN_HWND.get() {
                            let _ = shwnd.post_message(WM_APP_QUIT, WPARAM(0), LPARAM(0));
                        }
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = remove_tray_icon(hwnd);
                windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn tooltip_utf16(tooltip: &str) -> [u16; 128] {
    utf16_null_terminated(tooltip)
}

fn utf16_null_terminated<const N: usize>(value: &str) -> [u16; N] {
    let mut output = [0u16; N];
    let mut written = 0;

    for ch in value.chars() {
        if ch == '\0' {
            break;
        }

        let encoded_len = ch.len_utf16();
        if written + encoded_len >= N {
            break;
        }

        let mut encoded = [0u16; 2];
        let encoded = ch.encode_utf16(&mut encoded);
        output[written..written + encoded.len()].copy_from_slice(encoded);
        written += encoded.len();
    }

    output
}

fn show_native_notification(
    hwnd: HWND,
    notification: &NativeNotification,
) -> windows::core::Result<()> {
    let info_flags = match notification.severity {
        NotificationSeverity::Info => NIIF_INFO,
        NotificationSeverity::Warning => NIIF_WARNING,
        NotificationSeverity::Error => NIIF_ERROR,
    } | NIIF_RESPECT_QUIET_TIME;

    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_INFO,
        szInfo: utf16_null_terminated(&notification.body),
        szInfoTitle: utf16_null_terminated(&notification.title),
        dwInfoFlags: info_flags,
        ..Default::default()
    };

    unsafe {
        if Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() {
            Ok(())
        } else {
            Err(windows::core::Error::from_thread())
        }
    }
}

fn register_tray_icon(hwnd: HWND, hicon: HICON, tooltip: &str) -> windows::core::Result<()> {
    let tip_utf16 = tooltip_utf16(tooltip);

    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAYICON,
        hIcon: hicon,
        szTip: tip_utf16,
        ..Default::default()
    };

    unsafe {
        if Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
            Ok(())
        } else {
            Err(windows::core::Error::from_thread())
        }
    }
}

fn update_tray_icon(hwnd: HWND, hicon: HICON, tooltip: &str) -> windows::core::Result<()> {
    let tip_utf16 = tooltip_utf16(tooltip);

    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_TIP,
        hIcon: hicon,
        szTip: tip_utf16,
        ..Default::default()
    };

    unsafe {
        if Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() {
            Ok(())
        } else {
            Err(windows::core::Error::from_thread())
        }
    }
}

fn remove_tray_icon(hwnd: HWND) -> windows::core::Result<()> {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };

    unsafe {
        if Shell_NotifyIconW(NIM_DELETE, &nid).as_bool() {
            Ok(())
        } else {
            Err(windows::core::Error::from_thread())
        }
    }
}

pub fn create_tray_window() -> windows::core::Result<HWND> {
    unsafe {
        let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)?;
        let hinstance = windows::Win32::Foundation::HINSTANCE(instance.0);
        let class_name = w!("QuotifyTrayClass");

        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(tray_wnd_proc),
            hInstance: hinstance,
            lpszClassName: class_name,
            ..Default::default()
        };

        RegisterClassW(&wnd_class);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("Quotify Tray Controller"),
            WINDOW_STYLE(0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            Some(hinstance),
            None,
        )?;

        Ok(hwnd)
    }
}

pub struct TrayController {
    hwnd: HWND,
}

unsafe impl Send for TrayController {}
unsafe impl Sync for TrayController {}

impl TrayController {
    pub fn new() -> windows::core::Result<Self> {
        let hwnd = create_tray_window()?;
        let _ = TRAY_HWND.set(SendHWND::new(hwnd));
        Ok(Self { hwnd })
    }

    pub fn from_hwnd(hwnd: HWND) -> Self {
        Self { hwnd }
    }

    #[allow(dead_code)]
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn update_icon_with_tooltip(&self, hicon: HICON, tooltip: &str) {
        let mut current = CURRENT_HICON.lock();
        if let Some(old_shicon) = *current {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(old_shicon.0);
            }
        }
        *current = Some(SendHICON(hicon));
        *CURRENT_TOOLTIP.lock() = tooltip.to_string();

        if let Err(update_err) = update_tray_icon(self.hwnd, hicon, tooltip)
            && let Err(register_err) = register_tray_icon(self.hwnd, hicon, tooltip)
        {
            tracing::error!(
                "Failed to update tray icon ({update_err}); re-register failed: {register_err}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RefreshRequestOrigin, request_automatic_refresh, request_refresh, take_refresh_request,
        utf16_null_terminated, wait_for_refresh_or_timeout,
    };
    static REFRESH_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn user_refresh_request_takes_priority_over_automatic_request() {
        let _guard = REFRESH_TEST_LOCK.lock().unwrap();
        let _ = take_refresh_request();
        request_automatic_refresh();
        request_refresh();

        assert_eq!(
            take_refresh_request(),
            Some(RefreshRequestOrigin::UserInitiated)
        );
        assert_eq!(take_refresh_request(), None);
    }

    #[test]
    fn pending_refresh_is_observed_before_waiting() {
        let _guard = REFRESH_TEST_LOCK.lock().unwrap();
        let _ = take_refresh_request();
        request_automatic_refresh();
        let started = std::time::Instant::now();

        wait_for_refresh_or_timeout(std::time::Duration::from_secs(1));

        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        assert_eq!(
            take_refresh_request(),
            Some(RefreshRequestOrigin::Automatic)
        );
    }

    #[test]
    fn utf16_helper_truncates_and_null_terminates() {
        let encoded = utf16_null_terminated::<4>("abcd");

        assert_eq!(encoded, ['a' as u16, 'b' as u16, 'c' as u16, 0]);
    }

    #[test]
    fn utf16_helper_keeps_complete_surrogate_pair_at_boundary() {
        let encoded = utf16_null_terminated::<4>("a😀b");
        let terminator = encoded.iter().position(|unit| *unit == 0).unwrap();

        assert_eq!(String::from_utf16(&encoded[..terminator]).unwrap(), "a😀");
    }

    #[test]
    fn utf16_helper_does_not_split_surrogate_pair() {
        let encoded = utf16_null_terminated::<3>("a😀");

        assert_eq!(encoded, ['a' as u16, 0, 0]);
        assert!(String::from_utf16(&encoded[..1]).is_ok());
    }

    #[test]
    fn utf16_helper_stops_at_embedded_null() {
        let encoded = utf16_null_terminated::<8>("quota\0hidden");

        assert_eq!(String::from_utf16(&encoded[..5]).unwrap(), "quota");
        assert_eq!(encoded[5], 0);
    }
}
