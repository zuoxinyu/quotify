use anyhow::{Context, Result};
use windows::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};
use windows::core::{PCWSTR, PWSTR};

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn target(provider: &str, field: &str) -> String {
    format!("quotify/{provider}/{field}")
}

fn get_raw(provider: &str, field: &str) -> Result<Option<String>> {
    let target = wide_null(&target(provider, field));
    let mut credential = std::ptr::null_mut();
    let ok = unsafe {
        CredReadW(
            PCWSTR(target.as_ptr()),
            CRED_TYPE_GENERIC,
            Some(0),
            &mut credential,
        )
    };
    if ok.is_err() {
        return Ok(None);
    }

    let credential_ref = unsafe { &*credential };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            credential_ref.CredentialBlob,
            credential_ref.CredentialBlobSize as usize,
        )
    };
    let value = String::from_utf8_lossy(bytes).to_string();
    unsafe {
        CredFree(credential.cast());
    }
    Ok(Some(value))
}

pub fn get(provider: &str, field: &str) -> Result<Option<String>> {
    // 1. Try to read as chunked parts
    let mut parts = Vec::new();
    let mut i = 0;
    while let Some(part) = get_raw(provider, &format!("{field}/part{i}"))? {
        parts.push(part);
        i += 1;
    }

    if !parts.is_empty() {
        return Ok(Some(parts.join("")));
    }

    // 2. Fallback to reading the single key
    get_raw(provider, field)
}

fn set_raw(provider: &str, field: &str, value: &str) -> Result<()> {
    let target_name = wide_null(&target(provider, field));
    let user_name = wide_null("quotify");
    let mut blob = value.as_bytes().to_vec();

    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target_name.as_ptr() as *mut _),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user_name.as_ptr() as *mut _),
        ..Default::default()
    };

    unsafe { CredWriteW(&credential, 0) }.context("Failed to write Windows credential")?;
    Ok(())
}

pub fn set(provider: &str, field: &str, value: &str) -> Result<()> {
    if value.len() <= 400 {
        // Legacy single key format
        set_raw(provider, field, value)?;
        // Clean up any old parts if present
        for i in 0..20 {
            let _ = delete_raw(provider, &format!("{field}/part{i}"));
        }
    } else {
        // Chunked format
        let _ = delete_raw(provider, field);
        let chars: Vec<char> = value.chars().collect();
        let chunk_size = 400;
        let mut i = 0;
        for chunk in chars.chunks(chunk_size) {
            let chunk_str: String = chunk.iter().collect();
            set_raw(provider, &format!("{field}/part{i}"), &chunk_str)?;
            i += 1;
        }
        // Clean up any remaining/older parts
        for part_idx in i..20 {
            let _ = delete_raw(provider, &format!("{field}/part{part_idx}"));
        }
    }
    Ok(())
}

fn delete_raw(provider: &str, field: &str) -> Result<()> {
    let target = wide_null(&target(provider, field));
    let _ = unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, Some(0)) };
    Ok(())
}

pub fn delete(provider: &str, field: &str) -> Result<()> {
    let _ = delete_raw(provider, field);
    for i in 0..20 {
        let _ = delete_raw(provider, &format!("{field}/part{i}"));
    }
    Ok(())
}

