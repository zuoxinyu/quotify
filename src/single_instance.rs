use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, SetLastError, WIN32_ERROR,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

pub struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    pub fn acquire() -> Result<Self> {
        unsafe {
            SetLastError(WIN32_ERROR(0));
        }
        let handle = unsafe { CreateMutexW(None, true, w!("Local\\QuotifySingleInstance")) }
            .context("Failed to create single-instance mutex")?;

        let last_error = unsafe { GetLastError() };
        if last_error == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            bail!("Quotify is already running");
        }

        Ok(Self { handle })
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn activate_existing_instance() -> bool {
    unsafe {
        if let Ok(hwnd) = windows::Win32::UI::WindowsAndMessaging::FindWindowW(
            windows::core::w!("QuotifyTrayClass"),
            None,
        ) && !hwnd.0.is_null()
        {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(hwnd),
                windows::Win32::UI::WindowsAndMessaging::WM_COMMAND,
                windows::Win32::Foundation::WPARAM(1), // IDM_SHOW (which is 1)
                windows::Win32::Foundation::LPARAM(0),
            );
            return true;
        }
    }
    false
}
