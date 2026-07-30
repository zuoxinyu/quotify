#![recursion_limit = "1024"]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(test, allow(dead_code, unused_variables, unused_imports))]

#[cfg(not(test))]
mod app;
mod config;
mod diagnostics;
mod disclosure;
mod icon;
mod notifications;
mod provider;
mod secrets;
mod single_instance;
mod startup;
mod tray;
mod trend_cache;
mod usage_history;
mod version;

use anyhow::Result;
use clap::{Parser, Subcommand};
use gpui::prelude::*;
use parking_lot::RwLock;
use provider::{
    Provider, UsageData, abacus::AbacusProvider, alibabatoken::AlibabaTokenProvider,
    amp::AmpProvider, antigravity::AntigravityProvider, augment::AugmentProvider,
    azureopenai::AzureOpenAiProvider, bedrock::BedrockProvider, claude::ClaudeProvider,
    codebuff::CodebuffProvider, codex::CodexProvider, copilot::CopilotProvider, crof::CrofProvider,
    cursor::CursorProvider, deepgram::DeepgramProvider, deepseek::DeepSeekProvider,
    doubao::DoubaoProvider, droid::DroidProvider, elevenlabs::ElevenLabsProvider,
    gemini::GeminiProvider, grok::GrokProvider, groqcloud::GroqCloudProvider,
    jetbrains::JetBrainsProvider, kilo::KiloProvider, kimi::KimiProvider, kiro::KiroProvider,
    llmproxy::LlmProxyProvider, mimo::MimoProvider, minimax::MiniMaxProvider,
    mistral::MistralProvider, moonshot::MoonshotProvider, ollama::OllamaProvider,
    openai::OpenAiProvider, opencode::OpenCodeProvider, openrouter::OpenRouterProvider,
    stepfun::StepFunProvider, synthetic::SyntheticProvider, t3chat::T3ChatProvider,
    venice::VeniceProvider, vertexai::VertexAiProvider, warp::WarpProvider,
    windsurf::WindsurfProvider, zai::ZaiProvider,
};
use std::{
    cell::RefCell,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};
pub static UPDATE_CHANNEL: OnceLock<tokio::sync::mpsc::Sender<()>> = OnceLock::new();

pub fn trigger_gui_update() {
    if let Some(tx) = UPDATE_CHANNEL.get() {
        let _ = tx.try_send(());
    }
}

pub static IS_BACKDROP_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static BACKDROP_DARK_MODE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static BACKDROP_SETTING: OnceLock<RwLock<String>> = OnceLock::new();
pub static SYSTEM_SLEEPING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static FLYOUT_STATE: AtomicU8 = AtomicU8::new(FlyoutState::Hidden as u8);
static NEXT_FLYOUT_TIMER_ID: AtomicUsize = AtomicUsize::new(0x5155);
static STARTUP_SHOW_CANCELLED: AtomicBool = AtomicBool::new(false);
const FLYOUT_WIDTH_DIP: f32 = 400.0;
const FLYOUT_HEIGHT_DIP: f32 = 520.0;
const FLYOUT_GAP_PX: i32 = 16;
const FLYOUT_SHOW_DURATION: Duration = Duration::from_millis(180);
const FLYOUT_HIDE_DURATION: Duration = Duration::from_millis(150);
const FLYOUT_TIMER_INTERVAL_MS: u32 = 10;
const STARTUP_INACTIVE_TIMEOUT: Duration = Duration::from_secs(5);
const STARTUP_INACTIVE_TIMER_INTERVAL_MS: u32 = 100;
pub const PROVIDER_ORDER: [&str; 42] = [
    "codex",
    "openai",
    "opencode",
    "claude",
    "gemini",
    "antigravity",
    "deepseek",
    "openrouter",
    "moonshot",
    "elevenlabs",
    "doubao",
    "zai",
    "venice",
    "crof",
    "synthetic",
    "warp",
    "groqcloud",
    "deepgram",
    "llmproxy",
    "codebuff",
    "kiro",
    "copilot",
    "azureopenai",
    "ollama",
    "minimax",
    "jetbrains",
    "kimi",
    "kilo",
    "augment",
    "bedrock",
    "vertexai",
    "stepfun",
    "abacus",
    "alibabatoken",
    "t3chat",
    "amp",
    "mistral",
    "grok",
    "cursor",
    "droid",
    "windsurf",
    "mimo",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum FlyoutState {
    Hidden,
    Showing,
    Visible,
    Hiding,
}

fn flyout_state() -> FlyoutState {
    match FLYOUT_STATE.load(Ordering::SeqCst) {
        value if value == FlyoutState::Showing as u8 => FlyoutState::Showing,
        value if value == FlyoutState::Visible as u8 => FlyoutState::Visible,
        value if value == FlyoutState::Hiding as u8 => FlyoutState::Hiding,
        _ => FlyoutState::Hidden,
    }
}

fn set_flyout_state(state: FlyoutState) {
    FLYOUT_STATE.store(state as u8, Ordering::SeqCst);
    tray::WINDOW_VISIBLE.store(state != FlyoutState::Hidden, Ordering::SeqCst);
}

fn normalize_backdrop_setting(setting: &str) -> &'static str {
    if setting.eq_ignore_ascii_case("mica") {
        "mica"
    } else if setting.eq_ignore_ascii_case("acrylic") {
        "acrylic"
    } else if setting.eq_ignore_ascii_case("none") {
        "none"
    } else {
        // Preserve the stronger material introduced for dark mode, and use it for
        // missing or invalid values from older configuration files.
        "mica_alt"
    }
}

fn current_backdrop_setting() -> String {
    BACKDROP_SETTING
        .get_or_init(|| RwLock::new("mica_alt".to_string()))
        .read()
        .clone()
}

#[derive(Parser)]
#[command(
    name = "quotify",
    about = "AI provider quota monitor for Windows",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, help = "Path to config file")]
    config: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    Fetch {
        #[arg(long, help = "Only fetch specific provider(s)")]
        provider: Option<Vec<String>>,
    },
    Init,
    Tray,
    Uninstall {
        #[arg(long, help = "Keep configuration and history files")]
        keep_data: bool,
    },
}

