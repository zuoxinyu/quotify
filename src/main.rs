#![recursion_limit = "1024"]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(test))]
mod app;
mod config;
mod diagnostics;
mod icon;
mod provider;
mod secrets;
mod single_instance;
mod startup;
mod tray;
mod usage_history;
mod version;

use gpui::prelude::*;
use anyhow::Result;
use clap::{Parser, Subcommand};
use parking_lot::{Mutex, RwLock};
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
    sync::{Arc, OnceLock, atomic::Ordering},
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

pub static IS_MICA_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static MICA_DARK_MODE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
pub static SYSTEM_SLEEPING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static IGNORE_INACTIVE_UNTIL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
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

fn inactive_guard() -> &'static Mutex<Option<Instant>> {
    IGNORE_INACTIVE_UNTIL.get_or_init(|| Mutex::new(None))
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
                Some(Box::new(BedrockProvider::new(
                    config.bedrock.api_key.clone(),
                )))
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

async fn fetch_all_providers(
    config: &config::AppConfig,
    data: Arc<RwLock<Vec<UsageData>>>,
    last_refresh: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
    history: Arc<RwLock<usage_history::UsageHistory>>,
) {
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

    let results = fetch_providers(config, provider_names).await;

    *data.write() = results;
    *last_refresh.write() = chrono::Utc::now();
    {
        let mut history = history.write();
        history.append(data.read().clone());
        if let Err(err) = history.save() {
            tracing::error!("Failed to save usage history: {err}");
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
                            tracing::error!("Quotify is already running, but could not activate the existing instance: {err}");
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

    let results = fetch_providers(config, provider_names).await;

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
        if !names.iter().any(|existing| existing == name)
            && create_provider(name, config).is_some()
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
                tracing::error!("Failed to fetch {}: {}", name, e);
                results.push(provider_error_data(name, e.to_string()));
            }
            Err(e) => {
                tracing::error!("Failed to join {} fetch task: {}", name, e);
                results.push(provider_error_data(name, e.to_string()));
            }
        }
    }

    results
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
        let tray_controller = Arc::new(tray::TrayController::new().expect("Failed to create tray controller"));
        tray_tx.send(tray_controller.clone()).unwrap();

        // Set initial loading icon before data is fetched
        let (initial_icon, tooltip) = {
            let d = data_bg_tray.read();
            let d_resolved: Vec<UsageData> = d.iter().map(|item| {
                if item.error.is_some() {
                    if let Some(cached) = history_bg_tray.read().latest_successful_for(&item.provider) {
                        cached
                    } else {
                        item.clone()
                    }
                } else {
                    item.clone()
                }
            }).collect();
            let active_provider = active_provider_bg_tray.read();
            let icon = icon::generate_icon(&d_resolved, active_provider_option(&active_provider));
            let tooltip = icon::tray_tooltip(&d_resolved, active_provider_option(&active_provider));
            (icon, tooltip)
        };
        if let Ok(hicon) = initial_icon.to_hicon() {
            tray_controller.update_icon_with_tooltip(hicon, &tooltip);
        }

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
            let forced = tray::REFRESH_REQUESTED.swap(false, Ordering::SeqCst);
            let now = std::time::Instant::now();
            let elapsed = last_fetch.map(|last| now.saturating_duration_since(last));
            if forced || elapsed.is_none_or(|elapsed| elapsed >= refresh_interval_duration) {
                let current_config = load_runtime_config(config_path_bg.as_ref(), &config_bg);
                let current_active_provider =
                    current_config.general.active_provider.trim().to_string();
                *active_provider_bg.write() = current_active_provider;

                bg_rt.block_on(fetch_all_providers(
                    &current_config,
                    data_bg.clone(),
                    last_refresh_bg.clone(),
                    history_bg.clone(),
                ));

                // Regenerate HICON
                let d = data_bg.read();
                let d_resolved: Vec<UsageData> = d.iter().map(|item| {
                    if item.error.is_some() {
                        if let Some(cached) = history_bg.read().latest_successful_for(&item.provider) {
                            cached
                        } else {
                            item.clone()
                        }
                    } else {
                        item.clone()
                    }
                }).collect();
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
                        tray::request_refresh();
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
        let win_w = 400.0_f32;
        let win_h = 520.0_f32;
        let pos = hidden_popup_position();

        let window_options = gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                origin: gpui::Point { x: gpui::px(pos[0] as f32), y: gpui::px(pos[1] as f32) },
                size: gpui::size(gpui::px(win_w), gpui::px(win_h)),
            })),
            titlebar: None,
            focus: true,
            show: true,
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

        let mica_dark = match config_window.general.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                cx.window_appearance(),
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            ),
        };

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

        let win_w = cx.open_window(window_options, move |window, _cx| {
            // Get window handle and store it in OnceLock
            use raw_window_handle::HasWindowHandle;
            if let Ok(handle) = window.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(win32_handle) = handle.as_raw() {
                    let hwnd = HWND(win32_handle.hwnd.get() as *mut std::ffi::c_void);
                    tracing::info!("Successfully resolved Win32 HWND: {:?}", hwnd.0);
                    let _ = tray::MAIN_HWND.set(tray::SendHWND::new(hwnd));
                } else {
                    tracing::warn!("Resolved window handle but it is not Win32");
                }
            } else {
                tracing::warn!("Failed to obtain window handle from window context");
            }
            view.clone()
        }).expect("failed to open window");

        win_w
            .update(cx, |_, window, cx| {
                cx.observe_window_appearance(window, |app, window, cx| {
                    if app.config.general.theme == "system" {
                        let dark = matches!(
                            window.appearance(),
                            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                        );
                        refresh_mica_backdrop(dark);
                    }
                    cx.notify();
                })
                .detach();
            })
            .expect("failed to observe window appearance");

        // Initialize Win32 attributes safely after GPUI window creation finishes to avoid RefCell reentrancy panics
        if let Some(shwnd) = tray::MAIN_HWND.get() {
            let hwnd = shwnd.raw();
            apply_mica_backdrop(hwnd, mica_dark);
            apply_rounded_window_region(hwnd);
            move_popup_offscreen(hwnd);
            set_dwm_cloak(hwnd, true);
            unsafe {
                use windows::Win32::UI::Shell::SetWindowSubclass;
                let _ = SetWindowSubclass(hwnd, Some(main_window_subclass), 1, 0);
            }
        }

        // Spawn a listener task to handle repaint triggers from update channel
        cx.spawn(move |cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                while let Some(_) = update_rx.recv().await {
                    cx.update(|cx| {
                        win_w.update(cx, |_, _, cx| {
                            cx.notify();
                        }).ok();
                    }).ok();
                }
            }
        }).detach();
    });

    Ok(())
}

