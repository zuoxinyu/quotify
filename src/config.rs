use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: u64,
    #[serde(default)]
    pub active_provider: String,
    #[serde(default)]
    pub provider_order: Vec<String>,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub start_with_windows: bool,
}

fn default_refresh_interval() -> u64 {
    300
}

fn default_theme() -> String {
    "system".to_string()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            refresh_interval: default_refresh_interval(),
            active_provider: String::new(),
            provider_order: Vec::new(),
            theme: default_theme(),
            start_with_windows: false,
        }
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub proxy: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct DeepSeekConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub deployment: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub auth_file: String,
    #[serde(default)]
    pub session_key: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub access_token: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct CodexConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub auth_file: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct GeminiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub auth_cookie: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct MimoConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub service_token: String,
    #[serde(default)]
    pub cookie_header: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub deepseek: DeepSeekConfig,
    #[serde(default)]
    pub openrouter: ApiKeyProviderConfig,
    #[serde(default)]
    pub openai: ApiKeyProviderConfig,
    #[serde(default)]
    pub moonshot: ApiKeyProviderConfig,
    #[serde(default)]
    pub elevenlabs: ApiKeyProviderConfig,
    #[serde(default)]
    pub doubao: ApiKeyProviderConfig,
    #[serde(default)]
    pub zai: ApiKeyProviderConfig,
    #[serde(default)]
    pub venice: ApiKeyProviderConfig,
    #[serde(default)]
    pub crof: ApiKeyProviderConfig,
    #[serde(default)]
    pub synthetic: ApiKeyProviderConfig,
    #[serde(default)]
    pub warp: ApiKeyProviderConfig,
    #[serde(default)]
    pub groqcloud: ApiKeyProviderConfig,
    #[serde(default)]
    pub deepgram: ApiKeyProviderConfig,
    #[serde(default)]
    pub llmproxy: ApiKeyProviderConfig,
    #[serde(default)]
    pub codebuff: ApiKeyProviderConfig,
    #[serde(default)]
    pub kiro: ApiKeyProviderConfig,
    #[serde(default)]
    pub copilot: ApiKeyProviderConfig,
    #[serde(default)]
    pub azureopenai: ApiKeyProviderConfig,
    #[serde(default)]
    pub ollama: ApiKeyProviderConfig,
    #[serde(default)]
    pub minimax: ApiKeyProviderConfig,
    #[serde(default)]
    pub jetbrains: ApiKeyProviderConfig,
    #[serde(default)]
    pub kimi: ApiKeyProviderConfig,
    #[serde(default)]
    pub kilo: ApiKeyProviderConfig,
    #[serde(default)]
    pub augment: ApiKeyProviderConfig,
    #[serde(default)]
    pub bedrock: ApiKeyProviderConfig,
    #[serde(default)]
    pub vertexai: ApiKeyProviderConfig,
    #[serde(default)]
    pub stepfun: ApiKeyProviderConfig,
    #[serde(default)]
    pub abacus: ApiKeyProviderConfig,
    #[serde(default)]
    pub alibabatoken: ApiKeyProviderConfig,
    #[serde(default)]
    pub t3chat: ApiKeyProviderConfig,
    #[serde(default)]
    pub amp: ApiKeyProviderConfig,
    #[serde(default)]
    pub mistral: ApiKeyProviderConfig,
    #[serde(default)]
    pub grok: ApiKeyProviderConfig,
    #[serde(default)]
    pub cursor: ApiKeyProviderConfig,
    #[serde(default)]
    pub droid: ApiKeyProviderConfig,
    #[serde(default)]
    pub windsurf: ApiKeyProviderConfig,
    #[serde(default)]
    pub claude: ClaudeConfig,
    #[serde(default)]
    pub codex: CodexConfig,
    #[serde(default)]
    pub gemini: GeminiConfig,
    #[serde(default)]
    pub antigravity: AntigravityConfig,
    #[serde(default)]
    pub opencode: OpenCodeConfig,
    #[serde(default)]
    pub mimo: MimoConfig,
}

