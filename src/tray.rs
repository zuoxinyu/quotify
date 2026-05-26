use parking_lot::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use windows::Win32::Foundation::POINT;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyMenu, DestroyWindow, GetCursorPos, HICON, HWND_MESSAGE, MF_SEPARATOR,
    MF_STRING, PostMessageW, RegisterClassW, SetForegroundWindow, TPM_LEFTALIGN, TPM_RIGHTBUTTON,
    TrackPopupMenu, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_LBUTTONUP,
    WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::w;

pub const WM_TRAYICON: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;
pub const WM_APP_SHOW: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 2;
pub const WM_APP_UPDATE_DATA: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 3;
pub const WM_APP_QUIT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 4;

pub const IDM_SHOW: usize = 1;
pub const IDM_REFRESH: usize = 2;
pub const IDM_QUIT: usize = 3;

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
pub static REFRESH_REQUESTED: AtomicBool = AtomicBool::new(false);
pub static WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);
static CURRENT_HICON: Mutex<Option<SendHICON>> = Mutex::new(None);

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
                let _ = register_tray_icon(hwnd, shicon.0);
            }
            return LRESULT(1);
        }

        match msg {
            WM_CREATE => LRESULT(0),
            WM_TRAYICON => {
                let event = lparam.0 as u32;
                match event {
                    WM_LBUTTONUP => {
                        if let Some(&shwnd) = MAIN_HWND.get() {
                            let _ = shwnd.post_message(WM_APP_SHOW, WPARAM(0), LPARAM(0));
                        }
                    }
                    WM_RBUTTONUP => {
                        let mut pt = POINT { x: 0, y: 0 };
                        let _ = GetCursorPos(&mut pt);

                        let _ = SetForegroundWindow(hwnd);

                        if let Ok(hmenu) = CreatePopupMenu() {
                            let _ = AppendMenuW(
                                hmenu,
                                MF_STRING,
                                IDM_SHOW,
                                w!("显示面板 (Show Details)"),
                            );
                            let _ = AppendMenuW(
                                hmenu,
                                MF_STRING,
                                IDM_REFRESH,
                                w!("立即刷新 (Refresh Now)"),
                            );
                            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, None);
                            let _ = AppendMenuW(hmenu, MF_STRING, IDM_QUIT, w!("退出 (Quit)"));

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
            WM_COMMAND => {
                let id = wparam.0 & 0xFFFF;
                match id {
                    IDM_SHOW => {
                        if let Some(&shwnd) = MAIN_HWND.get() {
                            let _ = shwnd.post_message(WM_APP_SHOW, WPARAM(0), LPARAM(0));
                        }
                    }
                    IDM_REFRESH => {
                        REFRESH_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    IDM_QUIT => {
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

fn register_tray_icon(hwnd: HWND, hicon: HICON) -> windows::core::Result<()> {
    let tooltip = "Quotify - AI Quota Monitor";
    let mut tip_utf16 = [0u16; 128];
    let encoded: Vec<u16> = tooltip.encode_utf16().collect();
    let len = encoded.len().min(127);
    tip_utf16[..len].copy_from_slice(&encoded[..len]);

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
            Err(windows::core::Error::from_win32())
        }
    }
}

fn update_tray_icon(hwnd: HWND, hicon: HICON) -> windows::core::Result<()> {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON,
        hIcon: hicon,
        ..Default::default()
    };

    unsafe {
        if Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() {
            Ok(())
        } else {
            Err(windows::core::Error::from_win32())
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
            Err(windows::core::Error::from_win32())
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
            Some(HWND_MESSAGE),
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

    #[allow(dead_code)]
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn update_icon(&self, hicon: HICON) {
        let mut current = CURRENT_HICON.lock();
        if let Some(old_shicon) = *current {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(old_shicon.0);
            }
        }
        *current = Some(SendHICON(hicon));

        if update_tray_icon(self.hwnd, hicon).is_err() {
            let _ = register_tray_icon(self.hwnd, hicon);
        }
    }
}