fn compute_popup_position(win_w: f32, win_h: f32) -> [f32; 2] {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
        };
        use windows::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let mut pt = POINT { x: 0, y: 0 };
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }

        // Try to get actual tray icon rect
        if let Some(&shwnd) = crate::tray::TRAY_HWND.get() {
            let identifier = NOTIFYICONIDENTIFIER {
                cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: shwnd.raw(),
                uID: 1,
                guidItem: Default::default(),
            };
            unsafe {
                if let Ok(rect) = Shell_NotifyIconGetRect(&identifier) {
                    // Use icon center as the reference point instead of arbitrary cursor pos
                    pt.x = rect.left + (rect.right - rect.left) / 2;
                    pt.y = rect.top + (rect.bottom - rect.top) / 2;
                }
            }
        }

        unsafe {
            let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..std::mem::zeroed()
            };
            if GetMonitorInfoW(hmon, &mut mi).as_bool() {
                let work = mi.rcWork;
                let monitor = mi.rcMonitor;
                let margin = 20.0;

                // Determine taskbar position by comparing work area to monitor area
                if work.bottom < monitor.bottom {
                    // Taskbar is at the bottom
                    let mut x = pt.x as f32 - win_w / 2.0;
                    x = x.clamp(
                        work.left as f32 + margin,
                        (work.right as f32 - win_w - margin).max(work.left as f32),
                    );
                    let y = work.bottom as f32 - win_h - margin;
                    return [x, y];
                } else if work.top > monitor.top {
                    // Taskbar is at the top
                    let mut x = pt.x as f32 - win_w / 2.0;
                    x = x.clamp(
                        work.left as f32 + margin,
                        (work.right as f32 - win_w - margin).max(work.left as f32),
                    );
                    let y = work.top as f32 + margin;
                    return [x, y];
                } else if work.left > monitor.left {
                    // Taskbar is on the left
                    let x = work.left as f32 + margin;
                    let mut y = pt.y as f32 - win_h / 2.0;
                    y = y.clamp(
                        work.top as f32 + margin,
                        (work.bottom as f32 - win_h - margin).max(work.top as f32),
                    );
                    return [x, y];
                } else if work.right < monitor.right {
                    // Taskbar is on the right
                    let x = work.right as f32 - win_w - margin;
                    let mut y = pt.y as f32 - win_h / 2.0;
                    y = y.clamp(
                        work.top as f32 + margin,
                        (work.bottom as f32 - win_h - margin).max(work.top as f32),
                    );
                    return [x, y];
                } else {
                    // Taskbar might be auto-hidden (work == monitor)
                    // Determine the edge based on the tray icon position `pt` relative to `monitor`
                    let mon_w = (monitor.right - monitor.left) as f32;
                    let mon_h = (monitor.bottom - monitor.top) as f32;
                    let pt_x_rel = (pt.x - monitor.left) as f32;
                    let pt_y_rel = (pt.y - monitor.top) as f32;

                    if pt_y_rel < mon_h / 4.0 {
                        // Taskbar is at the top
                        let mut x = pt.x as f32 - win_w / 2.0;
                        x = x.clamp(
                            monitor.left as f32 + margin,
                            (monitor.right as f32 - win_w - margin).max(monitor.left as f32),
                        );
                        let y = monitor.top as f32 + margin;
                        return [x, y];
                    } else if pt_x_rel < mon_w / 4.0 {
                        // Taskbar is on the left
                        let x = monitor.left as f32 + margin;
                        let mut y = pt.y as f32 - win_h / 2.0;
                        y = y.clamp(
                            monitor.top as f32 + margin,
                            (monitor.bottom as f32 - win_h - margin).max(monitor.top as f32),
                        );
                        return [x, y];
                    } else if pt_x_rel > mon_w * 0.75 {
                        // Taskbar is on the right
                        let x = monitor.right as f32 - win_w - margin;
                        let mut y = pt.y as f32 - win_h / 2.0;
                        y = y.clamp(
                            monitor.top as f32 + margin,
                            (monitor.bottom as f32 - win_h - margin).max(monitor.top as f32),
                        );
                        return [x, y];
                    } else {
                        // Taskbar is at the bottom (default fallback)
                        let mut x = pt.x as f32 - win_w / 2.0;
                        x = x.clamp(
                            monitor.left as f32 + margin,
                            (monitor.right as f32 - win_w - margin).max(monitor.left as f32),
                        );
                        let y = monitor.bottom as f32 - win_h - margin;
                        return [x, y];
                    }
                }
            }
        }

        [
            (pt.x as f32 - win_w / 2.0).max(0.0),
            (pt.y as f32 - win_h).max(0.0),
        ]
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (win_w, win_h);
        [100.0, 100.0]
    }
}

