use crate::config::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStatus {
    Active,
    Error,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageLevel {
    Normal,
    Warning,
    Critical,
}

pub fn provider_catalog() -> &'static [(&'static str, &'static str)] {
    &[
        ("codex", "Codex"),
        ("openai", "OpenAI"),
        ("opencode", "OpenCode"),
        ("opencodego", "OpenCode Go"),
        ("claude", "Claude"),
        ("gemini", "Gemini"),
        ("antigravity", "Antigravity"),
        ("deepseek", "DeepSeek"),
        ("openrouter", "OpenRouter"),
        ("moonshot", "Moonshot"),
        ("elevenlabs", "ElevenLabs"),
        ("doubao", "Doubao"),
        ("zai", "z.ai"),
        ("venice", "Venice"),
        ("crof", "Crof"),
        ("synthetic", "Synthetic"),
        ("warp", "Warp"),
        ("groqcloud", "GroqCloud"),
        ("deepgram", "Deepgram"),
        ("llmproxy", "LLM Proxy"),
        ("codebuff", "Codebuff"),
        ("kiro", "Kiro"),
        ("copilot", "Copilot"),
        ("azureopenai", "Azure OpenAI"),
        ("ollama", "Ollama"),
        ("minimax", "MiniMax"),
        ("jetbrains", "JetBrains AI"),
        ("kimi", "Kimi"),
        ("kilo", "Kilo Code"),
        ("augment", "Augment"),
        ("bedrock", "AWS Bedrock"),
        ("vertexai", "Vertex AI"),
        ("stepfun", "StepFun"),
        ("abacus", "Abacus AI"),
        ("alibabatoken", "Alibaba Token"),
        ("t3chat", "T3 Chat"),
        ("amp", "Amp"),
        ("mistral", "Mistral"),
        ("grok", "Grok"),
        ("cursor", "Cursor"),
        ("droid", "Factory Droid"),
        ("windsurf", "Windsurf"),
        ("mimo", "MiMo"),
    ]
}

pub fn provider_display_order(config: &AppConfig) -> Vec<(String, &'static str)> {
    let mut ordered = Vec::new();
    for configured in &config.general.provider_order {
        if let Some((id, display_name)) = provider_catalog()
            .iter()
            .find(|(id, _)| id.eq_ignore_ascii_case(configured))
            && !ordered.iter().any(|(existing, _)| existing == id)
        {
            ordered.push(((*id).to_string(), *display_name));
        }
    }

    for (id, display_name) in provider_catalog() {
        if !ordered.iter().any(|(existing, _)| existing == id) {
            ordered.push(((*id).to_string(), *display_name));
        }
    }

    ordered
}

pub fn ensure_provider_order(order: &mut Vec<String>) {
    let mut normalized = Vec::new();
    for configured in order.iter() {
        if let Some((id, _)) = provider_catalog()
            .iter()
            .find(|(id, _)| id.eq_ignore_ascii_case(configured))
            && !normalized.iter().any(|existing| existing == id)
        {
            normalized.push((*id).to_string());
        }
    }

    for (id, _) in provider_catalog() {
        if !normalized.iter().any(|existing| existing == id) {
            normalized.push((*id).to_string());
        }
    }

    *order = normalized;
}

pub fn reorder_provider(order: &mut Vec<String>, dragged: &str, target: &str) -> bool {
    ensure_provider_order(order);
    let Some(from) = order.iter().position(|id| id == dragged) else {
        return false;
    };
    let Some(to) = order.iter().position(|id| id == target) else {
        return false;
    };
    if from == to {
        return false;
    }

    let item = order.remove(from);
    let target_index = order.iter().position(|id| id == target).unwrap_or(to);
    order.insert(target_index, item);
    true
}

pub fn provider_status(data: Option<&crate::provider::UsageData>) -> ProviderStatus {
    match data {
        Some(d) if d.error.is_some() => ProviderStatus::Error,
        Some(_) => ProviderStatus::Active,
        None => ProviderStatus::Disabled,
    }
}

