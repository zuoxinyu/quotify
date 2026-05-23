use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: u64,
}

fn default_refresh_interval() -> u64 {
    300
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            refresh_interval: default_refresh_interval(),
        }
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct DeepSeekConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub auth_file: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct CodexConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub auth_file: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct GeminiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct MimoConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub deepseek: DeepSeekConfig,
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
        if !path.exists() {
            let config = Self::default();
            config.save_to(path)?;
            return Ok(config);
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {:?}", path))?;

        let config: AppConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {:?}", path))?;

        Ok(config)
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
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config to {:?}", path))?;
        Ok(())
    }
}