fn apply_mica_backdrop(hwnd: HWND, dark: bool) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Graphics::Dwm::{
            DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
            DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
        };
        use windows::Win32::UI::Controls::MARGINS;

        MICA_DARK_MODE.store(dark, std::sync::atomic::Ordering::SeqCst);
        IS_MICA_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
        if hwnd.0.is_null() {
            tracing::warn!("apply_mica_backdrop called with null HWND");
            return;
        }

        let dark_mode = if dark { 1_i32 } else { 0_i32 };
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };

        unsafe {
            let dark_result = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark_mode as *const _ as *const _,
                std::mem::size_of_val(&dark_mode) as u32,
            );
            if let Err(err) = dark_result {
                tracing::warn!("Failed to set Mica theme: {err}");
            }

            let frame_result = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let backdrop_type = DWMSBT_MAINWINDOW.0;
            let backdrop_result = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &backdrop_type as *const _ as *const _,
                std::mem::size_of_val(&backdrop_type) as u32,
            );

            // Windows 11 build 22000 predates DWMWA_SYSTEMBACKDROP_TYPE. Use the
            // former private Mica attribute only when the supported API is rejected.
            let effect_result = backdrop_result.or_else(|_| {
                let mica_attribute = windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(1029);
                let enabled = 1_i32;
                DwmSetWindowAttribute(
                    hwnd,
                    mica_attribute,
                    &enabled as *const _ as *const _,
                    std::mem::size_of_val(&enabled) as u32,
                )
            });

            match (frame_result, effect_result) {
                (Ok(()), Ok(())) => {
                    IS_MICA_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
                    tracing::info!("Mica backdrop enabled for HWND {:?}", hwnd.0);
                }
                (frame, effect) => {
                    tracing::warn!(
                        "Mica backdrop unavailable (extend frame: {frame:?}, backdrop: {effect:?})"
                    );
                }
            }
        }
    }
}

