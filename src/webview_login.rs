use anyhow::{Context, Result, anyhow};
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
use windows::core::{PCWSTR, w};
use wry::WebViewBuilder;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum LoginMode {
    Mimo,
    OpenCode,
    Ollama,
}

const LOGIN_REQUIRED_PREFIX: &str = "WebView login required";

pub fn supports_provider(provider: &str) -> bool {
    matches!(
        provider.to_ascii_lowercase().as_str(),
        "mimo" | "opencode" | "opencodego" | "ollama"
    )
}

pub fn login_required_error(provider: &str, reason: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{LOGIN_REQUIRED_PREFIX} for {provider}: {reason}")
}

pub fn login_required_message(error: &str) -> Option<String> {
    login_required_message_for_language(error, crate::i18n::current_language())
}

fn login_required_message_for_language(
    error: &str,
    language: crate::i18n::Language,
) -> Option<String> {
    let index = error.find(LOGIN_REQUIRED_PREFIX)?;
    let details = error[index + LOGIN_REQUIRED_PREFIX.len()..].trim();
    let details = details.strip_prefix("for ").unwrap_or(details);
    let (provider, reason) = details.split_once(':').unwrap_or((details, ""));
    Some(crate::i18n::login_required_for(
        language,
        provider.trim(),
        reason.trim(),
    ))
}

pub fn login_and_store_for_provider(provider: &str, force_reauth: bool) -> Result<String> {
    login_and_store_for_provider_at(provider, force_reauth, None)
}

pub fn login_and_store_for_provider_at(
    provider: &str,
    force_reauth: bool,
    initial_url: Option<String>,
) -> Result<String> {
    let provider = provider.to_ascii_lowercase();
    let (secret_provider, secret_field, cookie) = match provider.as_str() {
        "mimo" => (
            "mimo",
            "cookie_header",
            run_login_flow(LoginMode::Mimo, force_reauth, None)?,
        ),
        "opencode" | "opencodego" => {
            let target = initial_url.or_else(stored_opencode_target_url);
            (
                "opencode",
                "auth_cookie",
                // Never clear the WebView profile's auth cookie here. The browser
                // session often remains valid after Quotify's saved copy expires.
                run_login_flow(LoginMode::OpenCode, false, target)?,
            )
        }
        "ollama" => (
            "ollama",
            "auth_cookie",
            run_login_flow(LoginMode::Ollama, force_reauth, None)?,
        ),
        _ => anyhow::bail!("Provider '{provider}' does not support WebView login"),
    };

    crate::secrets::set(secret_provider, secret_field, &cookie).with_context(|| {
        format!("Failed to store {provider} WebView credentials in Windows Credential Manager")
    })?;
    Ok(cookie)
}

pub async fn login_and_store_async(provider: &'static str, force_reauth: bool) -> Result<String> {
    login_and_store_async_at(provider, force_reauth, None).await
}

pub async fn login_and_store_async_at(
    provider: &'static str,
    force_reauth: bool,
    initial_url: Option<String>,
) -> Result<String> {
    match tokio::task::spawn_blocking(move || {
        login_and_store_for_provider_at(provider, force_reauth, initial_url)
    })
    .await
    {
        Ok(Ok(cookie)) => Ok(cookie),
        Ok(Err(err)) => Err(login_required_error(
            provider,
            format!("login was not completed: {err:#}"),
        )),
        Err(err) => Err(login_required_error(
            provider,
            format!("login task failed: {err}"),
        )),
    }
}

