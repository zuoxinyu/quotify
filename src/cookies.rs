use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::{Context, Result};
use base64::Engine;
use std::os::windows::io::FromRawHandle;
use std::path::PathBuf;

struct BrowserPaths {
    name: &'static str,
    cookie_db_paths: &'static [&'static str],
    local_state_path: &'static str,
}

const BROWSERS: &[BrowserPaths] = &[
    BrowserPaths {
        name: "Chrome",
        cookie_db_paths: &[
            r"Google\Chrome\User Data\Default\Network\Cookies",
            r"Google\Chrome\User Data\Profile 1\Network\Cookies",
            r"Google\Chrome\User Data\Profile 2\Network\Cookies",
            r"Google\Chrome\User Data\Profile 3\Network\Cookies",
        ],
        local_state_path: r"Google\Chrome\User Data\Local State",
    },
    BrowserPaths {
        name: "Edge",
        cookie_db_paths: &[
            r"Microsoft\Edge\User Data\Default\Network\Cookies",
            r"Microsoft\Edge\User Data\Profile 1\Network\Cookies",
            r"Microsoft\Edge\User Data\Profile 2\Network\Cookies",
            r"Microsoft\Edge\User Data\Profile 3\Network\Cookies",
        ],
        local_state_path: r"Microsoft\Edge\User Data\Local State",
    },
    BrowserPaths {
        name: "Brave",
        cookie_db_paths: &[
            r"BraveSoftware\Brave-Browser\User Data\Default\Network\Cookies",
            r"BraveSoftware\Brave-Browser\User Data\Profile 1\Network\Cookies",
        ],
        local_state_path: r"BraveSoftware\Brave-Browser\User Data\Local State",
    },
];

fn local_app_data() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)
}

fn get_master_key(local_state_path: &PathBuf) -> Result<[u8; 32]> {
    let content = std::fs::read_to_string(local_state_path)
        .with_context(|| format!("Failed to read {:?}", local_state_path))?;

    let json: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse Local State JSON")?;

    let encrypted_key_b64 = json
        .pointer("/os_crypt/encrypted_key")
        .and_then(|v| v.as_str())
        .context("os_crypt.encrypted_key not found in Local State")?;

    let encrypted_key = base64::engine::general_purpose::STANDARD
        .decode(encrypted_key_b64)
        .context("Failed to base64-decode encrypted key")?;

    if encrypted_key.len() < 5 || &encrypted_key[..5] != b"DPAPI" {
        anyhow::bail!("Encrypted key does not have DPAPI prefix");
    }

    let dpapi_blob = &encrypted_key[5..];
    let decrypted = decrypt_dpapi(dpapi_blob).context("DPAPI decryption failed")?;

    if decrypted.len() != 32 {
        anyhow::bail!("Decrypted key is not 32 bytes (got {})", decrypted.len());
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&decrypted);
    Ok(key)
}

fn decrypt_dpapi(data: &[u8]) -> Result<Vec<u8>> {
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};

    let data_in = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    unsafe {
        CryptUnprotectData(
            &data_in,
            None,
            None,
            None,
            None,
            windows::Win32::Security::Cryptography::CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        )
        .map_err(|e| anyhow::anyhow!("CryptUnprotectData failed: {e}"))?;
    }

    let result = unsafe {
        let slice = std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize);
        slice.to_vec()
    };

    // Note: We intentionally don't free data_out.pbData via LocalFree because
    // the windows 0.61 crate doesn't expose it. The memory is small and will be
    // freed when the process exits.

    Ok(result)
}

fn decrypt_aes_gcm(encrypted: &[u8], key: &[u8; 32]) -> Result<String> {
    if encrypted.len() < 15 {
        anyhow::bail!("Encrypted value too short");
    }

    let version = encrypted[0];
    match version {
        b'v' => {
            // v10 (Chrome 80-126) or v20 (Chrome 127+, app-bound)
            // Format: "v10" or "v20" + 1 byte separator (0x01) + 12-byte nonce + ciphertext + 16-byte tag
            let nonce_start = 3; // "v10" is 3 bytes, then nonce starts
            if encrypted.len() < nonce_start + 12 + 16 {
                anyhow::bail!("v10 encrypted value too short");
            }

            // Check for v20 (app-bound encryption) - we can't decrypt these yet
            if encrypted[1] == b'2' && encrypted[2] == b'0' {
                anyhow::bail!(
                    "v20 (app-bound) encrypted cookies are not supported yet. Chrome 127+ required."
                );
            }

            let nonce = &encrypted[nonce_start..nonce_start + 12];
            let ciphertext_and_tag = &encrypted[nonce_start + 12..];

            let cipher = Aes256Gcm::new_from_slice(key)
                .map_err(|e| anyhow::anyhow!("AES key init failed: {e}"))?;
            let nonce = Nonce::from_slice(nonce);
            let plaintext = cipher
                .decrypt(nonce, ciphertext_and_tag)
                .map_err(|e| anyhow::anyhow!("AES-GCM decrypt failed: {e}"))?;

            String::from_utf8(plaintext).context("Decrypted value is not valid UTF-8")
        }
        _ => {
            // Try as DPAPI blob (pre-Chrome 80)
            match decrypt_dpapi(encrypted) {
                Ok(decrypted) => {
                    String::from_utf8(decrypted).context("Decrypted DPAPI value is not valid UTF-8")
                }
                Err(e) => anyhow::bail!("Unsupported cookie encryption version {}: {}", version, e),
            }
        }
    }
}