pub(crate) fn refresh_mica_backdrop(dark: bool) {
    if let Some(hwnd) = tray::MAIN_HWND.get() {
        apply_mica_backdrop(hwnd.raw(), dark);
        trigger_gui_update();
    }
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
            SW_HIDE, SetForegroundWindow, ShowWindow, WA_INACTIVE, WM_ACTIVATE, WM_CLOSE,
            WM_DESTROY, WM_DWMCOMPOSITIONCHANGED, WM_SIZE,
        };

        match msg {
            tray::WM_APP_SHOW => {
                let target_page = wparam.0 as u32;
                let current_page = tray::ACTIVE_PAGE.load(Ordering::SeqCst);

                if tray::WINDOW_VISIBLE.load(Ordering::SeqCst) {
                    if target_page == current_page {
                        hide_popup_window(hwnd);
                        return LRESULT(0);
                    } else {
                        tray::ACTIVE_PAGE.store(target_page, Ordering::SeqCst);
                        trigger_gui_update();
                        let _ = SetForegroundWindow(hwnd);
                        use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
                        let _ = SetFocus(Some(hwnd));
                        return LRESULT(0);
                    }
                }

                tray::ACTIVE_PAGE.store(target_page, Ordering::SeqCst);

                let scale = window_scale_factor();
                let physical_w = (400.0 * scale) as i32;
                let physical_h = (520.0 * scale) as i32;
                unsafe {
                    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOZORDER, SetWindowPos};
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        physical_w,
                        physical_h,
                        SWP_NOMOVE | SWP_NOZORDER,
                    );
                }

                let (win_w, win_h) = actual_window_size(hwnd).unwrap_or((400.0 * scale, 520.0 * scale));
                let pos = compute_popup_position(win_w, win_h);
                *inactive_guard().lock() = Some(Instant::now() + Duration::from_millis(350));
                apply_rounded_window_region(hwnd);
                set_dwm_cloak(hwnd, false);
                show_popup_window(hwnd, pos);
                let _ = SetForegroundWindow(hwnd);

                use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
                let _ = SetFocus(Some(hwnd));

                trigger_gui_update();

                LRESULT(0)
            }
            WM_ACTIVATE => {
                let active_state = (wparam.0 & 0xFFFF) as u32;
                if active_state == WA_INACTIVE {
                    let ignore_inactive = inactive_guard()
                        .lock()
                        .is_some_and(|until| Instant::now() < until);
                    if !ignore_inactive && !is_pointer_on_tray_icon() {
                        hide_popup_window(hwnd);
                    }
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
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
                if power_event == 4 { // PBT_APMSUSPEND
                    tracing::info!("System is suspending (sleeping). Pausing refresh.");
                    SYSTEM_SLEEPING.store(true, Ordering::SeqCst);
                } else if power_event == 18 { // PBT_APMRESUMEAUTOMATIC
                    tracing::info!("System resumed from sleep. Triggering refresh.");
                    SYSTEM_SLEEPING.store(false, Ordering::SeqCst);
                    tray::request_refresh();
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_DWMCOMPOSITIONCHANGED => {
                apply_mica_backdrop(hwnd, MICA_DARK_MODE.load(Ordering::SeqCst));
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_SIZE => {
                apply_rounded_window_region(hwnd);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            tray::WM_APP_QUIT => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
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

fn window_scale_factor() -> f32 {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC, GetDeviceCaps, LOGPIXELSX};
        unsafe {
            let hdc = GetDC(None);
            if !hdc.0.is_null() {
                let dpi = GetDeviceCaps(Some(hdc), LOGPIXELSX);
                let _ = ReleaseDC(None, hdc);
                return (dpi as f32 / 96.0).max(1.0);
            }
        }
    }
    1.0
}

fn actual_window_size(hwnd: HWND) -> Option<(f32, f32)> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

        if hwnd.0.is_null() {
            return None;
        }

        let mut rect = RECT::default();
        unsafe {
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                let width = rect.right - rect.left;
                let height = rect.bottom - rect.top;
                if width > 0 && height > 0 {
                    return Some((width as f32, height as f32));
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
        None
    }
}

static CURRENT_ANIMATION_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn show_popup_window(hwnd: HWND, final_pos: [f32; 2]) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            SW_SHOW, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowPos, ShowWindow,
        };

        tray::WINDOW_VISIBLE.store(true, Ordering::SeqCst);
        let scale = window_scale_factor();
        let physical_w = (400.0 * scale) as i32;
        let physical_h = (520.0 * scale) as i32;

        let (_win_w, win_h) = actual_window_size(hwnd).unwrap_or((400.0 * scale, 520.0 * scale));
        let anchor_y = popup_anchor_y().unwrap_or(final_pos[1] + 1.0);
        let start_y = if anchor_y < final_pos[1] {
            final_pos[1] - win_h
        } else {
            final_pos[1] + win_h
        };
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                final_pos[0] as i32,
                start_y as i32,
                physical_w,
                physical_h,
                SWP_NOZORDER,
            );
            let _ = ShowWindow(hwnd, SW_SHOW);
        }

        let send_hwnd = tray::SendHWND::new(hwnd);
        let anim_id = CURRENT_ANIMATION_ID.fetch_add(1, Ordering::SeqCst) + 1;

        std::thread::spawn(move || {
            let hwnd = send_hwnd.raw();
            let steps = 18; // Butter-smooth 180ms animation
            for frame in 1..=steps {
                if CURRENT_ANIMATION_ID.load(Ordering::SeqCst) != anim_id {
                    return; // Aborted
                }
                let t = frame as f32 / steps as f32;
                let eased = 1.0 - (1.0 - t).powi(3); // Decelerating ease-out curve
                let y = start_y + (final_pos[1] - start_y) * eased;
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        final_pos[0] as i32,
                        y as i32,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            }

            if CURRENT_ANIMATION_ID.load(Ordering::SeqCst) != anim_id {
                return; // Aborted
            }

            // Show animation finished, make the window topmost (always-on-top)
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{
                    HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
                };
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (hwnd, final_pos);
    }
}