pub(crate) fn create_provider(name: &str, config: &config::AppConfig) -> Option<Box<dyn Provider>> {
    let proxy = config.network.proxy.trim();
    let proxy = (!proxy.is_empty()).then_some(proxy);

    match name {
        "deepseek" => {
            if config.deepseek.enabled == Some(false) {
                return None;
            }
            let api_key = if !config.deepseek.api_key.is_empty() {
                config.deepseek.api_key.clone()
            } else {
                std::env::var("DEEPSEEK_API_KEY").unwrap_or_default()
            };
            if config.deepseek.enabled.unwrap_or(false) || !api_key.is_empty() {
                Some(Box::new(DeepSeekProvider::new(api_key, proxy)))
            } else {
                None
            }
        }
        "openrouter" => {
            if config.openrouter.enabled == Some(false) {
                return None;
            }
            if config.openrouter.enabled.unwrap_or(false)
                || !config.openrouter.api_key.is_empty()
                || std::env::var("OPENROUTER_API_KEY")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(OpenRouterProvider::new(
                    config.openrouter.api_key.clone(),
                    config.openrouter.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "openai" => {
            if config.openai.enabled == Some(false) {
                return None;
            }
            if config.openai.enabled.unwrap_or(false)
                || !config.openai.api_key.is_empty()
                || std::env::var("OPENAI_ADMIN_KEY")
                    .or_else(|_| std::env::var("OPENAI_API_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(OpenAiProvider::new(
                    config.openai.api_key.clone(),
                    config.openai.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "moonshot" => {
            if config.moonshot.enabled == Some(false) {
                return None;
            }
            if config.moonshot.enabled.unwrap_or(false)
                || !config.moonshot.api_key.is_empty()
                || std::env::var("MOONSHOT_API_KEY")
                    .or_else(|_| std::env::var("KIMI_API_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(MoonshotProvider::new(
                    config.moonshot.api_key.clone(),
                    config.moonshot.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "elevenlabs" => {
            if config.elevenlabs.enabled == Some(false) {
                return None;
            }
            if config.elevenlabs.enabled.unwrap_or(false)
                || !config.elevenlabs.api_key.is_empty()
                || std::env::var("ELEVENLABS_API_KEY")
                    .or_else(|_| std::env::var("XI_API_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(ElevenLabsProvider::new(
                    config.elevenlabs.api_key.clone(),
                    config.elevenlabs.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "doubao" => {
            if config.doubao.enabled == Some(false) {
                return None;
            }
            if config.doubao.enabled.unwrap_or(false)
                || !config.doubao.api_key.is_empty()
                || std::env::var("ARK_API_KEY")
                    .or_else(|_| std::env::var("VOLCENGINE_API_KEY"))
                    .or_else(|_| std::env::var("DOUBAO_API_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(DoubaoProvider::new(
                    config.doubao.api_key.clone(),
                    config.doubao.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "zai" => {
            if config.zai.enabled == Some(false) {
                return None;
            }
            if config.zai.enabled.unwrap_or(false)
                || !config.zai.api_key.is_empty()
                || std::env::var("Z_AI_API_KEY")
                    .or_else(|_| std::env::var("ZAI_API_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(ZaiProvider::new(
                    config.zai.api_key.clone(),
                    config.zai.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "venice" => {
            if config.venice.enabled == Some(false) {
                return None;
            }
            if config.venice.enabled.unwrap_or(false)
                || !config.venice.api_key.is_empty()
                || std::env::var("VENICE_API_KEY")
                    .or_else(|_| std::env::var("VENICE_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(VeniceProvider::new(
                    config.venice.api_key.clone(),
                    config.venice.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "crof" => {
            if config.crof.enabled == Some(false) {
                return None;
            }
            if config.crof.enabled.unwrap_or(false)
                || !config.crof.api_key.is_empty()
                || std::env::var("CROF_API_KEY")
                    .or_else(|_| std::env::var("CROFAI_API_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(CrofProvider::new(
                    config.crof.api_key.clone(),
                    config.crof.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "synthetic" => {
            if config.synthetic.enabled == Some(false) {
                return None;
            }
            if config.synthetic.enabled.unwrap_or(false)
                || !config.synthetic.api_key.is_empty()
                || std::env::var("SYNTHETIC_API_KEY")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(SyntheticProvider::new(
                    config.synthetic.api_key.clone(),
                    config.synthetic.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "warp" => {
            if config.warp.enabled == Some(false) {
                return None;
            }
            if config.warp.enabled.unwrap_or(false)
                || !config.warp.api_key.is_empty()
                || std::env::var("WARP_API_KEY")
                    .or_else(|_| std::env::var("WARP_TOKEN"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(WarpProvider::new(
                    config.warp.api_key.clone(),
                    config.warp.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "groqcloud" => {
            if config.groqcloud.enabled == Some(false) {
                return None;
            }
            if config.groqcloud.enabled.unwrap_or(false)
                || !config.groqcloud.api_key.is_empty()
                || std::env::var("GROQ_API_KEY")
                    .or_else(|_| std::env::var("GROQCLOUD_API_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(GroqCloudProvider::new(
                    config.groqcloud.api_key.clone(),
                    config.groqcloud.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "deepgram" => {
            if config.deepgram.enabled == Some(false) {
                return None;
            }
            if config.deepgram.enabled.unwrap_or(false)
                || !config.deepgram.api_key.is_empty()
                || std::env::var("DEEPGRAM_API_KEY")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(DeepgramProvider::new(
                    config.deepgram.api_key.clone(),
                    config.deepgram.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "llmproxy" => {
            if config.llmproxy.enabled == Some(false) {
                return None;
            }
            if config.llmproxy.enabled.unwrap_or(false)
                || !config.llmproxy.api_key.is_empty()
                || std::env::var("LLM_PROXY_API_KEY")
                    .or_else(|_| std::env::var("LLMPROXY_API_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(LlmProxyProvider::new(
                    config.llmproxy.api_key.clone(),
                    config.llmproxy.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "codebuff" => {
            if config.codebuff.enabled == Some(false) {
                return None;
            }
            if config.codebuff.enabled.unwrap_or(false)
                || !config.codebuff.api_key.is_empty()
                || CodebuffProvider::credentials_file_exists()
                || std::env::var("CODEBUFF_API_KEY")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(CodebuffProvider::new(
                    config.codebuff.api_key.clone(),
                    config.codebuff.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "kiro" => {
            if config.kiro.enabled == Some(false) {
                return None;
            }
            if config.kiro.enabled.unwrap_or(false)
                || !config.kiro.api_key.is_empty()
                || std::env::var("KIRO_API_KEY")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(KiroProvider::new(config.kiro.api_key.clone())))
            } else {
                None
            }
        }
        "copilot" => {
            if config.copilot.enabled == Some(false) {
                return None;
            }
            if config.copilot.enabled.unwrap_or(false)
                || !config.copilot.api_key.is_empty()
                || std::env::var("GITHUB_COPILOT_TOKEN")
                    .or_else(|_| std::env::var("COPILOT_TOKEN"))
                    .or_else(|_| std::env::var("GITHUB_TOKEN"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(CopilotProvider::new(
                    config.copilot.api_key.clone(),
                    config.copilot.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "azureopenai" => {
            if config.azureopenai.enabled == Some(false) {
                return None;
            }
            if config.azureopenai.enabled.unwrap_or(false)
                || !config.azureopenai.api_key.is_empty()
                || std::env::var("AZURE_OPENAI_API_KEY")
                    .or_else(|_| std::env::var("AZURE_OPENAI_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(AzureOpenAiProvider::new(
                    config.azureopenai.api_key.clone(),
                    config.azureopenai.base_url.clone(),
                    config.azureopenai.deployment.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "ollama" => {
            if config.ollama.enabled == Some(false) {
                return None;
            }
            if config.ollama.enabled.unwrap_or(false)
                || !config.ollama.api_key.is_empty()
                || std::env::var("OLLAMA_API_KEY")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
                || std::env::var("OLLAMA_COOKIE")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
                || std::env::var("OLLAMA_SESSION_COOKIE")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(OllamaProvider::new(
                    config.ollama.api_key.clone(),
                    config.ollama.base_url.clone(),
                    config.general.auto_webview_login,
                    proxy,
                )))
            } else {
                None
            }
        }
        "minimax" => {
            if config.minimax.enabled == Some(false) {
                return None;
            }
            if config.minimax.enabled.unwrap_or(false)
                || !config.minimax.api_key.is_empty()
                || std::env::var("MINIMAX_API_KEY")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(MiniMaxProvider::new(
                    config.minimax.api_key.clone(),
                    config.minimax.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "jetbrains" => {
            if config.jetbrains.enabled == Some(false) {
                return None;
            }
            if config.jetbrains.enabled.unwrap_or(false)
                || !config.jetbrains.api_key.is_empty()
                || JetBrainsProvider::quota_file_exists(&config.jetbrains.base_url)
            {
                Some(Box::new(JetBrainsProvider::new(
                    config.jetbrains.base_url.clone(),
                )))
            } else {
                None
            }
        }
        "kimi" => {
            if config.kimi.enabled == Some(false) {
                return None;
            }
            if config.kimi.enabled.unwrap_or(false)
                || !config.kimi.api_key.is_empty()
                || std::env::var("KIMI_AUTH_TOKEN")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(KimiProvider::new(
                    config.kimi.api_key.clone(),
                    config.kimi.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "kilo" => {
            if config.kilo.enabled == Some(false) {
                return None;
            }
            if config.kilo.enabled.unwrap_or(false)
                || !config.kilo.api_key.is_empty()
                || KiloProvider::has_cli_or_token(&config.kilo.api_key)
            {
                Some(Box::new(KiloProvider::new(config.kilo.api_key.clone())))
            } else {
                None
            }
        }
        "augment" => {
            if config.augment.enabled == Some(false) {
                return None;
            }
            if config.augment.enabled.unwrap_or(false)
                || !config.augment.api_key.is_empty()
                || AugmentProvider::has_cli_or_token(&config.augment.api_key)
            {
                Some(Box::new(AugmentProvider::new(
                    config.augment.api_key.clone(),
                )))
            } else {
                None
            }
        }
        "bedrock" => {
            if config.bedrock.enabled == Some(false) {
                return None;
            }
            if config.bedrock.enabled.unwrap_or(false)
                || std::env::var("AWS_ACCESS_KEY_ID").is_ok()
                || std::env::var("AWS_PROFILE").is_ok()
                || std::env::var("CODEXBAR_BEDROCK_BUDGET").is_ok()
            {
                Some(Box::new(BedrockProvider::new()))
            } else {
                None
            }
        }
        "vertexai" => {
            if config.vertexai.enabled == Some(false) {
                return None;
            }
            if config.vertexai.enabled.unwrap_or(false)
                || VertexAiProvider::has_project(&config.vertexai.api_key)
            {
                Some(Box::new(VertexAiProvider::new(
                    config.vertexai.api_key.clone(),
                )))
            } else {
                None
            }
        }
        "stepfun" => {
            if config.stepfun.enabled == Some(false) {
                return None;
            }
            if config.stepfun.enabled.unwrap_or(false)
                || !config.stepfun.api_key.is_empty()
                || std::env::var("STEPFUN_TOKEN")
                    .or_else(|_| std::env::var("OASIS_TOKEN"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(StepFunProvider::new(
                    config.stepfun.api_key.clone(),
                    config.stepfun.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "abacus" => {
            if config.abacus.enabled == Some(false) {
                return None;
            }
            if config.abacus.enabled.unwrap_or(false)
                || !config.abacus.api_key.is_empty()
                || std::env::var("ABACUS_COOKIE")
                    .or_else(|_| std::env::var("ABACUS_COOKIE_HEADER"))
                    .or_else(|_| std::env::var("ABACUS_AI_COOKIE"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(AbacusProvider::new(
                    config.abacus.api_key.clone(),
                    config.abacus.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "alibabatoken" => {
            if config.alibabatoken.enabled == Some(false) {
                return None;
            }
            if config.alibabatoken.enabled.unwrap_or(false)
                || !config.alibabatoken.api_key.is_empty()
                || std::env::var("ALIBABA_TOKEN_PLAN_COOKIE")
                    .or_else(|_| std::env::var("ALIBABA_TOKEN_COOKIE"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(AlibabaTokenProvider::new(
                    config.alibabatoken.api_key.clone(),
                    config.alibabatoken.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "t3chat" => {
            if config.t3chat.enabled == Some(false) {
                return None;
            }
            if config.t3chat.enabled.unwrap_or(false)
                || !config.t3chat.api_key.is_empty()
                || std::env::var("T3_CHAT_COOKIE")
                    .or_else(|_| std::env::var("T3CHAT_COOKIE"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(T3ChatProvider::new(
                    config.t3chat.api_key.clone(),
                    config.t3chat.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "amp" => {
            if config.amp.enabled == Some(false) {
                return None;
            }
            if config.amp.enabled.unwrap_or(false)
                || !config.amp.api_key.is_empty()
                || std::env::var("AMP_COOKIE")
                    .or_else(|_| std::env::var("AMPCODE_COOKIE"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(AmpProvider::new(
                    config.amp.api_key.clone(),
                    config.amp.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "mistral" => {
            if config.mistral.enabled == Some(false) {
                return None;
            }
            if config.mistral.enabled.unwrap_or(false)
                || !config.mistral.api_key.is_empty()
                || std::env::var("MISTRAL_API_KEY")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(MistralProvider::new(
                    config.mistral.api_key.clone(),
                    config.mistral.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "grok" => {
            if config.grok.enabled == Some(false) {
                return None;
            }
            if config.grok.enabled.unwrap_or(false)
                || !config.grok.api_key.is_empty()
                || std::env::var("XAI_API_KEY")
                    .or_else(|_| std::env::var("GROK_API_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(GrokProvider::new(
                    config.grok.api_key.clone(),
                    config.grok.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "cursor" => {
            if config.cursor.enabled == Some(false) {
                return None;
            }
            if config.cursor.enabled.unwrap_or(false)
                || !config.cursor.api_key.is_empty()
                || std::env::var("CURSOR_COOKIE")
                    .or_else(|_| std::env::var("CURSOR_SESSION_COOKIE"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(CursorProvider::new(
                    config.cursor.api_key.clone(),
                    config.cursor.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "droid" => {
            if config.droid.enabled == Some(false) {
                return None;
            }
            if config.droid.enabled.unwrap_or(false)
                || !config.droid.api_key.is_empty()
                || DroidProvider::has_cli_or_token(&config.droid.api_key)
            {
                Some(Box::new(DroidProvider::new(config.droid.api_key.clone())))
            } else {
                None
            }
        }
        "windsurf" => {
            if config.windsurf.enabled == Some(false) {
                return None;
            }
            if config.windsurf.enabled.unwrap_or(false)
                || !config.windsurf.api_key.is_empty()
                || std::env::var("WINDSURF_SERVICE_KEY")
                    .or_else(|_| std::env::var("CODEIUM_SERVICE_KEY"))
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(WindsurfProvider::new(
                    config.windsurf.api_key.clone(),
                    config.windsurf.base_url.clone(),
                    proxy,
                )))
            } else {
                None
            }
        }
        "claude" => {
            if config.claude.enabled == Some(false) {
                return None;
            }
            let session_key = if config.claude.session_key.is_empty() {
                None
            } else {
                Some(config.claude.session_key.clone())
            };
            let api_key = if config.claude.api_key.is_empty() {
                None
            } else {
                Some(config.claude.api_key.clone())
            };
            let access_token = if config.claude.access_token.is_empty() {
                None
            } else {
                Some(config.claude.access_token.clone())
            };
            let auth_file = if config.claude.auth_file.is_empty() {
                None
            } else {
                Some(config.claude.auth_file.clone())
            };

            let has_creds = config.claude.enabled.unwrap_or(false)
                || auth_file.is_some()
                || session_key.is_some()
                || api_key.is_some()
                || access_token.is_some()
                || std::env::var("CLAUDE_SESSION_KEY").is_ok()
                || std::env::var("CLAUDE_ACCESS_TOKEN").is_ok()
                || std::env::var("ANTHROPIC_ADMIN_KEY").is_ok()
                || std::env::var("ANTHROPIC_API_KEY").is_ok()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".claude")
                    .join(".credentials.json")
                    .exists()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".claude")
                    .join("settings.json")
                    .exists();

            if has_creds {
                Some(Box::new(ClaudeProvider::new(
                    auth_file,
                    session_key,
                    api_key,
                    access_token,
                    proxy,
                )))
            } else {
                None
            }
        }
        "codex" => {
            if config.codex.enabled == Some(false) {
                return None;
            }
            let has_auth = config.codex.enabled.unwrap_or(false)
                || !config.codex.auth_file.is_empty()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".codex")
                    .join("auth.json")
                    .exists();
            if has_auth {
                let auth_file = if config.codex.auth_file.is_empty() {
                    None
                } else {
                    Some(config.codex.auth_file.clone())
                };
                Some(Box::new(CodexProvider::new(auth_file, proxy)))
            } else {
                None
            }
        }
        "gemini" => {
            let api_key = if !config.gemini.api_key.is_empty() {
                Some(config.gemini.api_key.clone())
            } else {
                None
            };
            if config.gemini.enabled == Some(false) {
                return None;
            }
            if config.gemini.enabled.unwrap_or(false)
                || api_key.is_some()
                || std::env::var("GEMINI_API_KEY").is_ok()
                || std::env::var("GOOGLE_API_KEY").is_ok()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".gemini")
                    .join("oauth_creds.json")
                    .exists()
            {
                Some(Box::new(GeminiProvider::new(api_key, proxy)))
            } else {
                None
            }
        }
        "antigravity" => {
            let api_key = if !config.antigravity.api_key.is_empty() {
                Some(config.antigravity.api_key.clone())
            } else {
                None
            };
            if config.antigravity.enabled == Some(false) {
                return None;
            }
            if config.antigravity.enabled.unwrap_or(false)
                || api_key.is_some()
                || std::env::var("ANTIGRAVITY_API_KEY").is_ok()
                || std::env::var("ANTIGRAVITY_OAUTH_CREDENTIALS_JSON").is_ok()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".codexbar")
                    .join("antigravity")
                    .join("oauth_creds.json")
                    .exists()
            {
                Some(Box::new(AntigravityProvider::new(api_key, proxy)))
            } else {
                None
            }
        }
        "opencode" => {
            let workspace_id = if config.opencode.workspace_id.is_empty() {
                None
            } else {
                Some(config.opencode.workspace_id.clone())
            };
            let auth_cookie = if config.opencode.auth_cookie.is_empty() {
                None
            } else {
                Some(config.opencode.auth_cookie.clone())
            };

            if config.opencode.enabled == Some(false) {
                return None;
            }
            if config.opencode.enabled.unwrap_or(false)
                || workspace_id.is_some()
                || auth_cookie.is_some()
                || OpenCodeProvider::has_workspace_hint()
                || OpenCodeProvider::has_auth_cookie_hint()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".local")
                    .join("share")
                    .join("opencode")
                    .join("auth.json")
                    .exists()
            {
                Some(Box::new(OpenCodeProvider::new(
                    workspace_id,
                    auth_cookie,
                    config.general.auto_webview_login,
                    proxy,
                )))
            } else {
                None
            }
        }
        "opencodego" => create_provider("opencode", config),
        "mimo" => {
            let service_token = if config.mimo.service_token.is_empty() {
                None
            } else {
                Some(config.mimo.service_token.clone())
            };
            let cookie_header = if config.mimo.cookie_header.is_empty() {
                None
            } else {
                Some(config.mimo.cookie_header.clone())
            };

            if config.mimo.enabled == Some(false) {
                return None;
            }
            if config.mimo.enabled.unwrap_or(false)
                || service_token.is_some()
                || cookie_header.is_some()
                || std::env::var("MIMO_SERVICE_TOKEN")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
                || std::env::var("MIMO_COOKIE_HEADER")
                    .ok()
                    .is_some_and(|v| !v.is_empty())
            {
                Some(Box::new(MimoProvider::new(
                    config.mimo.api_key.clone(),
                    service_token,
                    cookie_header,
                    config.general.auto_webview_login,
                    proxy,
                )))
            } else {
                None
            }
        }
        _ => {
            eprintln!("Unknown provider: {name}");
            None
        }
    }
}

async fn fetch_all_providers(config: &config::AppConfig) -> Vec<UsageData> {
    let all_providers = PROVIDER_ORDER;

    let provider_names: Vec<String> = all_providers
        .iter()
        .filter(|name| create_provider(name, config).is_some())
        .map(|s| s.to_string())
        .collect();

    let provider_names = if provider_names.is_empty() {
        all_providers.iter().map(|s| s.to_string()).collect()
    } else {
        provider_names
    };

    fetch_providers(config, provider_names).await
}

fn publish_provider_results(
    config: &config::AppConfig,
    mut results: Vec<UsageData>,
    data: &Arc<RwLock<Vec<UsageData>>>,
    last_refresh: &Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    history: &Arc<RwLock<usage_history::UsageHistory>>,
    silent_refresh: bool,
) {
    let mut budget_refresh_failures = std::collections::BTreeSet::new();
    for result in &mut results {
        if apply_provider_budget(config, result) {
            budget_refresh_failures.insert(result.provider.trim().to_ascii_lowercase());
        }
    }
    let fetched_at = chrono::Utc::now();
    let notification_events = {
        let history = history.read();
        let mut events = notifications::evaluate(
            &config.notifications,
            &history,
            &results,
            fetched_at,
            silent_refresh,
        );
        events.extend(notifications::evaluate_budget_refresh_failures(
            &config.notifications,
            &history.budget_refresh_failures,
            &budget_refresh_failures,
            silent_refresh,
        ));
        events
    };

    *data.write() = results.clone();
    *last_refresh.write() = fetched_at;
    {
        let mut history = history.write();
        history.append(results);
        history.budget_refresh_failures = budget_refresh_failures;
        if let Err(err) = history.save() {
            tracing::error!("Failed to save usage history: {err}");
        }
    }

    if let Some((title, body)) = notifications::aggregate(&notification_events) {
        let severity = if notification_events.iter().any(|event| {
            matches!(
                event,
                notifications::NotificationEvent::SilentRefreshFailed { .. }
            )
        }) {
            tray::NotificationSeverity::Error
        } else if notification_events.iter().any(|event| {
            matches!(
                event,
                notifications::NotificationEvent::UsageThresholdExceeded { .. }
                    | notifications::NotificationEvent::BudgetRefreshFailed { .. }
            )
        }) {
            tray::NotificationSeverity::Warning
        } else {
            tray::NotificationSeverity::Info
        };
        if let Err(err) = tray::send_native_notification(title, body, severity) {
            tracing::warn!("Failed to queue Windows notification: {err}");
        }
    }
}

fn main() -> Result<()> {
    let _log_guard = diagnostics::init_file_logging();
    diagnostics::setup_panic_hook();

    let cli = Cli::parse();

    let config_path = cli.config.as_ref().map(std::path::PathBuf::from);
    let mut config = if let Some(ref path) = config_path {
        config::AppConfig::load_from(path)?
    } else {
        config::AppConfig::load()?
    };
    secrets::hydrate_config(&mut config);

    match cli.command.unwrap_or(Commands::Tray) {
        Commands::Fetch {
            provider: providers,
        } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(run_fetch(&config, providers))?;
        }
        Commands::Init => {
            let path = config::AppConfig::config_path();
            config::AppConfig::default().save()?;
            println!("Config written to: {}", path.display());
        }
        Commands::Tray => {
            #[cfg(not(test))]
            {
                match single_instance::SingleInstanceGuard::acquire() {
                    Ok(_guard) => {
                        run_tray(config, config_path)?;
                    }
                    Err(err) => {
                        if single_instance::activate_existing_instance() {
                            tracing::info!("Activated existing Quotify instance.");
                        } else {
                            tracing::error!(
                                "Quotify is already running, but could not activate the existing instance: {err}"
                            );
                            return Err(err);
                        }
                    }
                }
            }
        }
        Commands::Uninstall { keep_data } => {
            run_uninstall(keep_data)?;
        }
    }

    Ok(())
}

fn run_uninstall(keep_data: bool) -> Result<()> {
    println!("Uninstalling Quotify...");

    match startup::set_enabled(false) {
        Ok(_) => println!("- Removed Windows startup registry key"),
        Err(e) => println!("- Failed to remove startup registry key: {e}"),
    }

    if !keep_data {
        let app_dir = diagnostics::app_dir();
        if app_dir.exists() {
            println!("- Deleting data directory: {}", app_dir.display());
            if let Err(e) = std::fs::remove_dir_all(&app_dir) {
                println!("  Failed to delete data directory: {e}");
            } else {
                println!("  Successfully deleted data directory");
            }
        } else {
            println!("- Data directory does not exist or was already removed.");
        }
    } else {
        println!("- Keeping user configuration and usage history files.");
    }

    println!("Uninstall completed.");
    Ok(())
}

async fn run_fetch(config: &config::AppConfig, providers: Option<Vec<String>>) -> Result<()> {
    let provider_names = providers.unwrap_or_else(|| {
        let active = configured_fetch_order(config);

        if active.is_empty() {
            PROVIDER_ORDER.iter().map(|s| s.to_string()).collect()
        } else {
            active
        }
    });

    let mut results = fetch_providers(config, provider_names).await;
    for result in &mut results {
        apply_provider_budget(config, result);
    }

    let json = serde_json::to_string_pretty(&results)?;
    println!("{json}");

    Ok(())
}

/// Provider order for machine-readable `fetch` output. Keep the configured
/// order authoritative so downstream consumers (for example small external
/// displays) render providers in the same order as the quotify tray UI.
fn configured_fetch_order(config: &config::AppConfig) -> Vec<String> {
    let mut names = Vec::new();

    for configured in &config.general.provider_order {
        let name = configured.trim().to_ascii_lowercase();
        if name.is_empty() || names.iter().any(|existing| existing == &name) {
            continue;
        }
        if create_provider(&name, config).is_some() {
            names.push(name);
        }
    }

    for name in PROVIDER_ORDER {
        if !names.iter().any(|existing| existing == name) && create_provider(name, config).is_some()
        {
            names.push(name.to_string());
        }
    }

    names
}

async fn fetch_providers(
    config: &config::AppConfig,
    provider_names: Vec<String>,
) -> Vec<UsageData> {
    let mut handles = Vec::new();

    for name in provider_names {
        if let Some(provider) = create_provider(&name, config) {
            handles.push((
                name.clone(),
                tokio::spawn(async move { provider.fetch_usage().await }),
            ));
        }
    }

    let mut results = Vec::with_capacity(handles.len());
    for (name, handle) in handles {
        match handle.await {
            Ok(Ok(data)) => results.push(data),
            Ok(Err(e)) => {
                let error = format!("{e:#}");
                tracing::error!("Failed to fetch {}: {}", name, error);
                results.push(provider_error_data(name, error));
            }
            Err(e) => {
                let error = format!("{e:#}");
                tracing::error!("Failed to join {} fetch task: {}", name, error);
                results.push(provider_error_data(name, error));
            }
        }
    }

    results
}

/// Applies a configured API budget. Returns `true` when the provider refresh
/// itself succeeded but did not include usable 30-day spend data.
pub(crate) fn apply_provider_budget(config: &config::AppConfig, data: &mut UsageData) -> bool {
    let provider = data.provider.trim().to_ascii_lowercase();
    if !provider::supports_api_budget(&provider) {
        return false;
    }

    for window in data.windows.iter_mut().filter(|window| {
        provider::is_budget_spend_window(&provider, &window.label, window.unit.as_deref())
    }) {
        window.limit = None;
        window.used_percent = 0.0;
    }

    let budget = config
        .provider_budget(&provider)
        .or_else(|| legacy_bedrock_budget(config, &provider));
    let Some(budget) = budget else {
        return false;
    };
    if data.error.is_some() {
        return false;
    }

    let Some(window) = data.windows.iter_mut().find(|window| {
        provider::is_budget_spend_window(&provider, &window.label, window.unit.as_deref())
    }) else {
        return true;
    };
    let Some(used) = window.used.filter(|used| used.is_finite() && *used >= 0.0) else {
        return true;
    };

    window.limit = Some(budget);
    window.used_percent = (used / budget * 100.0).clamp(0.0, 100.0);
    false
}

fn legacy_bedrock_budget(config: &config::AppConfig, provider: &str) -> Option<f64> {
    if provider != "bedrock" {
        return None;
    }

    (!config.bedrock.api_key.trim().is_empty())
        .then_some(config.bedrock.api_key.as_str())
        .and_then(parse_positive_budget)
        .or_else(|| {
            std::env::var("CODEXBAR_BEDROCK_BUDGET")
                .ok()
                .as_deref()
                .and_then(parse_positive_budget)
        })
}

fn parse_positive_budget(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

#[cfg(test)]
mod provider_budget_tests {
    use super::*;

    fn usage(provider: &str, label: &str, used: f64, unit: &str) -> UsageData {
        UsageData {
            provider: provider.to_string(),
            windows: vec![provider::UsageWindow {
                label: label.to_string(),
                used_percent: 0.0,
                limit: None,
                used: Some(used),
                unit: Some(unit.to_string()),
                resets_at: None,
            }],
            credits: None,
            subscription_tier: None,
            fetched_at: chrono::Utc::now(),
            error: None,
        }
    }

    #[test]
    fn configured_budget_is_applied_to_monthly_spend() {
        let mut config = config::AppConfig::default();
        config.set_provider_budget("openai", Some(100.0));
        let mut data = usage("openai", "Cost 30d", 42.5, "USD");

        assert!(!apply_provider_budget(&config, &mut data));

        assert_eq!(data.windows[0].limit, Some(100.0));
        assert_eq!(data.windows[0].used_percent, 42.5);
        assert!(data.credits.is_none());
    }

    #[test]
    fn changing_or_clearing_a_budget_recomputes_cached_spend_data() {
        let mut config = config::AppConfig::default();
        config.set_provider_budget("openai", Some(100.0));
        let mut data = usage("openai", "Cost 30d", 50.0, "USD");
        assert!(!apply_provider_budget(&config, &mut data));
        assert_eq!(data.windows[0].used_percent, 50.0);

        config.set_provider_budget("openai", Some(200.0));
        assert!(!apply_provider_budget(&config, &mut data));
        assert_eq!(data.windows[0].limit, Some(200.0));
        assert_eq!(data.windows[0].used_percent, 25.0);

        config.set_provider_budget("openai", None);
        assert!(!apply_provider_budget(&config, &mut data));
        assert_eq!(data.windows[0].limit, None);
        assert_eq!(data.windows[0].used_percent, 0.0);
    }

    #[test]
    fn budget_does_not_relabel_balance_only_or_non_monthly_data() {
        let mut config = config::AppConfig::default();
        config.set_provider_budget("openai", Some(100.0));
        let mut credits = usage("openai", "Credits", 25.0, "USD");
        let mut wrong_unit = usage("openai", "Cost 30d", 25.0, "tokens");

        assert!(apply_provider_budget(&config, &mut credits));
        assert!(apply_provider_budget(&config, &mut wrong_unit));

        assert_eq!(credits.windows[0].limit, None);
        assert!(credits.credits.is_none());
        assert_eq!(wrong_unit.windows[0].limit, None);
        assert!(wrong_unit.credits.is_none());
    }

    #[test]
    fn claude_budget_targets_only_the_thirty_day_admin_window() {
        let mut config = config::AppConfig::default();
        config.set_provider_budget("claude", Some(50.0));
        let mut data = usage("claude", "30d Spend", 75.0, "usd");

        assert!(!apply_provider_budget(&config, &mut data));

        assert_eq!(data.windows[0].limit, Some(50.0));
        assert_eq!(data.windows[0].used_percent, 100.0);
        assert!(data.credits.is_none());
    }

    #[test]
    fn provider_error_owns_the_failure_notification_when_budget_is_configured() {
        let mut config = config::AppConfig::default();
        config.set_provider_budget("openai", Some(100.0));
        let mut data = usage("openai", "Error", 0.0, "");
        data.error = Some("unavailable".to_string());

        assert!(!apply_provider_budget(&config, &mut data));
    }
}

fn provider_error_data(provider: String, error: String) -> UsageData {
    UsageData {
        provider,
        windows: vec![provider::UsageWindow {
            label: "Error".to_string(),
            used_percent: 0.0,
            limit: None,
            used: None,
            unit: None,
            resets_at: None,
        }],
        credits: None,
        subscription_tier: None,
        fetched_at: chrono::Utc::now(),
        error: Some(error),
    }
}

fn active_provider_option(active_provider: &str) -> Option<&str> {
    let active_provider = active_provider.trim();
    if active_provider.is_empty() {
        None
    } else {
        Some(active_provider)
    }
}

fn load_runtime_config(
    config_path: Option<&std::path::PathBuf>,
    fallback: &config::AppConfig,
) -> config::AppConfig {
    let loaded = if let Some(path) = config_path {
        config::AppConfig::load_from(path)
    } else {
        config::AppConfig::load()
    };

    let mut config = loaded.unwrap_or_else(|err| {
        tracing::error!("Failed to reload config, using previous config: {err}");
        fallback.clone()
    });
    secrets::hydrate_config(&mut config);
    config
}

#[derive(rust_embed::Embed)]
#[folder = "assets/"]
struct AppAssets;

impl AppAssets {
    fn key(path: &str) -> &str {
        let path = path.trim_start_matches('/');
        path.strip_prefix("assets/").unwrap_or(path)
    }
}

impl gpui::AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        let path = path.replace('\\', "/");
        Ok(Self::get(Self::key(&path)).map(|asset| asset.data))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        let path = path.replace('\\', "/");
        let prefix = Self::key(&path).trim_end_matches('/');
        let prefix = if prefix.is_empty() {
            None
        } else {
            Some(format!("{prefix}/"))
        };

        Ok(Self::iter()
            .filter(|asset| {
                prefix
                    .as_ref()
                    .is_none_or(|prefix| asset.starts_with(prefix.as_str()))
            })
            .map(|asset| gpui::SharedString::from(format!("assets/{asset}")))
            .collect())
    }
}

#[cfg(test)]
mod app_assets_tests {
    use super::*;

    #[test]
    fn gpui_assets_are_embedded_and_keep_their_logical_paths() {
        let asset = gpui::AssetSource::load(&AppAssets, "assets/icons/quotify.svg")
            .unwrap()
            .expect("embedded Quotify icon");

        assert!(matches!(&asset, std::borrow::Cow::Borrowed(_)));
        assert!(asset.starts_with(b"<svg"));
        assert!(
            gpui::AssetSource::list(&AppAssets, "assets/icons")
                .unwrap()
                .iter()
                .any(|path| path.as_ref() == "assets/icons/quotify.svg")
        );

        for path in [
            "icons/chevron-down.svg",
            "icons/circle-check.svg",
            "icons/circle-x.svg",
            "icons/eye.svg",
            "icons/inbox.svg",
            "icons/info.svg",
            "icons/loader.svg",
            "icons/search.svg",
            "icons/triangle-alert.svg",
        ] {
            let asset = gpui::AssetSource::load(&AppAssets, path)
                .unwrap_or_else(|err| panic!("failed to load {path}: {err}"))
                .unwrap_or_else(|| panic!("missing gpui-component asset {path}"));
            assert!(asset.starts_with(b"<svg"), "invalid SVG at {path}");
        }
    }
}

#[cfg(not(test))]
fn run_tray(config: config::AppConfig, config_path: Option<std::path::PathBuf>) -> Result<()> {
    if let Err(err) = startup::set_enabled(config.general.start_with_windows) {
        tracing::error!("Failed to sync startup setting: {err}");
    }
    let _ = startup::verify_and_sync_path();

    let history = Arc::new(RwLock::new(usage_history::UsageHistory::load()));
    let cached_data = history.read().latest_successful();
    let data: Arc<RwLock<Vec<UsageData>>> = Arc::new(RwLock::new(cached_data));
    let last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>> = Arc::new(RwLock::new(
        history
            .read()
            .entries
            .last()
            .map(|entry| entry.fetched_at)
            .unwrap_or_else(chrono::Utc::now),
    ));
    let active_provider = Arc::new(RwLock::new(
        config.general.active_provider.trim().to_string(),
    ));

    // Spawn tray controller on a background thread so its Win32 message loop can block there.
    let (tray_tx, tray_rx) = std::sync::mpsc::channel();
    let data_bg_tray = data.clone();
    let active_provider_bg_tray = active_provider.clone();
    let history_bg_tray = history.clone();
    std::thread::spawn(move || {
        let tray_controller =
            Arc::new(tray::TrayController::new().expect("Failed to create tray controller"));

        // Set initial loading icon before data is fetched
        let (initial_icon, tooltip) = {
            let d = data_bg_tray.read();
            let d_resolved: Vec<UsageData> = d
                .iter()
                .map(|item| {
                    if item.error.is_some() {
                        if let Some(cached) =
                            history_bg_tray.read().latest_successful_for(&item.provider)
                        {
                            cached
                        } else {
                            item.clone()
                        }
                    } else {
                        item.clone()
                    }
                })
                .collect();
            let active_provider = active_provider_bg_tray.read();
            let icon = icon::generate_icon(&d_resolved, active_provider_option(&active_provider));
            let tooltip = icon::tray_tooltip(&d_resolved, active_provider_option(&active_provider));
            (icon, tooltip)
        };
        if let Ok(hicon) = initial_icon.to_hicon() {
            tray_controller.update_icon_with_tooltip(hicon, &tooltip);
        }
        // Publish the controller only after the first tray-icon registration
        // attempt, so the startup flyout can anchor to the actual icon.
        tray_tx.send(tray_controller.clone()).unwrap();

        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });

    let tray_controller = tray_rx.recv().unwrap();

    let refresh_interval = config.general.refresh_interval;
    let data_bg = data.clone();
    let last_refresh_bg = last_refresh.clone();
    let history_bg = history.clone();
    let config_bg = config.clone();
    let config_path_bg = config_path.clone();
    let tc_bg = tray_controller.clone();
    let active_provider_bg = active_provider.clone();

    // Spawn background refresh thread
    std::thread::spawn(move || {
        let bg_rt = tokio::runtime::Runtime::new().expect("Failed to create background runtime");
        let min_refresh_interval = refresh_interval.max(10);
        let refresh_interval_duration = std::time::Duration::from_secs(min_refresh_interval);
        let mut last_fetch: Option<std::time::Instant> = None;
        loop {
            if SYSTEM_SLEEPING.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
            let refresh_request = tray::take_refresh_request();
            let now = std::time::Instant::now();
            let elapsed = last_fetch.map(|last| now.saturating_duration_since(last));
            if refresh_request.is_some()
                || elapsed.is_none_or(|elapsed| elapsed >= refresh_interval_duration)
            {
                let silent_refresh = !matches!(
                    refresh_request,
                    Some(tray::RefreshRequestOrigin::UserInitiated)
                );
                let fetch_config = load_runtime_config(config_path_bg.as_ref(), &config_bg);
                let results = bg_rt.block_on(fetch_all_providers(&fetch_config));

                // Provider requests can be slow. Reload lightweight presentation
                // and notification settings after they finish so turning
                // notifications off, or changing a budget mid-refresh, takes
                // effect before anything is published.
                let current_config = load_runtime_config(config_path_bg.as_ref(), &fetch_config);
                let current_active_provider =
                    current_config.general.active_provider.trim().to_string();
                *active_provider_bg.write() = current_active_provider;

                publish_provider_results(
                    &current_config,
                    results,
                    &data_bg,
                    &last_refresh_bg,
                    &history_bg,
                    silent_refresh,
                );

                // Regenerate HICON
                let d = data_bg.read();
                let d_resolved: Vec<UsageData> = d
                    .iter()
                    .map(|item| {
                        if item.error.is_some() {
                            if let Some(cached) =
                                history_bg.read().latest_successful_for(&item.provider)
                            {
                                cached
                            } else {
                                item.clone()
                            }
                        } else {
                            item.clone()
                        }
                    })
                    .collect();
                let active_provider_bg = active_provider_bg.read();
                let active_provider = active_provider_option(&active_provider_bg);
                let new_icon = icon::generate_icon(&d_resolved, active_provider);
                let tooltip = icon::tray_tooltip(&d_resolved, active_provider);
                if let Ok(hicon) = new_icon.to_hicon() {
                    tc_bg.update_icon_with_tooltip(hicon, &tooltip);
                }

                // Notify GPUI to redraw
                trigger_gui_update();

                last_fetch = Some(std::time::Instant::now());
                continue;
            }

            let wait_for = refresh_interval_duration.saturating_sub(elapsed.unwrap_or_default());
            tray::wait_for_refresh_or_timeout(wait_for);
        }
    });

    // Spawn network connection status change listener thread
    std::thread::spawn(move || {
        #[link(name = "iphlpapi")]
        unsafe extern "system" {
            fn NotifyAddrChange(
                Handle: *mut windows::Win32::Foundation::HANDLE,
                Overlapped: *const std::ffi::c_void,
            ) -> u32;
        }

        let mut handle = windows::Win32::Foundation::HANDLE::default();
        loop {
            unsafe {
                let res = NotifyAddrChange(&mut handle, std::ptr::null());
                if res == 0 || res == 997 {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if !SYSTEM_SLEEPING.load(Ordering::SeqCst) {
                        tracing::info!("Network change detected, requesting refresh.");
                        tray::request_automatic_refresh();
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                }
            }
        }
    });

    // Run GPUI App on the main thread
    let data_window = data.clone();
    let last_refresh_window = last_refresh.clone();
    let history_window = history.clone();
    let config_window = config.clone();
    let config_path_window = config_path.clone();
    let active_provider_window = active_provider.clone();

    let (update_tx, mut update_rx) = tokio::sync::mpsc::channel::<()>(100);
    let _ = UPDATE_CHANNEL.set(update_tx);

    let app = gpui::Application::new().with_assets(AppAssets);
    app.run(move |cx| {
        gpui_component::init(cx);
        app::apply_component_theme(&config_window.general.theme, None, cx);

        let win_w = 400.0_f32;
        let win_h = 520.0_f32;

        let window_options = gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                origin: gpui::Point {
                    x: gpui::px(0.0),
                    y: gpui::px(0.0),
                },
                size: gpui::size(gpui::px(win_w), gpui::px(win_h)),
            })),
            titlebar: None,
            focus: false,
            // GPUI still creates the native HWND, root view, renderer, and first frame
            // when show is false. Win32 owns every later visibility transition.
            show: false,
            kind: gpui::WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            // GPUI's Transparent mode installs ACCENT_ENABLE_TRANSPARENTGRADIENT on Windows.
            // Keep that legacy effect disabled so transparent DirectComposition pixels reveal
            // the DWM system backdrop instead.
            window_background: gpui::WindowBackgroundAppearance::Opaque,
            ..Default::default()
        };

        let backdrop_dark = match config_window.general.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                cx.window_appearance(),
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            ),
        };
        let backdrop_setting = config_window.general.backdrop.clone();

        let view = cx.new(|cx| {
            app::QuotifyApp::new(
                data_window,
                last_refresh_window,
                config_window,
                config_path_window,
                active_provider_window,
                history_window,
                cx,
            )
        });

        let root_view = view.clone();
        let created_hwnd = Arc::new(OnceLock::new());
        let created_hwnd_for_builder = created_hwnd.clone();
        let win_w = cx
            .open_window(window_options, move |window, cx| {
                // Resolve and prepare the native window, but publish it only after
                // open_window has installed the root and completed its first draw.
                use raw_window_handle::HasWindowHandle;
                if let Ok(handle) = window.window_handle() {
                    if let raw_window_handle::RawWindowHandle::Win32(win32_handle) = handle.as_raw()
                    {
                        let hwnd = HWND(win32_handle.hwnd.get() as *mut std::ffi::c_void);
                        tracing::info!("Successfully resolved Win32 HWND: {:?}", hwnd.0);
                        unsafe {
                            use windows::Win32::UI::Shell::SetWindowSubclass;
                            use windows::Win32::UI::WindowsAndMessaging::SetWindowTextW;
                            let _ = SetWindowTextW(hwnd, windows::core::w!("Quotify"));
                            let _ = SetWindowSubclass(hwnd, Some(main_window_subclass), 1, 0);
                        }
                        let _ = created_hwnd_for_builder.set(tray::SendHWND::new(hwnd));
                    } else {
                        tracing::warn!("Resolved window handle but it is not Win32");
                    }
                } else {
                    tracing::warn!("Failed to obtain window handle from window context");
                }
                cx.new(|cx| gpui_component::Root::new(root_view, window, cx))
            })
            .expect("failed to open window");

        win_w
            .update(cx, |_, window, cx| {
                cx.observe_window_appearance(window, move |_, window, cx| {
                    let theme_setting = app::current_component_theme_setting();
                    if theme_setting == "system" {
                        let dark = matches!(
                            window.appearance(),
                            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                        );
                        refresh_current_backdrop(dark);
                    }
                    app::apply_component_theme(&theme_setting, Some(window), cx);
                    cx.notify();
                })
                .detach();
            })
            .expect("failed to observe window appearance");

        // Operations such as DWM setup synchronously emit native callbacks. Run them
        // only after open_window has completed the root installation and first draw.
        let created_hwnd = created_hwnd.get().copied();
        std::thread::spawn(move || {
            let Some(shwnd) = created_hwnd else {
                tracing::error!("GPUI did not publish a native HWND");
                return;
            };

            let hwnd = shwnd.raw();
            apply_backdrop(hwnd, backdrop_dark, &backdrop_setting);
            // The backdrop becomes active after the first GPUI frame. Repaint immediately so
            // Quotify drops its opaque fallback fill and reveals the DWM backdrop.
            trigger_gui_update();
            apply_rounded_window_region(hwnd);
            let _ = tray::MAIN_HWND.set(shwnd);

            if tray::QUIT_REQUESTED.load(Ordering::SeqCst) {
                let _ = shwnd.post_message(tray::WM_APP_QUIT, WPARAM(0), LPARAM(0));
                return;
            }

            // Give Explorer a short, bounded window to publish the icon rect.
            // On a normal launch this is already available; during login it can
            // lag slightly behind NIM_ADD.
            let icon_rect_deadline = Instant::now() + Duration::from_secs(1);
            while tray_icon_rect().is_none() && Instant::now() < icon_rect_deadline {
                if tray::QUIT_REQUESTED.load(Ordering::SeqCst)
                    || STARTUP_SHOW_CANCELLED.load(Ordering::SeqCst)
                {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            if STARTUP_SHOW_CANCELLED.load(Ordering::SeqCst) {
                return;
            }
            if tray_icon_rect().is_none() {
                tracing::warn!(
                    "Tray icon rectangle was unavailable at startup; using placement fallback"
                );
            }

            // LPARAM(1) marks this as a startup show. It uses the same placement
            // and animation as tray actions, but remains visible if Windows
            // declines to grant foreground activation during login startup.
            if let Err(err) = shwnd.post_message(tray::WM_APP_SHOW, WPARAM(0), LPARAM(1)) {
                tracing::error!("Failed to request the startup flyout: {err}");
            }
        });

        // Spawn a listener task to handle repaint triggers from update channel
        cx.spawn(move |cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                while update_rx.recv().await.is_some() {
                    cx.update(|cx| {
                        win_w
                            .update(cx, |_, _, cx| {
                                cx.notify();
                            })
                            .ok();
                    })
                    .ok();
                }
            }
        })
        .detach();

        // Time-derived labels (for example, "12s ago" and reset countdowns) need an
        // explicit wake-up because GPUI otherwise redraws only in response to input or
        // state notifications. Avoid repainting while the tray popup is hidden.
        let clock_window = win_w;
        cx.spawn(move |cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;

                    if !tray::WINDOW_VISIBLE.load(Ordering::SeqCst)
                        || tray::ACTIVE_PAGE.load(Ordering::SeqCst) != 0
                    {
                        continue;
                    }

                    if cx
                        .update(|cx| {
                            clock_window.update(cx, |_, _, cx| cx.notify()).ok();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        })
        .detach();
    });

    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RectPx {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl RectPx {
    fn width(self) -> i32 {
        self.right - self.left
    }

    fn height(self) -> i32 {
        self.bottom - self.top
    }

    fn center(self) -> [i32; 2] {
        [self.left + self.width() / 2, self.top + self.height() / 2]
    }

    fn intersects(self, other: Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }
}

impl From<windows::Win32::Foundation::RECT> for RectPx {
    fn from(rect: windows::Win32::Foundation::RECT) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskbarEdge {
    Left,
    Top,
    Right,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FlyoutPlacement {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    gap: i32,
    edge: TaskbarEdge,
}

#[derive(Clone, Copy, Debug)]
struct FlyoutAnimation {
    generation: u64,
    phase: FlyoutState,
    from: [i32; 2],
    to: [i32; 2],
    started_at: Instant,
    duration: Duration,
}

#[derive(Default)]
struct FlyoutController {
    placement: Option<FlyoutPlacement>,
    animation: Option<FlyoutAnimation>,
    timer_id: Option<usize>,
    startup_inactive_deadline: Option<Instant>,
    startup_inactive_timer_id: Option<usize>,
    generation: u64,
    quit_after_hide: bool,
}

std::thread_local! {
    static FLYOUT_CONTROLLER: RefCell<FlyoutController> =
        RefCell::new(FlyoutController::default());
}

fn nearest_monitor_edge(monitor: RectPx, anchor: [i32; 2]) -> TaskbarEdge {
    let distances = [
        (anchor[0] - monitor.left).abs(),
        (anchor[1] - monitor.top).abs(),
        (monitor.right - anchor[0]).abs(),
        (monitor.bottom - anchor[1]).abs(),
    ];
    let index = distances
        .iter()
        .enumerate()
        .min_by_key(|(_, distance)| *distance)
        .map(|(index, _)| index)
        .unwrap_or(3);
    match index {
        0 => TaskbarEdge::Left,
        1 => TaskbarEdge::Top,
        2 => TaskbarEdge::Right,
        _ => TaskbarEdge::Bottom,
    }
}

fn taskbar_edge_from_rect(monitor: RectPx, taskbar: RectPx) -> TaskbarEdge {
    if taskbar.width() >= taskbar.height() {
        let top_distance = (taskbar.top - monitor.top).abs();
        let bottom_distance = (monitor.bottom - taskbar.bottom).abs();
        if top_distance <= bottom_distance {
            TaskbarEdge::Top
        } else {
            TaskbarEdge::Bottom
        }
    } else {
        let left_distance = (taskbar.left - monitor.left).abs();
        let right_distance = (monitor.right - taskbar.right).abs();
        if left_distance <= right_distance {
            TaskbarEdge::Left
        } else {
            TaskbarEdge::Right
        }
    }
}

fn taskbar_edge_from_work_area(monitor: RectPx, work_area: RectPx) -> Option<TaskbarEdge> {
    let insets = [
        (work_area.left - monitor.left, TaskbarEdge::Left),
        (work_area.top - monitor.top, TaskbarEdge::Top),
        (monitor.right - work_area.right, TaskbarEdge::Right),
        (monitor.bottom - work_area.bottom, TaskbarEdge::Bottom),
    ];
    insets
        .into_iter()
        .filter(|(inset, _)| *inset > 0)
        .max_by_key(|(inset, _)| *inset)
        .map(|(_, edge)| edge)
}

fn clamp_to_span(value: i32, minimum: i32, maximum: i32) -> i32 {
    if maximum < minimum {
        minimum
    } else {
        value.clamp(minimum, maximum)
    }
}

fn scale_dip(dip: f32, dpi: u32) -> i32 {
    (dip * dpi.max(96) as f32 / 96.0).round() as i32
}

fn layout_flyout(
    monitor: RectPx,
    work_area: RectPx,
    taskbar: Option<RectPx>,
    anchor: [i32; 2],
    popup_size: [i32; 2],
    gap: i32,
    edge: TaskbarEdge,
) -> FlyoutPlacement {
    let taskbar = taskbar.filter(|rect| rect.intersects(monitor));
    let width = popup_size[0].min((work_area.width() - gap * 2).max(1));
    let height = popup_size[1].min((work_area.height() - gap * 2).max(1));

    let x_min = work_area.left + gap;
    let x_max = work_area.right - width - gap;
    let y_min = work_area.top + gap;
    let y_max = work_area.bottom - height - gap;

    let (preferred_x, preferred_y) = match edge {
        TaskbarEdge::Bottom => {
            let boundary = taskbar
                .map(|rect| rect.top)
                .unwrap_or_else(|| work_area.bottom.min(monitor.bottom));
            (anchor[0] - width / 2, boundary - gap - height)
        }
        TaskbarEdge::Top => {
            let boundary = taskbar
                .map(|rect| rect.bottom)
                .unwrap_or_else(|| work_area.top.max(monitor.top));
            (anchor[0] - width / 2, boundary + gap)
        }
        TaskbarEdge::Left => {
            let boundary = taskbar
                .map(|rect| rect.right)
                .unwrap_or_else(|| work_area.left.max(monitor.left));
            (boundary + gap, anchor[1] - height / 2)
        }
        TaskbarEdge::Right => {
            let boundary = taskbar
                .map(|rect| rect.left)
                .unwrap_or_else(|| work_area.right.min(monitor.right));
            (boundary - gap - width, anchor[1] - height / 2)
        }
    };
    let x = clamp_to_span(preferred_x, x_min, x_max);
    let y = clamp_to_span(preferred_y, y_min, y_max);

    FlyoutPlacement {
        x,
        y,
        width,
        height,
        gap,
        edge,
    }
}

fn hidden_flyout_origin(placement: FlyoutPlacement) -> [i32; 2] {
    match placement.edge {
        TaskbarEdge::Bottom => [placement.x, placement.y + placement.height + placement.gap],
        TaskbarEdge::Top => [placement.x, placement.y - placement.height - placement.gap],
        TaskbarEdge::Left => [placement.x - placement.width - placement.gap, placement.y],
        TaskbarEdge::Right => [placement.x + placement.width + placement.gap, placement.y],
    }
}

#[cfg(test)]
mod flyout_layout_tests {
    use super::*;

    const MONITOR: RectPx = RectPx {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };
    const POPUP: [i32; 2] = [400, 520];

    #[test]
    fn positions_flyout_above_bottom_taskbar_with_sixteen_pixel_gap() {
        let placement = layout_flyout(
            MONITOR,
            RectPx {
                bottom: 1040,
                ..MONITOR
            },
            Some(RectPx {
                left: 0,
                top: 1040,
                right: 1920,
                bottom: 1080,
            }),
            [1880, 1060],
            POPUP,
            16,
            TaskbarEdge::Bottom,
        );

        assert_eq!((placement.x, placement.y), (1504, 504));
    }

    #[test]
    fn positions_flyout_for_top_left_and_right_taskbars() {
        let top = layout_flyout(
            MONITOR,
            RectPx { top: 40, ..MONITOR },
            Some(RectPx {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 40,
            }),
            [1000, 20],
            POPUP,
            16,
            TaskbarEdge::Top,
        );
        assert_eq!((top.x, top.y), (800, 56));

        let left = layout_flyout(
            MONITOR,
            RectPx {
                left: 48,
                ..MONITOR
            },
            Some(RectPx {
                left: 0,
                top: 0,
                right: 48,
                bottom: 1080,
            }),
            [24, 540],
            POPUP,
            16,
            TaskbarEdge::Left,
        );
        assert_eq!((left.x, left.y), (64, 280));

        let right = layout_flyout(
            MONITOR,
            RectPx {
                right: 1872,
                ..MONITOR
            },
            Some(RectPx {
                left: 1872,
                top: 0,
                right: 1920,
                bottom: 1080,
            }),
            [1896, 540],
            POPUP,
            16,
            TaskbarEdge::Right,
        );
        assert_eq!((right.x, right.y), (1456, 280));
    }

    #[test]
    fn preserves_negative_virtual_screen_coordinates() {
        let monitor = RectPx {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        let placement = layout_flyout(
            monitor,
            RectPx {
                bottom: 1040,
                ..monitor
            },
            Some(RectPx {
                left: -1920,
                top: 1040,
                right: 0,
                bottom: 1080,
            }),
            [-40, 1060],
            POPUP,
            16,
            TaskbarEdge::Bottom,
        );

        assert_eq!((placement.x, placement.y), (-416, 504));
    }

    #[test]
    fn keeps_the_taskbar_gap_at_sixteen_physical_pixels_for_every_dpi() {
        let monitor = RectPx {
            left: 0,
            top: 0,
            right: 3840,
            bottom: 2160,
        };
        let work_area = RectPx {
            bottom: 2080,
            ..monitor
        };
        let taskbar = RectPx {
            left: 0,
            top: 2080,
            right: 3840,
            bottom: 2160,
        };

        for dpi in [96, 120, 144, 192] {
            let placement = layout_flyout(
                monitor,
                work_area,
                Some(taskbar),
                [3800, 2120],
                [
                    scale_dip(FLYOUT_WIDTH_DIP, dpi),
                    scale_dip(FLYOUT_HEIGHT_DIP, dpi),
                ],
                FLYOUT_GAP_PX,
                TaskbarEdge::Bottom,
            );
            assert_eq!(taskbar.top - (placement.y + placement.height), 16);
        }
    }

    #[test]
    fn derives_taskbar_edges_from_the_returned_rectangle() {
        assert_eq!(
            taskbar_edge_from_rect(
                MONITOR,
                RectPx {
                    right: 48,
                    ..MONITOR
                }
            ),
            TaskbarEdge::Left
        );
        assert_eq!(
            taskbar_edge_from_rect(
                MONITOR,
                RectPx {
                    bottom: 40,
                    ..MONITOR
                }
            ),
            TaskbarEdge::Top
        );
        assert_eq!(
            taskbar_edge_from_rect(
                MONITOR,
                RectPx {
                    left: 1872,
                    ..MONITOR
                }
            ),
            TaskbarEdge::Right
        );
        assert_eq!(
            taskbar_edge_from_rect(
                MONITOR,
                RectPx {
                    top: 1040,
                    ..MONITOR
                }
            ),
            TaskbarEdge::Bottom
        );
    }

    #[test]
    fn hides_the_window_fully_behind_each_taskbar_edge() {
        let placement = |edge| FlyoutPlacement {
            x: 100,
            y: 200,
            width: 400,
            height: 520,
            gap: 16,
            edge,
        };

        assert_eq!(
            hidden_flyout_origin(placement(TaskbarEdge::Bottom)),
            [100, 736]
        );
        assert_eq!(
            hidden_flyout_origin(placement(TaskbarEdge::Top)),
            [100, -336]
        );
        assert_eq!(
            hidden_flyout_origin(placement(TaskbarEdge::Left)),
            [-316, 200]
        );
        assert_eq!(
            hidden_flyout_origin(placement(TaskbarEdge::Right)),
            [516, 200]
        );
    }

    #[test]
    fn preserves_animation_timing_and_shortens_reversals() {
        assert_eq!(
            remaining_animation_duration([0, 0], [0, 536], 536, FLYOUT_SHOW_DURATION),
            Duration::from_millis(180)
        );
        assert_eq!(
            remaining_animation_duration([0, 268], [0, 536], 536, FLYOUT_SHOW_DURATION),
            Duration::from_millis(90)
        );
        assert_eq!(
            remaining_animation_duration([0, 536], [0, 0], 536, FLYOUT_HIDE_DURATION),
            Duration::from_millis(150)
        );
    }

    #[test]
    fn animation_curves_keep_exact_endpoints() {
        assert_eq!(animation_easing(FlyoutState::Showing, 0.0), 0.0);
        assert_eq!(animation_easing(FlyoutState::Showing, 1.0), 1.0);
        assert_eq!(animation_easing(FlyoutState::Hiding, 0.0), 0.0);
        assert_eq!(animation_easing(FlyoutState::Hiding, 1.0), 1.0);
        assert_eq!(interpolate_origin([10, 20], [110, 220], 0.0), [10, 20]);
        assert_eq!(interpolate_origin([10, 20], [110, 220], 1.0), [110, 220]);
    }

    #[test]
    fn auto_hidden_taskbar_uses_the_monitor_edge() {
        let placement = layout_flyout(
            MONITOR,
            MONITOR,
            None,
            [960, 1079],
            POPUP,
            16,
            TaskbarEdge::Bottom,
        );

        assert_eq!((placement.x, placement.y), (760, 544));
    }

    #[test]
    fn oversized_flyout_does_not_panic() {
        let placement = layout_flyout(
            RectPx {
                left: 0,
                top: 0,
                right: 300,
                bottom: 200,
            },
            RectPx {
                left: 0,
                top: 0,
                right: 300,
                bottom: 160,
            },
            Some(RectPx {
                left: 0,
                top: 160,
                right: 300,
                bottom: 200,
            }),
            [280, 180],
            POPUP,
            16,
            TaskbarEdge::Bottom,
        );

        assert_eq!(
            (placement.x, placement.y, placement.width, placement.height),
            (16, 16, 268, 128)
        );
    }
}

fn tray_icon_rect() -> Option<RectPx> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};

        let shwnd = crate::tray::TRAY_HWND.get().copied()?;
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: shwnd.raw(),
            uID: 1,
            guidItem: Default::default(),
        };
        unsafe { Shell_NotifyIconGetRect(&identifier).ok().map(Into::into) }
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

fn system_taskbar() -> Option<RectPx> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};

        let mut data = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        if unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) } == 0 {
            return None;
        }
        Some(data.rc.into())
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

fn compute_popup_placement(hwnd: HWND) -> FlyoutPlacement {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
        };
        use windows::Win32::UI::HiDpi::GetDpiForWindow;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
        };

        let icon = tray_icon_rect();
        let mut anchor = icon.map(RectPx::center).unwrap_or([0, 0]);
        if icon.is_none() {
            let mut cursor = POINT::default();
            unsafe {
                if GetCursorPos(&mut cursor).is_ok() {
                    anchor = [cursor.x, cursor.y];
                }
            }
        }

        let monitor_handle = unsafe {
            MonitorFromPoint(
                POINT {
                    x: anchor[0],
                    y: anchor[1],
                },
                MONITOR_DEFAULTTONEAREST,
            )
        };
        let mut monitor_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor_handle, &mut monitor_info) }.as_bool() {
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            let width = scale_dip(FLYOUT_WIDTH_DIP, dpi);
            let height = scale_dip(FLYOUT_HEIGHT_DIP, dpi);
            let gap = FLYOUT_GAP_PX;
            return FlyoutPlacement {
                x: anchor[0] - width / 2,
                y: anchor[1] - height - gap,
                width,
                height,
                gap,
                edge: TaskbarEdge::Bottom,
            };
        }

        let monitor: RectPx = monitor_info.rcMonitor.into();
        let work_area: RectPx = monitor_info.rcWork.into();

        // Move the still-hidden window onto the tray icon's monitor before asking
        // Windows for its DPI. GPUI receives WM_DPICHANGED synchronously here.
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                work_area.left,
                work_area.top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let popup_size = [
            scale_dip(FLYOUT_WIDTH_DIP, dpi),
            scale_dip(FLYOUT_HEIGHT_DIP, dpi),
        ];
        let gap = FLYOUT_GAP_PX;

        // Notification icons in the overflow flyout are not physically inside
        // the taskbar rectangle, but they still belong to the system taskbar.
        let taskbar = system_taskbar().filter(|rect| rect.intersects(monitor));
        let edge = taskbar
            .map(|rect| taskbar_edge_from_rect(monitor, rect))
            .or_else(|| {
                icon.is_none()
                    .then(|| taskbar_edge_from_work_area(monitor, work_area))
                    .flatten()
            })
            .unwrap_or_else(|| nearest_monitor_edge(monitor, anchor));

        layout_flyout(monitor, work_area, taskbar, anchor, popup_size, gap, edge)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
        FlyoutPlacement {
            x: 100,
            y: 100,
            width: FLYOUT_WIDTH_DIP as i32,
            height: FLYOUT_HEIGHT_DIP as i32,
            gap: FLYOUT_GAP_PX,
            edge: TaskbarEdge::Bottom,
        }
    }
}

fn apply_backdrop(hwnd: HWND, dark: bool, setting: &str) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Graphics::Dwm::{
            DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TABBEDWINDOW, DWMSBT_TRANSIENTWINDOW,
            DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea,
            DwmSetWindowAttribute,
        };
        use windows::Win32::UI::Controls::MARGINS;

        let setting = normalize_backdrop_setting(setting);
        *BACKDROP_SETTING
            .get_or_init(|| RwLock::new("mica_alt".to_string()))
            .write() = setting.to_string();
        BACKDROP_DARK_MODE.store(dark, std::sync::atomic::Ordering::SeqCst);
        IS_BACKDROP_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
        if hwnd.0.is_null() {
            tracing::warn!("apply_backdrop called with null HWND");
            return;
        }

        let dark_mode = if dark { 1_i32 } else { 0_i32 };
        let margins = if setting == "none" {
            MARGINS::default()
        } else {
            MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            }
        };

        unsafe {
            let dark_result = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark_mode as *const _ as *const _,
                std::mem::size_of_val(&dark_mode) as u32,
            );
            if let Err(err) = dark_result {
                tracing::warn!("Failed to set backdrop theme: {err}");
            }

            let frame_result = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let (backdrop_type, backdrop_name) = match setting {
                "mica" => (DWMSBT_MAINWINDOW.0, "Mica"),
                "acrylic" => (DWMSBT_TRANSIENTWINDOW.0, "Acrylic"),
                "none" => (DWMSBT_NONE.0, "None"),
                _ => (DWMSBT_TABBEDWINDOW.0, "Mica Alt"),
            };
            let backdrop_result = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &backdrop_type as *const _ as *const _,
                std::mem::size_of_val(&backdrop_type) as u32,
            );

            // Windows 11 build 22000 predates DWMWA_SYSTEMBACKDROP_TYPE. Its former
            // private attribute can enable/disable Mica, but has no Acrylic equivalent.
            let effect_result = match setting {
                "mica" | "mica_alt" => backdrop_result.or_else(|_| {
                    let mica_attribute = windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(1029);
                    let enabled = 1_i32;
                    DwmSetWindowAttribute(
                        hwnd,
                        mica_attribute,
                        &enabled as *const _ as *const _,
                        std::mem::size_of_val(&enabled) as u32,
                    )
                }),
                "none" => backdrop_result.or_else(|_| {
                    let mica_attribute = windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(1029);
                    let disabled = 0_i32;
                    DwmSetWindowAttribute(
                        hwnd,
                        mica_attribute,
                        &disabled as *const _ as *const _,
                        std::mem::size_of_val(&disabled) as u32,
                    )
                }),
                _ => backdrop_result,
            };

            match (frame_result, effect_result) {
                (Ok(()), Ok(())) => {
                    if setting == "none" {
                        tracing::info!("DWM backdrop disabled for HWND {:?}", hwnd.0);
                    } else {
                        IS_BACKDROP_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
                        tracing::info!("{backdrop_name} backdrop enabled for HWND {:?}", hwnd.0);
                    }
                }
                (frame, effect) => {
                    tracing::warn!(
                        "{backdrop_name} backdrop unavailable (extend frame: {frame:?}, backdrop: {effect:?})"
                    );
                }
            }
        }
    }
}

pub(crate) fn refresh_backdrop(dark: bool, setting: &str) {
    if let Some(hwnd) = tray::MAIN_HWND.get() {
        apply_backdrop(hwnd.raw(), dark, setting);
        trigger_gui_update();
    }
}

fn refresh_current_backdrop(dark: bool) {
    let setting = current_backdrop_setting();
    refresh_backdrop(dark, &setting);
}
fn apply_rounded_window_region(hwnd: HWND) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Graphics::Dwm::{
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
        };

        if !hwnd.0.is_null() {
            let preference = DWMWCP_ROUND.0;
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &preference as *const _ as *const _,
                    std::mem::size_of::<i32>() as u32,
                );
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
    }
}

unsafe extern "system" fn main_window_subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _ref_data: usize,
) -> LRESULT {
    unsafe {
        use windows::Win32::UI::Shell::DefSubclassProc;
        use windows::Win32::UI::WindowsAndMessaging::{
            WA_INACTIVE, WM_ACTIVATE, WM_CLOSE, WM_DESTROY, WM_DWMCOMPOSITIONCHANGED, WM_KEYDOWN,
            WM_SIZE, WM_TIMER,
        };

        match msg {
            tray::WM_APP_TOGGLE => {
                STARTUP_SHOW_CANCELLED.store(true, Ordering::SeqCst);
                if matches!(flyout_state(), FlyoutState::Showing | FlyoutState::Visible) {
                    hide_popup_window(hwnd);
                } else {
                    tray::ACTIVE_PAGE.store(wparam.0 as u32, Ordering::SeqCst);
                    show_popup_window(hwnd, false);
                    trigger_gui_update();
                }
                LRESULT(0)
            }
            tray::WM_APP_SHOW => {
                let startup_show = lparam.0 != 0;
                if startup_show && STARTUP_SHOW_CANCELLED.load(Ordering::SeqCst) {
                    return LRESULT(0);
                }
                if !startup_show {
                    STARTUP_SHOW_CANCELLED.store(true, Ordering::SeqCst);
                }
                tray::ACTIVE_PAGE.store(wparam.0 as u32, Ordering::SeqCst);
                show_popup_window(hwnd, startup_show);
                trigger_gui_update();
                LRESULT(0)
            }
            tray::WM_APP_HIDE => {
                STARTUP_SHOW_CANCELLED.store(true, Ordering::SeqCst);
                hide_popup_window(hwnd);
                LRESULT(0)
            }
            WM_ACTIVATE => {
                let active_state = (wparam.0 & 0xFFFF) as u32;
                if active_state == WA_INACTIVE {
                    if matches!(flyout_state(), FlyoutState::Showing | FlyoutState::Visible)
                        && !startup_flyout_is_inactive()
                        && !is_pointer_on_tray_icon()
                    {
                        hide_popup_window(hwnd);
                    }
                } else {
                    promote_activated_startup_flyout(hwnd);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_KEYDOWN
                if wparam.0
                    == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 as usize =>
            {
                hide_popup_window(hwnd);
                LRESULT(0)
            }
            WM_CLOSE => {
                hide_popup_window(hwnd);
                LRESULT(0)
            }
            tray::WM_APP_UPDATE_DATA => {
                if tray::WINDOW_VISIBLE.load(Ordering::SeqCst) {
                    trigger_gui_update();
                }
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_POWERBROADCAST => {
                let power_event = wparam.0 as u32;
                if power_event == 4 {
                    // PBT_APMSUSPEND
                    tracing::info!("System is suspending (sleeping). Pausing refresh.");
                    SYSTEM_SLEEPING.store(true, Ordering::SeqCst);
                } else if power_event == 18 {
                    // PBT_APMRESUMEAUTOMATIC
                    tracing::info!("System resumed from sleep. Triggering refresh.");
                    SYSTEM_SLEEPING.store(false, Ordering::SeqCst);
                    tray::request_automatic_refresh();
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_DWMCOMPOSITIONCHANGED => {
                let setting = current_backdrop_setting();
                apply_backdrop(hwnd, BACKDROP_DARK_MODE.load(Ordering::SeqCst), &setting);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_SIZE => {
                apply_rounded_window_region(hwnd);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_TIMER
                if handle_flyout_timer(hwnd, wparam.0)
                    || handle_startup_inactive_timer(hwnd, wparam.0) =>
            {
                LRESULT(0)
            }
            tray::WM_APP_QUIT => {
                quit_popup_window(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                reset_flyout_controller(hwnd);
                set_flyout_state(FlyoutState::Hidden);
                let _ = windows::Win32::UI::Shell::RemoveWindowSubclass(
                    hwnd,
                    Some(main_window_subclass),
                    1,
                );
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

fn activate_popup_window(hwnd: HWND) -> bool {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        GetForegroundWindow() == hwnd
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
        true
    }
}

fn movement_distance(from: [i32; 2], to: [i32; 2]) -> i32 {
    (from[0] - to[0]).abs().max((from[1] - to[1]).abs())
}

fn remaining_animation_duration(
    from: [i32; 2],
    to: [i32; 2],
    full_distance: i32,
    full_duration: Duration,
) -> Duration {
    let remaining = movement_distance(from, to);
    if remaining == 0 {
        return Duration::from_millis(1);
    }

    let ratio = remaining as f64 / full_distance.max(1) as f64;
    let millis = (full_duration.as_millis() as f64 * ratio)
        .round()
        .clamp(1.0, full_duration.as_millis() as f64) as u64;
    Duration::from_millis(millis)
}

fn animation_easing(phase: FlyoutState, progress: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    match phase {
        FlyoutState::Showing => 1.0 - (1.0 - progress).powi(3),
        FlyoutState::Hiding => progress.powi(3),
        _ => progress,
    }
}

fn interpolate_origin(from: [i32; 2], to: [i32; 2], progress: f32) -> [i32; 2] {
    [
        (from[0] as f32 + (to[0] - from[0]) as f32 * progress).round() as i32,
        (from[1] as f32 + (to[1] - from[1]) as f32 * progress).round() as i32,
    ]
}

fn current_window_origin(hwnd: HWND, fallback: [i32; 2]) -> [i32; 2] {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
            return [rect.left, rect.top];
        }
    }

    let _ = hwnd;
    fallback
}

fn begin_flyout_animation(phase: FlyoutState, from: [i32; 2], to: [i32; 2], full_distance: i32) {
    let full_duration = match phase {
        FlyoutState::Showing => FLYOUT_SHOW_DURATION,
        FlyoutState::Hiding => FLYOUT_HIDE_DURATION,
        _ => Duration::from_millis(1),
    };
    let duration = remaining_animation_duration(from, to, full_distance, full_duration);

    FLYOUT_CONTROLLER.with(|controller| {
        let mut controller = controller.borrow_mut();
        controller.generation = controller.generation.wrapping_add(1);
        controller.animation = Some(FlyoutAnimation {
            generation: controller.generation,
            phase,
            from,
            to,
            started_at: Instant::now(),
            duration,
        });
    });
}

fn ensure_flyout_timer(hwnd: HWND) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::SetTimer;

        if FLYOUT_CONTROLLER.with(|controller| controller.borrow().timer_id.is_some()) {
            return true;
        }

        let requested_id = NEXT_FLYOUT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
        let timer_id =
            unsafe { SetTimer(Some(hwnd), requested_id, FLYOUT_TIMER_INTERVAL_MS, None) };
        if timer_id == 0 {
            tracing::error!("Failed to create flyout animation timer");
            return false;
        }
        FLYOUT_CONTROLLER.with(|controller| {
            controller.borrow_mut().timer_id = Some(timer_id);
        });
        true
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
        false
    }
}

fn startup_flyout_is_inactive() -> bool {
    FLYOUT_CONTROLLER.with(|controller| controller.borrow().startup_inactive_deadline.is_some())
}

fn arm_startup_inactive_timer(hwnd: HWND) -> bool {
    let already_armed = FLYOUT_CONTROLLER.with(|controller| {
        let mut controller = controller.borrow_mut();
        controller.startup_inactive_deadline = Some(Instant::now() + STARTUP_INACTIVE_TIMEOUT);
        controller.startup_inactive_timer_id.is_some()
    });
    if already_armed {
        return true;
    }

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::SetTimer;

        let requested_id = NEXT_FLYOUT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
        let timer_id = unsafe {
            SetTimer(
                Some(hwnd),
                requested_id,
                STARTUP_INACTIVE_TIMER_INTERVAL_MS,
                None,
            )
        };
        if timer_id == 0 {
            FLYOUT_CONTROLLER.with(|controller| {
                controller.borrow_mut().startup_inactive_deadline = None;
            });
            tracing::error!("Failed to create startup flyout timeout timer");
            return false;
        }
        FLYOUT_CONTROLLER.with(|controller| {
            controller.borrow_mut().startup_inactive_timer_id = Some(timer_id);
        });
        true
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
        true
    }
}

fn clear_startup_inactive_timer(hwnd: HWND) -> bool {
    let (timer_id, was_inactive) = FLYOUT_CONTROLLER.with(|controller| {
        let mut controller = controller.borrow_mut();
        let timer_id = controller.startup_inactive_timer_id.take();
        let was_inactive = controller.startup_inactive_deadline.take().is_some();
        (timer_id, was_inactive)
    });

    #[cfg(target_os = "windows")]
    if let Some(timer_id) = timer_id {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::KillTimer;
            let _ = KillTimer(Some(hwnd), timer_id);
        }
    }

    #[cfg(not(target_os = "windows"))]
    let _ = (hwnd, timer_id);

    was_inactive
}

fn promote_activated_startup_flyout(hwnd: HWND) {
    if clear_startup_inactive_timer(hwnd) && flyout_state() == FlyoutState::Visible {
        set_popup_topmost(hwnd, true);
    }
}

fn handle_startup_inactive_timer(hwnd: HWND, timer_id: usize) -> bool {
    let deadline = FLYOUT_CONTROLLER.with(|controller| {
        let controller = controller.borrow();
        if controller.startup_inactive_timer_id == Some(timer_id) {
            controller.startup_inactive_deadline
        } else {
            None
        }
    });
    let Some(deadline) = deadline else {
        return false;
    };

    #[cfg(target_os = "windows")]
    if unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } == hwnd {
        promote_activated_startup_flyout(hwnd);
        return true;
    }

    if Instant::now() < deadline {
        return true;
    }

    clear_startup_inactive_timer(hwnd);
    if matches!(flyout_state(), FlyoutState::Showing | FlyoutState::Visible) {
        tracing::debug!("Auto-hiding an inactive startup flyout");
        hide_popup_window(hwnd);
    }
    true
}

fn set_popup_topmost(hwnd: HWND, topmost: bool) {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        };

        let insert_after = if topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        let _ = SetWindowPos(
            hwnd,
            Some(insert_after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (hwnd, topmost);
    }
}

fn animation_generation_is_current(generation: u64) -> bool {
    FLYOUT_CONTROLLER.with(|controller| controller.borrow().generation == generation)
}

fn finalize_flyout_animation(hwnd: HWND, animation: FlyoutAnimation) {
    if !animation_generation_is_current(animation.generation) {
        return;
    }

    match animation.phase {
        FlyoutState::Showing => {
            // A startup flyout that Windows would not activate stays
            // non-topmost until the user focuses it, and is auto-hidden by its
            // bounded timeout.
            set_popup_topmost(hwnd, !startup_flyout_is_inactive());
            if animation_generation_is_current(animation.generation) {
                set_flyout_state(FlyoutState::Visible);
                tracing::debug!("Flyout show animation completed");
            }
        }
        FlyoutState::Hiding => {
            #[cfg(target_os = "windows")]
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            set_popup_topmost(hwnd, false);

            if !animation_generation_is_current(animation.generation) {
                return;
            }
            let quit_after_hide = FLYOUT_CONTROLLER.with(|controller| {
                let mut controller = controller.borrow_mut();
                let quit_after_hide = controller.quit_after_hide;
                controller.placement = None;
                controller.quit_after_hide = false;
                quit_after_hide
            });
            set_flyout_state(FlyoutState::Hidden);
            tracing::debug!("Flyout hide animation completed");

            if quit_after_hide {
                #[cfg(target_os = "windows")]
                unsafe {
                    if let Err(err) = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd) {
                        tracing::error!("Failed to destroy flyout after exit animation: {err}");
                    }
                }
            }
        }
        _ => {}
    }
}

fn finish_flyout_animation_now(hwnd: HWND) {
    let (timer_id, animation) = FLYOUT_CONTROLLER.with(|controller| {
        let mut controller = controller.borrow_mut();
        (controller.timer_id.take(), controller.animation.take())
    });

    #[cfg(target_os = "windows")]
    if let Some(timer_id) = timer_id {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::KillTimer;
            let _ = KillTimer(Some(hwnd), timer_id);
        }
    }

    let Some(animation) = animation else {
        return;
    };
    #[cfg(target_os = "windows")]
    let move_result = unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
        };
        SetWindowPos(
            hwnd,
            None,
            animation.to[0],
            animation.to[1],
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
    #[cfg(target_os = "windows")]
    if let Err(err) = move_result {
        tracing::error!("Failed to finish flyout animation: {err}");
        let destroy_after_hide =
            FLYOUT_CONTROLLER.with(|controller| controller.borrow().quit_after_hide);
        hide_popup_immediately(hwnd, destroy_after_hide);
        return;
    }
    finalize_flyout_animation(hwnd, animation);
}

fn handle_flyout_timer(hwnd: HWND, timer_id: usize) -> bool {
    let frame = FLYOUT_CONTROLLER.with(|controller| {
        let mut controller = controller.borrow_mut();
        if controller.timer_id != Some(timer_id) {
            return None;
        }
        let animation = controller.animation?;
        let progress = (animation.started_at.elapsed().as_secs_f32()
            / animation.duration.as_secs_f32())
        .clamp(0.0, 1.0);
        let origin = interpolate_origin(
            animation.from,
            animation.to,
            animation_easing(animation.phase, progress),
        );
        let complete = progress >= 1.0;
        if complete {
            controller.animation = None;
            controller.timer_id = None;
        }
        Some((animation, origin, complete))
    });

    let Some((animation, origin, complete)) = frame else {
        return false;
    };

    #[cfg(target_os = "windows")]
    let move_result = unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            KillTimer, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
        };
        if complete {
            let _ = KillTimer(Some(hwnd), timer_id);
        }
        SetWindowPos(
            hwnd,
            None,
            origin[0],
            origin[1],
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };

    #[cfg(target_os = "windows")]
    if let Err(err) = move_result {
        tracing::error!("Failed to move flyout animation frame: {err}");
        let destroy_after_hide =
            FLYOUT_CONTROLLER.with(|controller| controller.borrow().quit_after_hide);
        hide_popup_immediately(hwnd, destroy_after_hide);
        return true;
    }

    if complete {
        finalize_flyout_animation(hwnd, animation);
    }
    true
}

fn show_popup_window(hwnd: HWND, startup_show: bool) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos,
        };

        let quitting = FLYOUT_CONTROLLER.with(|controller| controller.borrow().quit_after_hide);
        if quitting {
            return;
        }

        let startup_timeout_armed = if startup_show {
            arm_startup_inactive_timer(hwnd)
        } else {
            clear_startup_inactive_timer(hwnd);
            false
        };

        if matches!(flyout_state(), FlyoutState::Showing | FlyoutState::Visible) {
            if activate_popup_window(hwnd) {
                promote_activated_startup_flyout(hwnd);
            } else if !startup_timeout_armed {
                hide_popup_window(hwnd);
            }
            return;
        }

        let state = flyout_state();
        let placement = if state == FlyoutState::Hiding {
            FLYOUT_CONTROLLER
                .with(|controller| controller.borrow().placement)
                .unwrap_or_else(|| compute_popup_placement(hwnd))
        } else {
            compute_popup_placement(hwnd)
        };
        let final_origin = [placement.x, placement.y];
        let hidden_origin = hidden_flyout_origin(placement);
        let from = if state == FlyoutState::Hidden {
            hidden_origin
        } else {
            current_window_origin(hwnd, hidden_origin)
        };

        tracing::debug!(
            "Showing flyout at ({}, {}) size {}x{} with {}px gap from {:?} taskbar",
            placement.x,
            placement.y,
            placement.width,
            placement.height,
            placement.gap,
            placement.edge
        );

        FLYOUT_CONTROLLER.with(|controller| {
            let mut controller = controller.borrow_mut();
            controller.placement = Some(placement);
            controller.quit_after_hide = false;
        });
        begin_flyout_animation(
            FlyoutState::Showing,
            from,
            final_origin,
            movement_distance(hidden_origin, final_origin),
        );
        set_flyout_state(FlyoutState::Showing);

        let result = unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                from[0],
                from[1],
                placement.width,
                placement.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
        if let Err(err) = result {
            tracing::error!("Failed to show flyout: {err}");
            hide_popup_immediately(hwnd, false);
            return;
        }

        apply_rounded_window_region(hwnd);
        if !ensure_flyout_timer(hwnd) {
            finish_flyout_animation_now(hwnd);
        }
        if activate_popup_window(hwnd) {
            promote_activated_startup_flyout(hwnd);
        } else if startup_timeout_armed {
            tracing::info!(
                "Windows declined startup flyout activation; showing it without focus for up to {}s",
                STARTUP_INACTIVE_TIMEOUT.as_secs()
            );
        } else {
            tracing::warn!("Windows refused to activate the flyout; hiding it again");
            hide_popup_window(hwnd);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
    }
}

fn is_pointer_on_tray_icon() -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let Some(rect) = tray_icon_rect() else {
            return false;
        };

        let mut pt = POINT { x: 0, y: 0 };
        unsafe {
            if GetCursorPos(&mut pt).is_err() {
                return false;
            }

            let padding = 6;
            pt.x >= rect.left - padding
                && pt.x <= rect.right + padding
                && pt.y >= rect.top - padding
                && pt.y <= rect.bottom + padding
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

fn hide_popup_window(hwnd: HWND) {
    clear_startup_inactive_timer(hwnd);

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        };

        if matches!(flyout_state(), FlyoutState::Hidden | FlyoutState::Hiding) {
            return;
        }

        let placement = FLYOUT_CONTROLLER
            .with(|controller| controller.borrow().placement)
            .unwrap_or_else(|| compute_popup_placement(hwnd));
        let final_origin = [placement.x, placement.y];
        let hidden_origin = hidden_flyout_origin(placement);
        let from = current_window_origin(hwnd, final_origin);
        tracing::debug!(
            "Hiding flyout toward ({}, {}) behind {:?} taskbar",
            hidden_origin[0],
            hidden_origin[1],
            placement.edge
        );

        FLYOUT_CONTROLLER.with(|controller| {
            controller.borrow_mut().placement = Some(placement);
        });
        begin_flyout_animation(
            FlyoutState::Hiding,
            from,
            hidden_origin,
            movement_distance(final_origin, hidden_origin),
        );
        set_flyout_state(FlyoutState::Hiding);
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        if !ensure_flyout_timer(hwnd) {
            finish_flyout_animation_now(hwnd);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
    }
}

fn hide_popup_immediately(hwnd: HWND, destroy_after_hide: bool) {
    clear_startup_inactive_timer(hwnd);

    let timer_id = FLYOUT_CONTROLLER.with(|controller| {
        let mut controller = controller.borrow_mut();
        controller.generation = controller.generation.wrapping_add(1);
        controller.animation = None;
        controller.placement = None;
        controller.quit_after_hide = false;
        controller.timer_id.take()
    });

    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, KillTimer, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SetWindowPos, ShowWindow,
        };

        if let Some(timer_id) = timer_id {
            let _ = KillTimer(Some(hwnd), timer_id);
        }
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }

    set_flyout_state(FlyoutState::Hidden);

    if destroy_after_hide {
        #[cfg(target_os = "windows")]
        unsafe {
            if let Err(err) = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd) {
                tracing::error!("Failed to destroy hidden flyout: {err}");
            }
        }
    }
}

fn quit_popup_window(hwnd: HWND) {
    if flyout_state() == FlyoutState::Hidden {
        hide_popup_immediately(hwnd, true);
        return;
    }

    FLYOUT_CONTROLLER.with(|controller| {
        controller.borrow_mut().quit_after_hide = true;
    });
    if flyout_state() != FlyoutState::Hiding {
        hide_popup_window(hwnd);
    }
}

fn reset_flyout_controller(hwnd: HWND) {
    clear_startup_inactive_timer(hwnd);

    let timer_id = FLYOUT_CONTROLLER.with(|controller| {
        let mut controller = controller.borrow_mut();
        let timer_id = controller.timer_id.take();
        *controller = FlyoutController::default();
        timer_id
    });

    #[cfg(target_os = "windows")]
    if let Some(timer_id) = timer_id {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::KillTimer;
            let _ = KillTimer(Some(hwnd), timer_id);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (hwnd, timer_id);
    }
}
pub mod webview_login;
