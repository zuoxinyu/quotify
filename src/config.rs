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
}

fn default_refresh_interval() -> u64 {
    300
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            refresh_interval: default_refresh_interval(),
            active_provider: String::new(),
            provider_order: Vec::new(),
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