thread_local! {
    static WEBVIEW: RefCell<Option<wry::WebView>> = const { RefCell::new(None) };
    static TX: RefCell<Option<mpsc::Sender<String>>> = const { RefCell::new(None) };
    static TICKS: RefCell<usize> = const { RefCell::new(0) };
    static MODE: RefCell<LoginMode> = const { RefCell::new(LoginMode::Mimo) };
    static FORCE_REAUTH: RefCell<bool> = const { RefCell::new(false) };
    static OPENCODE_AUTH_CALLBACK_SEEN: RefCell<bool> = const { RefCell::new(false) };
    static OPENCODE_TARGET_URL: RefCell<Option<String>> = const { RefCell::new(None) };
    static OPENCODE_TARGET_PAGE_LOADED: RefCell<bool> = const { RefCell::new(false) };
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
                    let cookies = match mode {
                        LoginMode::OpenCode => {
                            webview.cookies_for_url("https://opencode.ai/_server")
                        }
                        _ => webview.cookies(),
                    };
                    if let Ok(cookies) = cookies {
                        let mut cookies_str = String::new();
                        let mut token_found = false;
                        let mut cookie_names: Vec<String> = Vec::new();

                        for cookie in cookies {
                            let name = cookie.name();
                            let value = cookie.value();
                            cookie_names.push(name.to_string());

                            let include_in_header = !matches!(mode, LoginMode::OpenCode)
                                || is_login_cookie(LoginMode::OpenCode, name);
                            if include_in_header {
                                if !cookies_str.is_empty() {
                                    cookies_str.push_str("; ");
                                }
                                cookies_str.push_str(&format!("{}={}", name, value));
                            }

                            match mode {
                                LoginMode::Mimo => {
                                    // MiMo often uses api-platform_serviceToken or serviceToken
                                    if name.to_lowercase().contains("servicetoken") && !value.is_empty() {
                                        tracing::info!("MiMo: Detected relevant token: {}", name);
                                        token_found = true;
                                    }
                                }
                                LoginMode::OpenCode => {
                                    // OpenCode currently accepts auth and __Host-auth.
                                    if is_login_cookie(LoginMode::OpenCode, name)
                                        && !value.is_empty()
                                    {
                                        let force_reauth =
                                            FORCE_REAUTH.with(|value| *value.borrow());
                                        let callback_seen = OPENCODE_AUTH_CALLBACK_SEEN
                                            .with(|value| *value.borrow());
                                        let target_required = OPENCODE_TARGET_URL.with(|value| {
                                            value
                                                .borrow()
                                                .as_deref()
                                                .is_some_and(|url| url.contains("/workspace/"))
                                        });
                                        let target_loaded = OPENCODE_TARGET_PAGE_LOADED
                                            .with(|value| *value.borrow());
                                        if should_accept_opencode_cookie(
                                            force_reauth,
                                            target_required,
                                            target_loaded,
                                            callback_seen,
                                        ) {
                                            tracing::info!(
                                                "OpenCode: Detected authenticated session cookie"
                                            );
                                            token_found = true;
                                        } else {
                                            tracing::debug!(
                                                "OpenCode: Ignoring auth cookie until the workspace Go page loads"
                                            );
                                        }
                                    }
                                }
                                LoginMode::Ollama => {
                                    // Ollama uses cookie named "__Host-next-auth.session-token" or similar session tokens
                                    let name_lower = name.to_lowercase();
                                    if (name_lower.contains("session") || name_lower.contains("auth")) && !value.is_empty() {
                                        tracing::info!("Ollama: Detected session/auth cookie: {}", name);
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
                                    LoginMode::Ollama => {
                                        tracing::debug!(
                                            "Ollama: Waiting for session/auth cookie. Current cookies: {:?}",
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
                                    LoginMode::Ollama => {
                                        tracing::info!("Ollama: Manual login required, showing window...");
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
    run_login_flow(LoginMode::Mimo, false, None)
}

pub fn opencode_login_and_get_cookie() -> Result<String> {
    run_login_flow(LoginMode::OpenCode, false, stored_opencode_target_url())
}

pub fn ollama_login_and_get_cookie() -> Result<String> {
    run_login_flow(LoginMode::Ollama, false, None)
}

fn is_login_cookie(mode: LoginMode, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    match mode {
        LoginMode::Mimo => name.contains("servicetoken"),
        LoginMode::OpenCode => matches!(name.as_str(), "auth" | "__host-auth"),
        LoginMode::Ollama => name.contains("session") || name.contains("auth"),
    }
}

fn login_url(mode: LoginMode, initial_url: Option<&str>) -> String {
    match mode {
        LoginMode::Mimo => "https://platform.xiaomimimo.com".to_string(),
        LoginMode::OpenCode => initial_url.unwrap_or("https://opencode.ai/").to_string(),
        LoginMode::Ollama => "https://ollama.com/signin".to_string(),
    }
}

fn cookie_scope_url(mode: LoginMode) -> &'static str {
    match mode {
        LoginMode::Mimo => "https://platform.xiaomimimo.com/",
        LoginMode::OpenCode => "https://opencode.ai/",
        LoginMode::Ollama => "https://ollama.com/",
    }
}

fn is_opencode_auth_callback_url(url: &str) -> bool {
    url.starts_with("https://opencode.ai/auth/callback")
}

fn should_accept_opencode_cookie(
    force_reauth: bool,
    target_required: bool,
    target_loaded: bool,
    callback_seen: bool,
) -> bool {
    target_loaded || (!target_required && (!force_reauth || callback_seen))
}

pub fn opencode_workspace_go_url(workspace_id: &str) -> Option<String> {
    let workspace_id = workspace_id.trim();
    if !workspace_id.starts_with("wrk_")
        || !workspace_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return None;
    }
    Some(format!("https://opencode.ai/workspace/{workspace_id}/go"))
}

fn stored_opencode_target_url() -> Option<String> {
    crate::secrets::get("opencode", "workspace_id")
        .ok()
        .flatten()
        .and_then(|workspace_id| opencode_workspace_go_url(&workspace_id))
        .or_else(|| {
            std::env::var("OPENCODE_WORKSPACE_ID")
                .ok()
                .and_then(|workspace_id| opencode_workspace_go_url(&workspace_id))
        })
}

fn run_login_flow(
    mode: LoginMode,
    force_reauth: bool,
    requested_initial_url: Option<String>,
) -> Result<String> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        unsafe {
            TICKS.with(|value| *value.borrow_mut() = 0);
            FORCE_REAUTH.with(|value| *value.borrow_mut() = force_reauth);
            OPENCODE_AUTH_CALLBACK_SEEN.with(|value| *value.borrow_mut() = false);
            OPENCODE_TARGET_URL.with(|value| *value.borrow_mut() = requested_initial_url.clone());
            OPENCODE_TARGET_PAGE_LOADED.with(|value| *value.borrow_mut() = false);

            let hinstance = GetModuleHandleW(None).unwrap_or_default();
            let class_name = match mode {
                LoginMode::Mimo => w!("QuotifyMimoLoginClass"),
                LoginMode::OpenCode => w!("QuotifyOpenCodeLoginClass"),
                LoginMode::Ollama => w!("QuotifyOllamaLoginClass"),
            };
            let provider_name = match mode {
                LoginMode::Mimo => "Xiaomi Mimo",
                LoginMode::OpenCode => "OpenCode",
                LoginMode::Ollama => "Ollama",
            };
            let title = format!(
                "{provider_name} — {}",
                crate::i18n::text(crate::i18n::Text::PleaseLoginToContinue)
            )
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();

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
                PCWSTR(title.as_ptr()),
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
                LoginMode::Ollama => data_dir.push("QuotifyOllamaWebviewData"),
            }
            let mut web_context = wry::WebContext::new(Some(data_dir));

            let url = login_url(mode, requested_initial_url.as_deref());

            let initial_url = if force_reauth {
                "about:blank"
            } else {
                url.as_str()
            };
            let webview = WebViewBuilder::new_with_web_context(&mut web_context)
                .with_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36")
                .with_url(initial_url)
                .with_devtools(true)
                .with_navigation_handler(|url| {
                    if is_opencode_auth_callback_url(&url) {
                        tracing::info!("OpenCode: Authentication callback reached");
                        OPENCODE_AUTH_CALLBACK_SEEN
                            .with(|value| *value.borrow_mut() = true);
                    }
                    true
                })
                .with_on_page_load_handler(|event, url| {
                    if matches!(event, wry::PageLoadEvent::Finished) {
                        let reached_target = OPENCODE_TARGET_URL.with(|target| {
                            target
                                .borrow()
                                .as_deref()
                                .is_some_and(|target| url.starts_with(target))
                        });
                        if reached_target {
                            tracing::info!("OpenCode: Workspace Go page loaded");
                            OPENCODE_TARGET_PAGE_LOADED
                                .with(|value| *value.borrow_mut() = true);
                        }
                    }
                })
                .with_new_window_req_handler(|new_url, _| {
                    tracing::info!("WebView requested new window for URL: {}", new_url);
                    WEBVIEW.with(|wv| {
                        if let Some(wv_ref) = wv.borrow().as_ref() {
                            let _ = wv_ref.load_url(&new_url);
                        }
                    });
                    wry::NewWindowResponse::Deny
                })
                .build(&window)
                .expect("Failed to build webview");

            if force_reauth {
                if let Ok(cookies) = webview.cookies_for_url(cookie_scope_url(mode)) {
                    for cookie in cookies {
                        if is_login_cookie(mode, cookie.name())
                            && let Err(err) = webview.delete_cookie(&cookie)
                        {
                            tracing::warn!(
                                "Failed to clear stale {:?} login cookie '{}': {err}",
                                mode,
                                cookie.name()
                            );
                        }
                    }
                }
                let _ = webview.load_url(&url);
                let _ = ShowWindow(hwnd, SW_SHOW);
            }

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
            let _webview = WEBVIEW.with(|wv| wv.borrow_mut().take());
            let _tx = TX.with(|t| t.borrow_mut().take());
            drop(_webview);
            drop(_tx);
            TICKS.with(|t| {
                *t.borrow_mut() = 0;
            });
            OPENCODE_TARGET_URL.with(|value| *value.borrow_mut() = None);
            OPENCODE_TARGET_PAGE_LOADED.with(|value| *value.borrow_mut() = false);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_supported_webview_providers() {
        assert!(supports_provider("mimo"));
        assert!(supports_provider("OpenCode"));
        assert!(supports_provider("opencodego"));
        assert!(supports_provider("OLLAMA"));
        assert!(!supports_provider("codex"));
    }

    #[test]
    fn marks_and_extracts_login_required_errors() {
        let error = login_required_error("mimo", "credentials expired").to_string();
        assert_eq!(
            login_required_message_for_language(&error, crate::i18n::Language::English).as_deref(),
            Some("WebView login required for mimo: credentials expired")
        );
        assert!(login_required_message("ordinary network error").is_none());
    }

    #[test]
    fn identifies_provider_login_cookies() {
        assert!(is_login_cookie(LoginMode::OpenCode, "auth"));
        assert!(is_login_cookie(LoginMode::OpenCode, "__Host-auth"));
        assert!(!is_login_cookie(LoginMode::OpenCode, "analytics"));
        assert!(is_login_cookie(
            LoginMode::Mimo,
            "api-platform_serviceToken"
        ));
        assert!(is_login_cookie(
            LoginMode::Ollama,
            "__Host-next-auth.session-token"
        ));
    }

    #[test]
    fn only_accepts_opencode_auth_callback_as_login_completion() {
        assert!(is_opencode_auth_callback_url(
            "https://opencode.ai/auth/callback?code=example&state=example"
        ));
        assert!(!is_opencode_auth_callback_url("https://opencode.ai/zh/go"));
        assert!(!is_opencode_auth_callback_url("https://opencode.ai/"));
    }

    #[test]
    fn opencode_login_reuses_workspace_go_page_instead_of_auth_endpoint() {
        let workspace_url = opencode_workspace_go_url("wrk_example").unwrap();
        assert_eq!(
            login_url(LoginMode::OpenCode, Some(&workspace_url)),
            "https://opencode.ai/workspace/wrk_example/go"
        );
        assert_eq!(login_url(LoginMode::OpenCode, None), "https://opencode.ai/");
        assert!(opencode_workspace_go_url("https://evil.example/").is_none());
    }

    #[test]
    fn opencode_workspace_login_waits_for_the_target_page() {
        assert!(!should_accept_opencode_cookie(false, true, false, false));
        assert!(should_accept_opencode_cookie(false, true, true, false));
        assert!(!should_accept_opencode_cookie(false, true, false, true));
        assert!(should_accept_opencode_cookie(false, false, false, false));
        assert!(should_accept_opencode_cookie(true, false, false, true));
    }
}