pub fn get_or_env(provider: &str, field: &str, env_names: &[&str]) -> String {
    match get(provider, field) {
        Ok(Some(value)) if !value.trim().is_empty() => value,
        Ok(_) => env_names
            .iter()
            .find_map(|name| {
                std::env::var(name)
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_default(),
        Err(err) => {
            tracing::warn!("Failed to read credential {provider}/{field}: {err}");
            String::new()
        }
    }
}

pub fn set_secret_from_input(provider: &str, field: &str, value: &mut String) {
    if value.trim().is_empty() {
        return;
    }
    if let Err(err) = set(provider, field, value.trim()) {
        tracing::error!("Failed to store credential {provider}/{field}: {err}");
        return;
    }
    value.clear();
}

pub fn configured(provider: &str, field: &str, env_names: &[&str]) -> bool {
    get(provider, field)
        .ok()
        .flatten()
        .is_some_and(|value| !value.trim().is_empty())
        || env_names.iter().any(|name| {
            std::env::var(name)
                .ok()
                .is_some_and(|value| !value.trim().is_empty())
        })
}

pub fn hydrate_config(config: &mut crate::config::AppConfig) {
    config.deepseek.api_key = get_or_env("deepseek", "api_key", &["DEEPSEEK_API_KEY"]);
    hydrate_api_key(
        &mut config.openrouter,
        "openrouter",
        &["OPENROUTER_API_KEY"],
    );
    hydrate_api_key(
        &mut config.openai,
        "openai",
        &["OPENAI_ADMIN_KEY", "OPENAI_API_KEY"],
    );
    hydrate_api_key(
        &mut config.moonshot,
        "moonshot",
        &["MOONSHOT_API_KEY", "KIMI_API_KEY"],
    );
    hydrate_api_key(
        &mut config.elevenlabs,
        "elevenlabs",
        &["ELEVENLABS_API_KEY", "XI_API_KEY"],
    );
    hydrate_api_key(
        &mut config.doubao,
        "doubao",
        &["ARK_API_KEY", "VOLCENGINE_API_KEY", "DOUBAO_API_KEY"],
    );
    hydrate_api_key(&mut config.zai, "zai", &["Z_AI_API_KEY", "ZAI_API_KEY"]);
    hydrate_api_key(
        &mut config.venice,
        "venice",
        &["VENICE_API_KEY", "VENICE_KEY"],
    );
    hydrate_api_key(
        &mut config.crof,
        "crof",
        &["CROF_API_KEY", "CROFAI_API_KEY"],
    );
    hydrate_api_key(&mut config.synthetic, "synthetic", &["SYNTHETIC_API_KEY"]);
    hydrate_api_key(&mut config.warp, "warp", &["WARP_API_KEY", "WARP_TOKEN"]);
    hydrate_api_key(
        &mut config.groqcloud,
        "groqcloud",
        &["GROQ_API_KEY", "GROQCLOUD_API_KEY"],
    );
    hydrate_api_key(&mut config.deepgram, "deepgram", &["DEEPGRAM_API_KEY"]);
    hydrate_api_key(
        &mut config.llmproxy,
        "llmproxy",
        &["LLM_PROXY_API_KEY", "LLMPROXY_API_KEY"],
    );
    hydrate_api_key(&mut config.codebuff, "codebuff", &["CODEBUFF_API_KEY"]);
    hydrate_api_key(&mut config.kiro, "kiro", &["KIRO_API_KEY"]);
    hydrate_api_key(
        &mut config.copilot,
        "copilot",
        &["GITHUB_COPILOT_TOKEN", "COPILOT_TOKEN"],
    );
    hydrate_api_key(
        &mut config.azureopenai,
        "azureopenai",
        &["AZURE_OPENAI_API_KEY", "AZURE_OPENAI_KEY"],
    );
    hydrate_api_key(
        &mut config.ollama,
        "ollama",
        &["OLLAMA_API_KEY", "OLLAMA_COOKIE", "OLLAMA_SESSION_COOKIE"],
    );
    hydrate_api_key(&mut config.minimax, "minimax", &["MINIMAX_API_KEY"]);
    hydrate_api_key(&mut config.kimi, "kimi", &["KIMI_AUTH_TOKEN"]);
    hydrate_api_key(&mut config.kilo, "kilo", &["KILO_API_KEY"]);
    hydrate_api_key(&mut config.augment, "augment", &["AUGMENT_SESSION_TOKEN"]);
    hydrate_api_key(&mut config.bedrock, "bedrock", &["CODEXBAR_BEDROCK_BUDGET"]);
    hydrate_api_key(
        &mut config.vertexai,
        "vertexai",
        &[
            "GOOGLE_CLOUD_PROJECT",
            "GCLOUD_PROJECT",
            "GOOGLE_PROJECT_ID",
        ],
    );
    hydrate_api_key(
        &mut config.stepfun,
        "stepfun",
        &["STEPFUN_TOKEN", "OASIS_TOKEN"],
    );
    hydrate_api_key(
        &mut config.abacus,
        "abacus",
        &["ABACUS_COOKIE", "ABACUS_COOKIE_HEADER", "ABACUS_AI_COOKIE"],
    );
    hydrate_api_key(
        &mut config.alibabatoken,
        "alibabatoken",
        &["ALIBABA_TOKEN_PLAN_COOKIE", "ALIBABA_TOKEN_COOKIE"],
    );
    hydrate_api_key(
        &mut config.t3chat,
        "t3chat",
        &["T3_CHAT_COOKIE", "T3CHAT_COOKIE"],
    );
    hydrate_api_key(&mut config.amp, "amp", &["AMP_COOKIE", "AMPCODE_COOKIE"]);
    hydrate_api_key(&mut config.mistral, "mistral", &["MISTRAL_API_KEY"]);
    hydrate_api_key(&mut config.grok, "grok", &["XAI_API_KEY", "GROK_API_KEY"]);
    hydrate_api_key(
        &mut config.cursor,
        "cursor",
        &["CURSOR_COOKIE", "CURSOR_SESSION_COOKIE"],
    );
    hydrate_api_key(&mut config.droid, "droid", &["FACTORY_API_KEY"]);
    hydrate_api_key(
        &mut config.windsurf,
        "windsurf",
        &["WINDSURF_SERVICE_KEY", "CODEIUM_SERVICE_KEY"],
    );

    config.claude.api_key = get_or_env(
        "claude",
        "api_key",
        &["ANTHROPIC_ADMIN_KEY", "ANTHROPIC_API_KEY"],
    );
    config.claude.session_key = get_or_env("claude", "session_key", &["CLAUDE_SESSION_KEY"]);
    config.claude.access_token = get_or_env("claude", "access_token", &["CLAUDE_ACCESS_TOKEN"]);
    config.gemini.api_key = get_or_env("gemini", "api_key", &["GEMINI_API_KEY", "GOOGLE_API_KEY"]);
    config.antigravity.api_key = get_or_env("antigravity", "api_key", &["ANTIGRAVITY_API_KEY"]);
    config.opencode.api_key = get_or_env("opencode", "api_key", &[]);
    config.opencode.auth_cookie = get_or_env("opencode", "auth_cookie", &["OPENCODE_AUTH_COOKIE"]);
    config.mimo.api_key = get_or_env("mimo", "api_key", &[]);
    config.mimo.service_token = get_or_env("mimo", "service_token", &["MIMO_SERVICE_TOKEN"]);
    config.mimo.cookie_header = get_or_env("mimo", "cookie_header", &["MIMO_COOKIE_HEADER"]);
}

pub fn store_and_scrub_config(config: &mut crate::config::AppConfig) {
    set_secret_from_input("deepseek", "api_key", &mut config.deepseek.api_key);
    store_api_key(&mut config.openrouter, "openrouter");
    store_api_key(&mut config.openai, "openai");
    store_api_key(&mut config.moonshot, "moonshot");
    store_api_key(&mut config.elevenlabs, "elevenlabs");
    store_api_key(&mut config.doubao, "doubao");
    store_api_key(&mut config.zai, "zai");
    store_api_key(&mut config.venice, "venice");
    store_api_key(&mut config.crof, "crof");
    store_api_key(&mut config.synthetic, "synthetic");
    store_api_key(&mut config.warp, "warp");
    store_api_key(&mut config.groqcloud, "groqcloud");
    store_api_key(&mut config.deepgram, "deepgram");
    store_api_key(&mut config.llmproxy, "llmproxy");
    store_api_key(&mut config.codebuff, "codebuff");
    store_api_key(&mut config.kiro, "kiro");
    store_api_key(&mut config.copilot, "copilot");
    store_api_key(&mut config.azureopenai, "azureopenai");
    store_api_key(&mut config.ollama, "ollama");
    store_api_key(&mut config.minimax, "minimax");
    store_api_key(&mut config.jetbrains, "jetbrains");
    store_api_key(&mut config.kimi, "kimi");
    store_api_key(&mut config.kilo, "kilo");
    store_api_key(&mut config.augment, "augment");
    store_api_key(&mut config.bedrock, "bedrock");
    store_api_key(&mut config.vertexai, "vertexai");
    store_api_key(&mut config.stepfun, "stepfun");
    store_api_key(&mut config.abacus, "abacus");
    store_api_key(&mut config.alibabatoken, "alibabatoken");
    store_api_key(&mut config.t3chat, "t3chat");
    store_api_key(&mut config.amp, "amp");
    store_api_key(&mut config.mistral, "mistral");
    store_api_key(&mut config.grok, "grok");
    store_api_key(&mut config.cursor, "cursor");
    store_api_key(&mut config.droid, "droid");
    store_api_key(&mut config.windsurf, "windsurf");

    set_secret_from_input("claude", "api_key", &mut config.claude.api_key);
    set_secret_from_input("claude", "session_key", &mut config.claude.session_key);
    set_secret_from_input("claude", "access_token", &mut config.claude.access_token);
    set_secret_from_input("gemini", "api_key", &mut config.gemini.api_key);
    set_secret_from_input("antigravity", "api_key", &mut config.antigravity.api_key);
    set_secret_from_input("opencode", "api_key", &mut config.opencode.api_key);
    set_secret_from_input("opencode", "auth_cookie", &mut config.opencode.auth_cookie);
    set_secret_from_input("mimo", "api_key", &mut config.mimo.api_key);
    set_secret_from_input("mimo", "service_token", &mut config.mimo.service_token);
    set_secret_from_input("mimo", "cookie_header", &mut config.mimo.cookie_header);
}

fn hydrate_api_key(config: &mut crate::config::ApiKeyProviderConfig, provider: &str, env: &[&str]) {
    config.api_key = get_or_env(provider, "api_key", env);
}

fn store_api_key(config: &mut crate::config::ApiKeyProviderConfig, provider: &str) {
    set_secret_from_input(provider, "api_key", &mut config.api_key);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_credential() {
        // Test short write
        let res = set("test_provider", "test_key", "test_value");
        assert!(res.is_ok());
        let read = get("test_provider", "test_key").unwrap();
        assert_eq!(read, Some("test_value".to_string()));
        let del = delete("test_provider", "test_key");
        assert!(del.is_ok());

        // Test long chunked write (1000 characters)
        let long_val = "a".repeat(1000);
        let res = set("test_provider", "test_key_long", &long_val);
        assert!(res.is_ok());
        let read = get("test_provider", "test_key_long").unwrap();
        assert_eq!(read, Some(long_val));
        let del = delete("test_provider", "test_key_long");
        assert!(del.is_ok());
    }
}