fn host_variants(host_key: &str) -> (String, String) {
    let bare = host_key.trim_start_matches('.').to_string();
    let dotted = format!(".{bare}");
    (bare, dotted)
}

/// Copy a file that may be locked by another process using Win32 shared read
fn copy_with_shared_read(src: &PathBuf, dst: &PathBuf) -> Result<()> {
    use std::fs;

    // Try normal copy first (works if file is not locked)
    if fs::copy(src, dst).is_ok() {
        return Ok(());
    }

    // Use Win32 CreateFileW with shared read/write/delete access
    use std::io::Read;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::core::HSTRING;

    let src_str = src.to_string_lossy().to_string();
    let src_wide = HSTRING::from(src_str.as_str());

    let handle: HANDLE = unsafe {
        CreateFileW(
            &src_wide,
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|e| anyhow::anyhow!("CreateFileW failed for {:?}: {e}", src))?
    };

    if handle == INVALID_HANDLE_VALUE {
        anyhow::bail!("Failed to open file with shared read: {:?}", src);
    }

    let result = (|| -> Result<()> {
        let file = unsafe { std::fs::File::from_raw_handle(handle.0 as *mut _) };
        let mut reader = std::io::BufReader::new(file);
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer)?;
        std::fs::write(dst, &buffer)?;
        Ok(())
    })();

    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}