pub fn format_credits_balance(balance: f64) -> String {
    if balance.abs() >= 1_000_000_000.0 {
        let val = balance / 1_000_000_000.0;
        format!("{val:.2}B")
    } else if balance.abs() >= 1_000_000.0 {
        let val = balance / 1_000_000.0;
        if (val - val.round()).abs() < 0.01 {
            format!("{val:.0}M")
        } else {
            format!("{val:.2}M")
        }
    } else if balance.abs() >= 1_000.0 {
        let val = balance / 1_000.0;
        if (val - val.round()).abs() < 0.01 {
            format!("{val:.0}K")
        } else {
            format!("{val:.2}K")
        }
    } else if (balance - balance.round()).abs() < 0.01 {
        format!("{balance:.0}")
    } else {
        format!("{balance:.2}")
    }
}

pub fn usage_level(pct: f32) -> UsageLevel {
    if pct >= 80.0 {
        UsageLevel::Critical
    } else if pct >= 50.0 {
        UsageLevel::Warning
    } else {
        UsageLevel::Normal
    }
}

pub fn reset_time_text(resets_at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    let Some(resets) = resets_at else {
        return "-".to_string();
    };

    let remaining = resets - chrono::Utc::now();
    if remaining.num_seconds() <= 0 {
        return "resetting".to_string();
    }

    let days = remaining.num_days();
    let hours = remaining.num_hours() % 24;
    let minutes = remaining.num_minutes() % 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

pub fn normalize_version(v: &str) -> String {
    let v = v.trim().trim_start_matches('v').trim_start_matches('V');
    if let Some((main, _)) = v.split_once('-') {
        main.to_string()
    } else {
        v.to_string()
    }
}

pub fn is_newer(current: &str, latest: &str) -> bool {
    let current_norm = normalize_version(current);
    let latest_norm = normalize_version(latest);

    let current_parts: Vec<u32> = current_norm
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let latest_parts: Vec<u32> = latest_norm
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();

    for i in 0..std::cmp::max(current_parts.len(), latest_parts.len()) {
        let curr = current_parts.get(i).cloned().unwrap_or(0);
        let lat = latest_parts.get(i).cloned().unwrap_or(0);
        if lat > curr {
            return true;
        } else if curr > lat {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_versions() {
        assert_eq!(normalize_version("v0.1.0-1-gae62f96"), "0.1.0");
        assert_eq!(normalize_version("V1.2.3"), "1.2.3");
        assert_eq!(normalize_version("2.0.0"), "2.0.0");
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.1.0-1-gae62f96", "v0.2.0"));
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(!is_newer("v0.2.0", "v0.1.0"));
        assert!(!is_newer("v0.1.0", "v0.1.0"));
        assert!(is_newer("v0.1.0", "1.0.0"));
    }

    #[test]
    fn normalizes_provider_order() {
        let mut order = vec![
            "Gemini".to_string(),
            "unknown".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
        ];
        ensure_provider_order(&mut order);
        assert_eq!(order[0], "gemini");
        assert_eq!(order[1], "codex");
        assert!(order.contains(&"openai".to_string()));
        assert!(!order.contains(&"unknown".to_string()));
    }

    #[test]
    fn reorders_provider() {
        let mut order = vec![
            "codex".to_string(),
            "openai".to_string(),
            "gemini".to_string(),
        ];
        assert!(reorder_provider(&mut order, "gemini", "codex"));
        assert_eq!(&order[..3], ["gemini", "codex", "openai"]);
    }

    #[test]
    fn formats_credit_balances() {
        assert_eq!(format_credits_balance(999.0), "999");
        assert_eq!(format_credits_balance(1_000.0), "1K");
        assert_eq!(format_credits_balance(1_250_000.0), "1.25M");
        assert_eq!(format_credits_balance(1_000_000_000.0), "1.00B");
    }
}