impl AppConfig {
    pub fn config_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("quotify");
        std::fs::create_dir_all(&config_dir).ok();
        config_dir.join("quotify.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &PathBuf) -> Result<Self> {
        let load_impl = |p: &PathBuf| -> Result<Self> {
            let content = std::fs::read_to_string(p)
                .with_context(|| format!("Failed to read config from {:?}", p))?;

            let mut config: AppConfig = toml::from_str(&content)
                .with_context(|| format!("Failed to parse config from {:?}", p))?;

            if config.general.active_provider.eq_ignore_ascii_case("opencodego") {
                config.general.active_provider = "opencode".to_string();
            }
            for item in &mut config.general.provider_order {
                if item.eq_ignore_ascii_case("opencodego") {
                    *item = "opencode".to_string();
                }
            }
            Ok(config)
        };

        if !path.exists() {
            let backup_path = path.with_extension("toml.bak");
            if backup_path.exists() {
                if let Ok(config) = load_impl(&backup_path) {
                    let _ = config.save_to(path);
                    return Ok(config);
                }
            }
            let config = Self::default();
            config.save_to(path)?;
            return Ok(config);
        }

        match load_impl(path) {
            Ok(config) => Ok(config),
            Err(err) => {
                let backup_path = path.with_extension("toml.bak");
                if backup_path.exists() {
                    tracing::warn!(
                        "Failed to load config from {:?}, attempting to load from backup {:?}: {:?}",
                        path,
                        backup_path,
                        err
                    );
                    match load_impl(&backup_path) {
                        Ok(config) => {
                            let _ = config.save_to(path);
                            return Ok(config);
                        }
                        Err(backup_err) => {
                            tracing::error!("Failed to load config from backup as well: {:?}", backup_err);
                        }
                    }
                }
                Err(err)
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        if path.exists() {
            let backup_path = path.with_extension("toml.bak");
            let _ = std::fs::copy(path, &backup_path);
        }

        let tmp_path = path.with_extension("toml.tmp");
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp_path)
                .with_context(|| format!("Failed to create temporary config file {:?}", tmp_path))?;
            file.write_all(content.as_bytes())
                .with_context(|| format!("Failed to write content to temporary config file {:?}", tmp_path))?;
            file.sync_all()
                .with_context(|| format!("Failed to sync temporary config file {:?}", tmp_path))?;
        }

        std::fs::rename(&tmp_path, path)
            .with_context(|| format!("Failed to rename temporary config file to {:?}", path))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_general_config_theme_defaults() {
        let toml_str = r#"
            refresh_interval = 300
            active_provider = "openai"
            provider_order = []
        "#;
        let gen_config: GeneralConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(gen_config.theme, "system");

        let toml_str_with_theme = r#"
            refresh_interval = 300
            active_provider = "openai"
            provider_order = []
            theme = "dark"
        "#;
        let gen_config2: GeneralConfig = toml::from_str(toml_str_with_theme).unwrap();
        assert_eq!(gen_config2.theme, "dark");
    }

    #[test]
    fn test_config_safe_write_and_recovery() {
        let temp_dir = std::env::temp_dir().join("quotify_test_config");
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_file = temp_dir.join("quotify.toml");
        let backup_file = temp_dir.join("quotify.toml.bak");

        let _ = std::fs::remove_file(&config_file);
        let _ = std::fs::remove_file(&backup_file);

        let mut config = AppConfig::default();
        config.general.refresh_interval = 4242;

        config.save_to(&config_file).unwrap();
        assert!(config_file.exists());

        config.general.refresh_interval = 4343;
        config.save_to(&config_file).unwrap();
        assert!(backup_file.exists());

        std::fs::write(&config_file, "INVALID TOML CONTENT").unwrap();

        let loaded = AppConfig::load_from(&config_file).unwrap();
        assert_eq!(loaded.general.refresh_interval, 4242);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
