use crate::config::AppConfig;

pub const AGENT_PROVIDER_IDS: &[&str] = &[
    "amp",
    "antigravity",
    "augment",
    "claude",
    "clinepass",
    "codebuff",
    "codex",
    "commandcode",
    "copilot",
    "cursor",
    "droid",
    "gemini",
    "jetbrains",
    "kilo",
    "kiro",
    "mimo",
    "opencode",
    "qoder",
    "windsurf",
    "zed",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedAgent {
    pub provider_id: &'static str,
    pub display_name: &'static str,
}

pub fn scan_available_agents(config: &AppConfig) -> Vec<DetectedAgent> {
    collect_detected_agents(|provider_id| crate::create_provider(provider_id, config).is_some())
}

fn collect_detected_agents(mut is_available: impl FnMut(&str) -> bool) -> Vec<DetectedAgent> {
    let mut detected = AGENT_PROVIDER_IDS
        .iter()
        .copied()
        .filter(|provider_id| is_available(provider_id))
        .map(|provider_id| DetectedAgent {
            provider_id,
            display_name: crate::provider::display_name(provider_id),
        })
        .collect::<Vec<_>>();
    detected.sort_by_cached_key(|agent| {
        (
            agent.display_name.to_lowercase(),
            agent.display_name,
            agent.provider_id,
        )
    });
    detected
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{AGENT_PROVIDER_IDS, collect_detected_agents};

    #[test]
    fn agent_candidates_are_registered_and_unique() {
        let registered = crate::provider::PROVIDER_CATALOG
            .iter()
            .map(|(provider_id, _)| *provider_id)
            .collect::<BTreeSet<_>>();
        let candidates = AGENT_PROVIDER_IDS.iter().copied().collect::<BTreeSet<_>>();

        assert_eq!(candidates.len(), AGENT_PROVIDER_IDS.len());
        assert!(candidates.is_subset(&registered));
    }

    #[test]
    fn scan_results_are_filtered_and_sorted_by_display_name() {
        let detected =
            collect_detected_agents(|provider_id| matches!(provider_id, "opencode" | "codex"));

        assert_eq!(
            detected
                .iter()
                .map(|agent| agent.display_name)
                .collect::<Vec<_>>(),
            vec!["Codex", "OpenCode"]
        );
    }
}