fn hidden_popup_position() -> [i32; 2] {
    [-32000, -32000]
}

fn move_popup_offscreen(hwnd: HWND) {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, SWP_NOACTIVATE, SWP_NOSIZE, SetWindowPos,
        };
        let pos = hidden_popup_position();
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_NOTOPMOST),
            pos[0],
            pos[1],
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
    }
}

fn set_dwm_cloak(hwnd: HWND, cloaked: bool) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};

        let value: i32 = if cloaked { 1 } else { 0 };
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_CLOAK,
                &value as *const _ as *const _,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (hwnd, cloaked);
    }
}

fn popup_anchor_y() -> Option<f32> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

        if let Some(&shwnd) = crate::tray::TRAY_HWND.get() {
            let identifier = NOTIFYICONIDENTIFIER {
                cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: shwnd.raw(),
                uID: 1,
                guidItem: Default::default(),
            };
            unsafe {
                if let Ok(rect) = Shell_NotifyIconGetRect(&identifier) {
                    return Some((rect.top + (rect.bottom - rect.top) / 2) as f32);
                }
            }
        }

        let mut pt = POINT { x: 0, y: 0 };
        unsafe {
            if GetCursorPos(&mut pt).is_ok() {
                return Some(pt.y as f32);
            }
        }
        None
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

