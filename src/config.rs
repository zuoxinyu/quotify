use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

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
    #[serde(default = "default_backdrop")]
    pub backdrop: String,
    #[serde(default = "default_auto_webview_login")]
    pub auto_webview_login: bool,
    #[serde(default)]
    pub start_with_windows: bool,
}

fn default_refresh_interval() -> u64 {
    300
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_backdrop() -> String {
    "mica_alt".to_string()
}

fn default_auto_webview_login() -> bool {
    true
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            refresh_interval: default_refresh_interval(),
            active_provider: String::new(),
            provider_order: Vec::new(),
            theme: default_theme(),
            backdrop: default_backdrop(),
            auto_webview_login: default_auto_webview_login(),
            start_with_windows: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub monthly_resets: bool,
    #[serde(default)]
    pub weekly_resets: bool,
    #[serde(default)]
    pub five_hour_resets: bool,
    #[serde(default)]
    pub usage_threshold_enabled: bool,
    #[serde(default = "default_usage_threshold_percent")]
    pub usage_threshold_percent: f64,
    #[serde(default)]
    pub silent_refresh_failures: bool,
}

fn default_usage_threshold_percent() -> f64 {
    80.0
}

impl NotificationConfig {
    pub fn threshold_percent(&self) -> f64 {
        if self.usage_threshold_percent.is_finite() {
            self.usage_threshold_percent.clamp(1.0, 100.0)
        } else {
            default_usage_threshold_percent()
        }
    }
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            monthly_resets: false,
            weekly_resets: false,
            five_hour_resets: false,
            usage_threshold_enabled: false,
            usage_threshold_percent: default_usage_threshold_percent(),
            silent_refresh_failures: false,
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
    pub notifications: NotificationConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_budgets: BTreeMap<String, f64>,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub deepseek: DeepSeekConfig,
    #[serde(default)]
    pub openrouter: ApiKeyProviderConfig,
    #[serde(default)]
    pub clinepass: ApiKeyProviderConfig,
    #[serde(default)]
    pub alibaba: ApiKeyProviderConfig,
    #[serde(default)]
    pub qwencloud: ApiKeyProviderConfig,
    #[serde(default)]
    pub devin: ApiKeyProviderConfig,
    #[serde(default)]
    pub manus: ApiKeyProviderConfig,
    #[serde(default)]
    pub zed: ApiKeyProviderConfig,
    #[serde(default)]
    pub perplexity: ApiKeyProviderConfig,
    #[serde(default)]
    pub sakana: ApiKeyProviderConfig,
    #[serde(default)]
    pub deepinfra: ApiKeyProviderConfig,
    #[serde(default)]
    pub commandcode: ApiKeyProviderConfig,
    #[serde(default)]
    pub qoder: ApiKeyProviderConfig,
    #[serde(default)]
    pub litellm: ApiKeyProviderConfig,
    #[serde(default)]
    pub poe: ApiKeyProviderConfig,
    #[serde(default)]
    pub chutes: ApiKeyProviderConfig,
    #[serde(default)]
    pub neuralwatt: ApiKeyProviderConfig,
    #[serde(default)]
    pub clawrouter: ApiKeyProviderConfig,
    #[serde(default)]
    pub longcat: ApiKeyProviderConfig,
    #[serde(default)]
    pub sub2api: ApiKeyProviderConfig,
    #[serde(default)]
    pub wayfinder: ApiKeyProviderConfig,
    #[serde(default)]
    pub zenmux: ApiKeyProviderConfig,
    #[serde(default)]
    pub aiand: ApiKeyProviderConfig,
    #[serde(default)]
    pub zoommate: ApiKeyProviderConfig,
    #[serde(default)]
    pub xai: ApiKeyProviderConfig,
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
    /// Legacy input-only section. It is merged into `opencode` during load and
    /// omitted on save so OpenCode and OpenCode Go stay one provider.
    #[serde(default, skip_serializing)]
    pub opencodego: OpenCodeConfig,
    #[serde(default)]
    pub mimo: MimoConfig,
}

impl AppConfig {
    fn merge_legacy_opencode_config(&mut self) {
        if self.opencode.enabled.is_none() {
            self.opencode.enabled = self.opencodego.enabled;
        }
        if self.opencode.api_key.trim().is_empty() {
            self.opencode.api_key = std::mem::take(&mut self.opencodego.api_key);
        }
        if self.opencode.workspace_id.trim().is_empty() {
            self.opencode.workspace_id = std::mem::take(&mut self.opencodego.workspace_id);
        }
        if self.opencode.auth_cookie.trim().is_empty() {
            self.opencode.auth_cookie = std::mem::take(&mut self.opencodego.auth_cookie);
        }
        self.opencodego = OpenCodeConfig::default();

        if self
            .general
            .active_provider
            .eq_ignore_ascii_case("opencodego")
        {
            self.general.active_provider = "opencode".to_string();
        }

        let mut merged_order = Vec::with_capacity(self.general.provider_order.len());
        for provider in &self.general.provider_order {
            let provider = if provider.eq_ignore_ascii_case("opencodego") {
                "opencode"
            } else {
                provider.trim()
            };
            if !provider.is_empty()
                && !merged_order
                    .iter()
                    .any(|known: &String| known.eq_ignore_ascii_case(provider))
            {
                merged_order.push(provider.to_string());
            }
        }
        self.general.provider_order = merged_order;
    }

    pub fn provider_budget(&self, provider: &str) -> Option<f64> {
        let provider = provider.trim();
        self.provider_budgets
            .get(&provider.to_ascii_lowercase())
            .or_else(|| {
                self.provider_budgets
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(provider))
                    .map(|(_, value)| value)
            })
            .copied()
            .filter(|value| value.is_finite() && *value > 0.0)
    }

    pub fn set_provider_budget(&mut self, provider: &str, budget: Option<f64>) {
        let provider = provider.trim().to_ascii_lowercase();
        if provider.is_empty() {
            return;
        }

        self.provider_budgets
            .retain(|name, _| !name.eq_ignore_ascii_case(&provider));
        if let Some(budget) = budget.filter(|value| value.is_finite() && *value > 0.0) {
            self.provider_budgets.insert(provider, budget);
        } else {
            self.provider_budgets.remove(&provider);
        }
    }

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
            config.merge_legacy_opencode_config();

            Ok(config)
        };

        if !path.exists() {
            let backup_path = path.with_extension("toml.bak");
            if backup_path.exists()
                && let Ok(config) = load_impl(&backup_path)
            {
                let _ = config.save_to(path);
                return Ok(config);
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
                            tracing::error!(
                                "Failed to load config from backup as well: {:?}",
                                backup_err
                            );
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
            let mut file = std::fs::File::create(&tmp_path).with_context(|| {
                format!("Failed to create temporary config file {:?}", tmp_path)
            })?;
            file.write_all(content.as_bytes()).with_context(|| {
                format!(
                    "Failed to write content to temporary config file {:?}",
                    tmp_path
                )
            })?;
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
    fn test_general_config_appearance_defaults() {
        let toml_str = r#"
            refresh_interval = 300
            active_provider = "openai"
            provider_order = []
        "#;
        let gen_config: GeneralConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(gen_config.theme, "system");
        assert_eq!(gen_config.backdrop, "mica_alt");
        assert!(gen_config.auto_webview_login);

        let toml_str_with_theme = r#"
            refresh_interval = 300
            active_provider = "openai"
            provider_order = []
            theme = "dark"
            backdrop = "acrylic"
            auto_webview_login = false
        "#;
        let gen_config2: GeneralConfig = toml::from_str(toml_str_with_theme).unwrap();
        assert_eq!(gen_config2.theme, "dark");
        assert_eq!(gen_config2.backdrop, "acrylic");
        assert!(!gen_config2.auto_webview_login);
    }

    #[test]
    fn notification_defaults_are_fully_disabled() {
        let config: AppConfig = toml::from_str("[general]\nrefresh_interval = 300").unwrap();

        assert!(!config.notifications.enabled);
        assert!(!config.notifications.monthly_resets);
        assert!(!config.notifications.weekly_resets);
        assert!(!config.notifications.five_hour_resets);
        assert!(!config.notifications.usage_threshold_enabled);
        assert!(!config.notifications.silent_refresh_failures);
        assert_eq!(config.notifications.threshold_percent(), 80.0);
    }

    #[test]
    fn provider_budgets_only_accept_positive_finite_values() {
        let mut config = AppConfig::default();
        config.set_provider_budget("OpenAI", Some(125.5));
        config.set_provider_budget("bedrock", Some(f64::NAN));

        assert_eq!(config.provider_budget("openai"), Some(125.5));
        assert_eq!(config.provider_budget("OPENAI"), Some(125.5));
        assert_eq!(config.provider_budget("bedrock"), None);

        config.set_provider_budget("openai", None);
        assert_eq!(config.provider_budget("openai"), None);
    }

    #[test]
    fn provider_budget_keys_are_case_insensitive_when_loaded_from_toml() {
        let mut config: AppConfig = toml::from_str("[provider_budgets]\nOpenAI = 75.0\n").unwrap();

        assert_eq!(config.provider_budget("openai"), Some(75.0));
        config.set_provider_budget("OPENAI", Some(90.0));
        assert_eq!(config.provider_budgets.len(), 1);
        assert_eq!(config.provider_budgets.get("openai"), Some(&90.0));
    }

    #[test]
    fn legacy_opencode_go_settings_merge_into_opencode() {
        let mut config: AppConfig = toml::from_str(
            r#"
            [general]
            active_provider = "opencodego"
            provider_order = ["codex", "opencodego", "opencode"]

            [opencode]
            enabled = true

            [opencodego]
            enabled = false
            workspace_id = "wrk_go"
            auth_cookie = "auth=legacy"
            "#,
        )
        .unwrap();
        config.merge_legacy_opencode_config();

        assert_eq!(config.opencode.enabled, Some(true));
        assert_eq!(config.opencode.workspace_id, "wrk_go");
        assert_eq!(config.opencode.auth_cookie, "auth=legacy");
        assert_eq!(config.general.active_provider, "opencode");
        assert_eq!(
            config.general.provider_order,
            vec!["codex".to_string(), "opencode".to_string()]
        );
        assert!(!toml::to_string(&config).unwrap().contains("[opencodego]"));
    }

    #[test]
    fn example_config_stays_parseable_as_provider_catalog_grows() {
        let config: AppConfig = toml::from_str(include_str!("../config.example.toml")).unwrap();
        let mut configured = config.general.provider_order.clone();
        configured.sort();
        let mut registered = crate::PROVIDER_ORDER
            .iter()
            .map(|provider| provider.to_string())
            .collect::<Vec<_>>();
        registered.sort();
        assert_eq!(configured, registered);
        assert_eq!(config.qwencloud.enabled, Some(false));
        assert_eq!(config.xai.deployment, "");
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