fn read_cookies_from_db(
    db_path: &PathBuf,
    master_key: &[u8; 32],
    host_key: &str,
    cookie_name: &str,
) -> Result<Vec<String>> {
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!(
        "quotify_cookies_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
    ));

    copy_with_shared_read(db_path, &tmp_path)
        .with_context(|| format!("Failed to copy cookie DB from {:?}", db_path))?;

    let conn = rusqlite::Connection::open_with_flags(
        &tmp_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .context("Failed to open cookie DB copy")?;

    let (bare_host, dotted_host) = host_variants(host_key);
    let mut stmt = conn.prepare(
        "SELECT encrypted_value, value, host_key FROM cookies WHERE host_key IN (?, ?) AND name = ? ORDER BY last_access_utc DESC",
    )?;

    let mut results = Vec::new();

    let rows = stmt.query_map(
        rusqlite::params![bare_host, dotted_host, cookie_name],
        |row| {
            let encrypted: Vec<u8> = row.get(0)?;
            let plain: Option<String> = row.get(1).ok();
            let host: String = row.get(2)?;
            Ok((encrypted, plain, host))
        },
    )?;

    for row in rows {
        let (encrypted, plain, _host) = row?;
        if let Some(val) = plain
            && !val.is_empty()
        {
            results.push(val);
            continue;
        }

        if !encrypted.is_empty() {
            match decrypt_aes_gcm(&encrypted, master_key) {
                Ok(val) => {
                    if !val.is_empty() {
                        results.push(val);
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to decrypt cookie for {}: {}", host_key, e);
                }
            }
        }
    }

    drop(stmt);
    drop(conn);

    let _ = std::fs::remove_file(&tmp_path);

    Ok(results)
}

fn read_cookie_header_from_db(
    db_path: &PathBuf,
    master_key: &[u8; 32],
    host_key: &str,
) -> Result<Vec<(String, String)>> {
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!(
        "quotify_cookie_header_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
    ));

    copy_with_shared_read(db_path, &tmp_path)
        .with_context(|| format!("Failed to copy cookie DB from {:?}", db_path))?;

    let conn = rusqlite::Connection::open_with_flags(
        &tmp_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .context("Failed to open cookie DB copy")?;

    let (bare_host, dotted_host) = host_variants(host_key);
    let mut stmt = conn.prepare(
        "SELECT name, encrypted_value, value FROM cookies WHERE host_key IN (?, ?) ORDER BY last_access_utc DESC",
    )?;

    let rows = stmt.query_map(rusqlite::params![bare_host, dotted_host], |row| {
        let name: String = row.get(0)?;
        let encrypted: Vec<u8> = row.get(1)?;
        let plain: Option<String> = row.get(2).ok();
        Ok((name, encrypted, plain))
    })?;

    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for row in rows {
        let (name, encrypted, plain) = row?;
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }

        if let Some(val) = plain
            && !val.is_empty()
        {
            results.push((name, val));
            continue;
        }

        if !encrypted.is_empty() {
            match decrypt_aes_gcm(&encrypted, master_key) {
                Ok(val) => {
                    if !val.is_empty() {
                        results.push((name, val));
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to decrypt cookie for {}: {}", host_key, e);
                }
            }
        }
    }

    drop(stmt);
    drop(conn);

    let _ = std::fs::remove_file(&tmp_path);

    Ok(results)
}

pub fn find_cookie(domain: &str, cookie_name: &str) -> Result<String> {
    let app_data = local_app_data().context("LOCALAPPDATA env var not found")?;

    let mut errors = Vec::new();

    for browser in BROWSERS {
        let local_state_path = app_data.join(browser.local_state_path);
        if !local_state_path.exists() {
            continue;
        }

        let master_key = match get_master_key(&local_state_path) {
            Ok(key) => key,
            Err(e) => {
                errors.push(format!("{} master key: {e}", browser.name));
                continue;
            }
        };

        for db_path_str in browser.cookie_db_paths {
            let db_path = app_data.join(db_path_str);
            if !db_path.exists() {
                continue;
            }

            match read_cookies_from_db(&db_path, &master_key, domain, cookie_name) {
                Ok(cookies) => {
                    if let Some(cookie) = cookies.first() {
                        tracing::debug!(
                            "Found cookie {} for {} from {} browser",
                            cookie_name,
                            domain,
                            browser.name
                        );
                        return Ok(cookie.clone());
                    }
                }
                Err(e) => {
                    errors.push(format!("{} cookie read: {e}", browser.name));
                }
            }
        }
    }

    if errors.is_empty() {
        anyhow::bail!(
            "No browser cookie database found for {} / {}",
            domain,
            cookie_name
        );
    } else {
        anyhow::bail!(
            "Failed to read cookie '{}' for '{}': {}",
            cookie_name,
            domain,
            errors.join("; ")
        );
    }
}

pub fn find_cookie_header(domains: &[&str]) -> Result<String> {
    let app_data = local_app_data().context("LOCALAPPDATA env var not found")?;

    let mut errors = Vec::new();

    for browser in BROWSERS {
        let local_state_path = app_data.join(browser.local_state_path);
        if !local_state_path.exists() {
            continue;
        }

        let master_key = match get_master_key(&local_state_path) {
            Ok(key) => key,
            Err(e) => {
                errors.push(format!("{} master key: {e}", browser.name));
                continue;
            }
        };

        for db_path_str in browser.cookie_db_paths {
            let db_path = app_data.join(db_path_str);
            if !db_path.exists() {
                continue;
            }

            let mut pairs = Vec::new();
            let mut seen = std::collections::HashSet::new();

            for domain in domains {
                match read_cookie_header_from_db(&db_path, &master_key, domain) {
                    Ok(domain_pairs) => {
                        for (name, value) in domain_pairs {
                            if seen.insert(name.clone()) {
                                pairs.push((name, value));
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(format!("{} cookie header read: {e}", browser.name));
                    }
                }
            }

            if !pairs.is_empty() {
                tracing::debug!(
                    "Found {} cookies for {} from {} browser",
                    pairs.len(),
                    domains.join(", "),
                    browser.name
                );
                return Ok(pairs
                    .into_iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join("; "));
            }
        }
    }

    if errors.is_empty() {
        anyhow::bail!(
            "No browser cookie database found for {}",
            domains.join(", ")
        );
    } else {
        anyhow::bail!(
            "Failed to read cookies for '{}': {}",
            domains.join(", "),
            errors.join("; ")
        );
    }
}

pub fn find_cookie_multiple(domains: &[&str], cookie_name: &str) -> Result<String> {
    for domain in domains {
        if let Ok(cookie) = find_cookie(domain, cookie_name) {
            return Ok(cookie);
        }
    }
    anyhow::bail!(
        "Cookie '{}' not found in any of: {}",
        cookie_name,
        domains.join(", ")
    )
}
