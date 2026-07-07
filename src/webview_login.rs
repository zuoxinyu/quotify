use anyhow::{Result, anyhow};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, Win32WindowHandle, WindowHandle,
    WindowsDisplayHandle,
};
use std::cell::RefCell;
use std::num::NonZeroIsize;
use std::sync::mpsc;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetMessageW, KillTimer, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOW, SetTimer,
    ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_SIZE, WM_TIMER,
    WNDCLASSW, WS_OVERLAPPEDWINDOW,
};
use windows::core::w;
use wry::WebViewBuilder;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum LoginMode {
    Mimo,
    OpenCode,
}

thread_local! {
    static WEBVIEW: RefCell<Option<wry::WebView>> = const { RefCell::new(None) };
    static TX: RefCell<Option<mpsc::Sender<String>>> = const { RefCell::new(None) };
    static TICKS: RefCell<usize> = const { RefCell::new(0) };
    static MODE: RefCell<LoginMode> = const { RefCell::new(LoginMode::Mimo) };
}

struct RawWindow {
    hwnd: HWND,
}

impl HasWindowHandle for RawWindow {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        let hwnd_val = NonZeroIsize::new(self.hwnd.0 as isize).unwrap();
        let handle = Win32WindowHandle::new(hwnd_val);
        Ok(unsafe { WindowHandle::borrow_raw(handle.into()) })
    }
}

