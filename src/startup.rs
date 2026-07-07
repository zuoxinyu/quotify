use anyhow::{Context, Result};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, PWSTR};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "Quotify";

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_run_key(access: u32) -> Result<HKEY> {
    let mut key = HKEY::default();
    let subkey = wide_null(RUN_KEY);
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            Some(0),
            PWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            windows::Win32::System::Registry::REG_SAM_FLAGS(access),
            None,
            &mut key,
            None,
        )
        .ok()
        .context("Failed to open HKCU Run key")?;
    }
    Ok(key)
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    let name = wide_null(VALUE_NAME);
    if enabled {
        let exe = std::env::current_exe().context("Failed to resolve current executable")?;
        let command = format!("\"{}\" tray", exe.display());
        let command_wide = wide_null(&command);
        let bytes = unsafe {
            std::slice::from_raw_parts(
                command_wide.as_ptr().cast::<u8>(),
                command_wide.len() * std::mem::size_of::<u16>(),
            )
        };
        let key = open_run_key(KEY_SET_VALUE.0)?;
        unsafe {
            RegSetValueExW(key, PCWSTR(name.as_ptr()), Some(0), REG_SZ, Some(bytes))
                .ok()
                .context("Failed to write startup entry")?;
            let _ = RegCloseKey(key);
        }
    } else {
        let key = open_run_key(KEY_SET_VALUE.0)?;
        unsafe {
            let _ = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
            let _ = RegCloseKey(key);
        }
    }
    Ok(())
}

pub fn is_enabled() -> Result<bool> {
    let mut key = HKEY::default();
    let subkey = wide_null(RUN_KEY);
    unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        )
        .ok()
        .context("Failed to read HKCU Run key")?;
    }

    let name = wide_null(VALUE_NAME);
    let result = unsafe {
        let mut value_type = REG_SZ;
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            None,
            None,
        )
    };

    unsafe {
        let _ = RegCloseKey(key);
    }

    Ok(result.is_ok())
}
