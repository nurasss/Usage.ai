use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use usage_providers::descriptor::{all_descriptors, find_descriptor, ProductDescriptorDto};

/// Which strategy implementations serve a product. The ORDER comes
/// from the product descriptor (`sources`); this mapping only resolves
/// a declared source id to its constructor — there is no second list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrategyKind {
    CodexJsonl,
    ClaudeJsonl,
    OpenAiCosts,
}

pub fn strategy_kind(source_id: &str) -> Option<StrategyKind> {
    match source_id {
        "codex-local-jsonl" => Some(StrategyKind::CodexJsonl),
        "claude-local-jsonl" => Some(StrategyKind::ClaudeJsonl),
        "openai-api-costs" => Some(StrategyKind::OpenAiCosts),
        _ => None,
    }
}

pub fn strategies_for(provider_id: &str, product_id: &str) -> Vec<StrategyKind> {
    usage_providers::descriptor::find_descriptor(provider_id, product_id)
        .map(|d| {
            d.sources
                .iter()
                .filter_map(|s| strategy_kind(s.id))
                .collect()
        })
        .unwrap_or_default()
}

/// Host facade policy derived from exactly one descriptor: a strategy
/// physically cannot reach hosts its product was not granted.
pub fn facade_policy_for(provider_id: &str, product_id: &str) -> usage_host::facade::FacadePolicy {
    use usage_host::facade::FacadePolicy;
    match usage_providers::descriptor::find_descriptor(provider_id, product_id) {
        None => FacadePolicy {
            files: false,
            http: false,
            allowed_hosts: &[],
            keychain_service: None,
        },
        Some(d) => FacadePolicy {
            files: d.local_glob.is_some(),
            http: !d.allowed_hosts.is_empty(),
            allowed_hosts: d.allowed_hosts,
            keychain_service: matches!(
                d.account_model,
                usage_providers::descriptor::AccountModel::ApiKey
            )
            .then_some("com.nurasss.usageai"),
        },
    }
}

/// Production provider registry: descriptors are the single source of
/// truth, strategies attach to them, private kill-switches gate them.
pub struct Registry {
    disabled_switches: Mutex<HashSet<String>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            disabled_switches: Mutex::new(HashSet::new()),
        }
    }
}

impl Registry {
    pub fn production() -> Self {
        Self::default()
    }

    pub fn descriptors(&self) -> Vec<ProductDescriptorDto> {
        all_descriptors()
            .iter()
            .map(ProductDescriptorDto::from)
            .collect()
    }

    pub fn find(
        &self,
        provider_id: &str,
        product_id: &str,
    ) -> Option<&'static usage_providers::descriptor::ProductDescriptor> {
        find_descriptor(provider_id, product_id)
    }

    pub fn disable_source(&self, kill_switch: &str) {
        if let Ok(mut guard) = self.disabled_switches.lock() {
            guard.insert(kill_switch.to_string());
        }
    }

    pub fn enable_source(&self, kill_switch: &str) {
        if let Ok(mut guard) = self.disabled_switches.lock() {
            guard.remove(kill_switch);
        }
    }

    pub fn is_disabled(&self, kill_switch: &str) -> bool {
        !kill_switch.is_empty()
            && self
                .disabled_switches
                .lock()
                .map(|guard| guard.contains(kill_switch))
                .unwrap_or(false)
    }

    pub fn disabled_sources(&self) -> Vec<String> {
        self.disabled_switches
            .lock()
            .map(|guard| guard.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// Human-readable source-class labels for diagnostics UI.
pub fn classification_label(
    class: usage_providers::strategy::SourceClassification,
) -> &'static str {
    match class {
        usage_providers::strategy::SourceClassification::OfficialApi => "Official API",
        usage_providers::strategy::SourceClassification::SupportedClientApi => {
            "Supported client API"
        }
        usage_providers::strategy::SourceClassification::LocalStructuredData => "Local client data",
        usage_providers::strategy::SourceClassification::OfficialExport => "Official export",
        usage_providers::strategy::SourceClassification::PrivateClientApi => "Private client API",
        usage_providers::strategy::SourceClassification::BrowserSession => "Browser session",
        usage_providers::strategy::SourceClassification::HtmlFallback => "HTML fallback",
    }
}

pub fn disabled_map_snapshot(disabled: &[String]) -> HashMap<String, bool> {
    disabled.iter().map(|s| (s.clone(), true)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_wired_product_has_a_descriptor() {
        for (provider, product) in [
            ("openai", "codex"),
            ("anthropic", "claude-code"),
            ("openai", "openai-api"),
        ] {
            assert!(find_descriptor(provider, product).is_some());
            assert!(!strategies_for(provider, product).is_empty());
        }
    }
    #[test]
    fn strategy_order_follows_descriptor_sources() {
        let descriptor = find_descriptor("openai", "codex").unwrap();
        let from_descriptor: Vec<_> = descriptor
            .sources
            .iter()
            .filter_map(|s| strategy_kind(s.id))
            .collect();
        assert_eq!(from_descriptor, strategies_for("openai", "codex"));
    }
    #[test]
    fn facade_policy_matches_descriptor_model() {
        let codex = facade_policy_for("openai", "codex");
        assert!(codex.files && !codex.http && codex.keychain_service.is_none());
        let api = facade_policy_for("openai", "openai-api");
        assert!(!api.files && api.http);
        assert_eq!(api.keychain_service, Some("com.nurasss.usageai"));
        let blocked = facade_policy_for("google", "antigravity");
        assert!(!blocked.files && !blocked.http && blocked.keychain_service.is_none());
    }
    #[test]
    fn discovery_products_have_no_strategies() {
        assert!(strategies_for("google", "antigravity").is_empty());
        assert!(strategies_for("opencode", "zen").is_empty());
    }
    #[test]
    fn kill_switch_roundtrips() {
        let registry = Registry::production();
        assert!(!registry.is_disabled("x"));
        registry.disable_source("x");
        assert!(registry.is_disabled("x"));
        registry.enable_source("x");
        assert!(!registry.is_disabled("x"));
    }
}