impl HasDisplayHandle for RawWindow {
    fn display_handle(&self) -> std::result::Result<DisplayHandle<'_>, HandleError> {
        let handle = WindowsDisplayHandle::new();
        Ok(unsafe { DisplayHandle::borrow_raw(handle.into()) })
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SIZE => unsafe {
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            WEBVIEW.with(|wv| {
                if let Some(webview) = wv.borrow().as_ref() {
                    let _ = webview.set_bounds(wry::Rect {
                        position: wry::dpi::PhysicalPosition::new(0, 0).into(),
                        size: wry::dpi::PhysicalSize::new(
                            (rect.right - rect.left) as u32,
                            (rect.bottom - rect.top) as u32,
                        )
                        .into(),
                    });
                }
            });
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        WM_TIMER => {
            let mut current_ticks = 0;
            TICKS.with(|t| {
                *t.borrow_mut() += 1;
                current_ticks = *t.borrow();
            });

            let mode = MODE.with(|m| *m.borrow());

            WEBVIEW.with(|wv| {
                if let Some(webview) = wv.borrow().as_ref() {
                    // Try to get all cookies
                    if let Ok(cookies) = webview.cookies() {
                        let mut cookies_str = String::new();
                        let mut token_found = false;
                        let mut cookie_names: Vec<String> = Vec::new();

                        for cookie in cookies {
                            let name = cookie.name();
                            let value = cookie.value();
                            cookie_names.push(name.to_string());

                            if !cookies_str.is_empty() {
                                cookies_str.push_str("; ");
                            }
                            cookies_str.push_str(&format!("{}={}", name, value));

                            match mode {
                                LoginMode::Mimo => {
                                    // MiMo often uses api-platform_serviceToken or serviceToken
                                    if name.to_lowercase().contains("servicetoken") && !value.is_empty() {
                                        tracing::info!("MiMo: Detected relevant token: {}", name);
                                        token_found = true;
                                    }
                                }
                                LoginMode::OpenCode => {
                                    // OpenCode uses cookie named "auth"
                                    if name.to_lowercase() == "auth" && !value.is_empty() {
                                        tracing::info!("OpenCode: Detected auth cookie");
                                        token_found = true;
                                    }
                                }
                            }
                        }

                        if token_found {
                            TX.with(|tx| {
                                if let Some(tx) = tx.borrow().as_ref() {
                                    let _ = tx.send(cookies_str);
                                }
                            });
                            // Close window
                            unsafe {
                                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                            }
                        } else {
                            if !cookie_names.is_empty() {
                                match mode {
                                    LoginMode::Mimo => {
                                        tracing::debug!(
                                            "MiMo: Waiting for serviceToken. Current cookies: {:?}",
                                            cookie_names
                                        );
                                    }
                                    LoginMode::OpenCode => {
                                        tracing::debug!(
                                            "OpenCode: Waiting for auth cookie. Current cookies: {:?}",
                                            cookie_names
                                        );
                                    }
                                }
                            }

                            // Show window after 3 seconds if not auto-logged in
                            if current_ticks == 3 {
                                match mode {
                                    LoginMode::Mimo => {
                                        tracing::info!("MiMo: Manual login required, showing window...");
                                    }
                                    LoginMode::OpenCode => {
                                        tracing::info!("OpenCode: Manual login required, showing window...");
                                    }
                                }
                                unsafe {
                                    let _ = ShowWindow(hwnd, SW_SHOW);
                                }
                            }
                        }
                    }
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => unsafe {
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub fn login_and_get_cookie() -> Result<String> {
    run_login_flow(LoginMode::Mimo)
}

pub fn opencode_login_and_get_cookie() -> Result<String> {
    run_login_flow(LoginMode::OpenCode)
}

fn run_login_flow(mode: LoginMode) -> Result<String> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        unsafe {
            let hinstance = GetModuleHandleW(None).unwrap_or_default();
            let class_name = match mode {
                LoginMode::Mimo => w!("QuotifyMimoLoginClass"),
                LoginMode::OpenCode => w!("QuotifyOpenCodeLoginClass"),
            };
            let title = match mode {
                LoginMode::Mimo => w!("Xiaomi Mimo Login (Please login to continue)"),
                LoginMode::OpenCode => w!("OpenCode Login (Please login to continue)"),
            };

            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: hinstance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };

            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                title,
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                900,
                700,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
            .expect("Failed to create window");

            let window = RawWindow { hwnd };

            let mut data_dir = std::env::temp_dir();
            match mode {
                LoginMode::Mimo => data_dir.push("QuotifyMimoWebviewData"),
                LoginMode::OpenCode => data_dir.push("QuotifyOpenCodeWebviewData"),
            }
            let mut web_context = wry::WebContext::new(Some(data_dir));

            let url = match mode {
                LoginMode::Mimo => "https://platform.xiaomimimo.com",
                LoginMode::OpenCode => "https://opencode.ai",
            };

            let webview = WebViewBuilder::new_with_web_context(&mut web_context)
                .with_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36")
                .with_url(url)
                .with_devtools(true)
                .build(&window)
                .expect("Failed to build webview");

            // Initial bounds setting
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let _ = webview.set_bounds(wry::Rect {
                position: wry::dpi::PhysicalPosition::new(0, 0).into(),
                size: wry::dpi::PhysicalSize::new(
                    (rect.right - rect.left) as u32,
                    (rect.bottom - rect.top) as u32,
                )
                .into(),
            });

            WEBVIEW.with(|wv| {
                *wv.borrow_mut() = Some(webview);
            });
            TX.with(|t| {
                *t.borrow_mut() = Some(tx);
            });
            MODE.with(|m| {
                *m.borrow_mut() = mode;
            });

            // Start polling timer (window starts hidden)
            let _ = SetTimer(Some(hwnd), 1, 1000, None);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // Clean up
            let _ = KillTimer(Some(hwnd), 1);
            WEBVIEW.with(|wv| {
                *wv.borrow_mut() = None;
            });
            TX.with(|t| {
                *t.borrow_mut() = None;
            });
            TICKS.with(|t| {
                *t.borrow_mut() = 0;
            });
            let _ = DestroyWindow(hwnd);
        }
    });

    let res = rx.recv().unwrap_or_else(|_| "".to_string());
    if res.is_empty() {
        Err(anyhow!(
            "Window closed before login was completed or no cookie found"
        ))
    } else {
        Ok(res)
    }
}
