use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

pub struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    pub fn acquire() -> Result<Self> {
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
