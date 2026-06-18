use anyhow::{anyhow, Result};
use std::sync::mpsc;
use std::cell::RefCell;
use wry::WebViewBuilder;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, DisplayHandle, WindowHandle, WindowsDisplayHandle, Win32WindowHandle, HandleError};
use std::num::NonZeroIsize;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, RegisterClassW,
    ShowWindow, TranslateMessage, CW_USEDEFAULT, MSG, SW_SHOW, WINDOW_EX_STYLE,
    WNDCLASSW, WS_OVERLAPPEDWINDOW, GetClientRect,
};

thread_local! {
    static WEBVIEW: RefCell<Option<wry::WebView>> = RefCell::new(None);
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
        windows::Win32::UI::WindowsAndMessaging::WM_SIZE => unsafe {
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            WEBVIEW.with(|wv| {
                if let Some(webview) = wv.borrow().as_ref() {
                    let _ = webview.set_bounds(wry::Rect {
                        position: wry::dpi::PhysicalPosition::new(0, 0).into(),
                        size: wry::dpi::PhysicalSize::new(
                            (rect.right - rect.left) as u32,
                            (rect.bottom - rect.top) as u32,
                        ).into(),
                    });
                }
            });
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => unsafe {
            windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub fn login_and_get_cookie() -> Result<String> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        unsafe {
            let hinstance = GetModuleHandleW(None).unwrap_or_default();
            let class_name = w!("QuotifyMimoLoginClass");
            
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
                w!("Xiaomi Mimo Login"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                800,
                600,
                None,
                None,
                Some(hinstance.into()),
                None,
            ).expect("Failed to create window");

            let window = RawWindow { hwnd };

            let tx_clone = tx.clone();
            
            let init_script = r#"
                setInterval(function() {
                    window.ipc.postMessage(document.cookie);
                }, 1000);
            "#;

            let webview = WebViewBuilder::new()
                .with_url("https://platform.xiaomimimo.com")
                .with_initialization_script(init_script)
                .with_ipc_handler(move |msg| {
                    let body = msg.body();
                    if body.contains("serviceToken=") {
                        let _ = tx_clone.send(body.clone());
                        // Post a close message to ourselves so the loop exits
                        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                            Some(hwnd),
                            windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                })
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
                ).into(),
            });

            WEBVIEW.with(|wv| {
                *wv.borrow_mut() = Some(webview);
            });

            let _ = ShowWindow(hwnd, SW_SHOW);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).into() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            
            // Clean up thread local
            WEBVIEW.with(|wv| {
                *wv.borrow_mut() = None;
            });
            let _ = DestroyWindow(hwnd);
        }
    });

    let res = rx.recv().unwrap_or_else(|_| "".to_string());
    if res.is_empty() {
        Err(anyhow!("Window closed before login was completed or no cookie found"))
    } else {
        Ok(res)
    }
}