fn is_pointer_on_tray_icon() -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let Some(&shwnd) = crate::tray::TRAY_HWND.get() else {
            return false;
        };

        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: shwnd.raw(),
            uID: 1,
            guidItem: Default::default(),
        };

        let mut pt = POINT { x: 0, y: 0 };
        unsafe {
            if GetCursorPos(&mut pt).is_err() {
                return false;
            }

            let Ok(rect) = Shell_NotifyIconGetRect(&identifier) else {
                return false;
            };

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
    #[cfg(target_os = "windows")]
    {
        *inactive_guard().lock() = None;
        tray::WINDOW_VISIBLE.store(false, Ordering::SeqCst);

        // BEFORE the slide-down animation starts, make it standard (NOTOPMOST) so it slides behind the taskbar
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                HWND_NOTOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
            };
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

        let send_hwnd = tray::SendHWND::new(hwnd);
        let anim_id = CURRENT_ANIMATION_ID.fetch_add(1, Ordering::SeqCst) + 1;

        let (win_w, win_h) = actual_window_size(hwnd).unwrap_or((400.0, 520.0));
        let final_pos = compute_popup_position(win_w, win_h);
        let anchor_y = popup_anchor_y().unwrap_or(final_pos[1] + 1.0);
        let start_y = if anchor_y < final_pos[1] {
            final_pos[1] - win_h
        } else {
            final_pos[1] + win_h
        };

        std::thread::spawn(move || {
            let hwnd = send_hwnd.raw();
            use windows::Win32::UI::WindowsAndMessaging::{SWP_NOSIZE, SWP_NOZORDER, SetWindowPos};

            let steps = 15; // Smooth 150ms animation
            for frame in 1..=steps {
                if CURRENT_ANIMATION_ID.load(Ordering::SeqCst) != anim_id {
                    return; // Aborted by a new show/hide animation
                }
                let t = frame as f32 / steps as f32;
                // Easing curve: ease-in (starts slow, speeds up towards taskbar)
                let eased = t.powi(3);
                let y = final_pos[1] + (start_y - final_pos[1]) * eased;

                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        final_pos[0] as i32,
                        y as i32,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER,
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            }

            if CURRENT_ANIMATION_ID.load(Ordering::SeqCst) != anim_id {
                return; // Aborted
            }

            move_popup_offscreen(hwnd);
            set_dwm_cloak(hwnd, true);
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
    }
}
pub mod webview_login;
