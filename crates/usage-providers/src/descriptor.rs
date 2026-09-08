use serde::{Deserialize, Serialize};
use usage_core::Capability;

/// How an account of this product is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountModel {
    LocalClient,
    ApiKey,
    DiscoveryOnly,
}

/// One fetch source declared by a product descriptor. The runtime
/// builds its strategy order from exactly this list — there is no
/// second hard-coded registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceDescriptor {
    pub id: &'static str,
    pub classification: crate::strategy::SourceClassification,
    pub kill_switch: &'static str,
    pub version: &'static str,
}

/// Single source of truth for every product surface:
/// runtime registry, UI product list, icons, capability-driven
/// settings, allowed hosts, diagnostics and account policy.
/// Rust and Svelte must not keep independent hard-coded lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductDescriptor {
    pub provider_id: &'static str,
    pub product_id: &'static str,
    pub provider_name: &'static str,
    pub product_name: &'static str,
    pub glyph: &'static str,
    pub color: &'static str,
    pub capabilities: &'static [Capability],
    pub account_model: AccountModel,
    pub allowed_hosts: &'static [&'static str],
    /// Ordered fetch sources; the runtime strategy order follows this list.
    pub sources: &'static [SourceDescriptor],
    /// Glob relative to the product root, e.g. `sessions/**/rollout-*.jsonl`.
    pub local_glob: Option<&'static str>,
    /// Env override for the product home, e.g. `CODEX_HOME`.
    pub local_dir_env: Option<&'static str>,
    /// Dot-directory under `$HOME`, e.g. `.codex`.
    pub local_dir_name: Option<&'static str>,
    pub allow_custom_path: bool,
    pub diagnostics_version: &'static str,
}

pub const CODEX_CAPS: &[Capability] = &[
    Capability::SubscriptionQuota,
    Capability::QuotaResetTime,
    Capability::ApiTokens,
    Capability::HistoryLocal,
    Capability::LocalSessions,
];

pub const CLAUDE_CODE_CAPS: &[Capability] = &[
    Capability::ApiTokens,
    Capability::HistoryLocal,
    Capability::LocalSessions,
    Capability::ModelBreakdown,
];

pub const OPENAI_API_CAPS: &[Capability] = &[
    Capability::ApiCostReported,
    Capability::ProjectBreakdown,
    Capability::HistoryRemote,
    Capability::MultiAccount,
];

pub const NO_CAPS: &[Capability] = &[];

pub const CODEX_SOURCES: &[SourceDescriptor] = &[SourceDescriptor {
    id: "codex-local-jsonl",
    classification: crate::strategy::SourceClassification::LocalStructuredData,
    kill_switch: "",
    version: "v1",
}];

pub const CLAUDE_SOURCES: &[SourceDescriptor] = &[SourceDescriptor {
    id: "claude-local-jsonl",
    classification: crate::strategy::SourceClassification::LocalStructuredData,
    kill_switch: "",
    version: "v1",
}];

pub const OPENAI_API_SOURCES: &[SourceDescriptor] = &[SourceDescriptor {
    id: "openai-api-costs",
    classification: crate::strategy::SourceClassification::OfficialApi,
    kill_switch: "",
    version: "v1",
}];

pub const NO_SOURCES: &[SourceDescriptor] = &[];

const DESCRIPTORS: &[ProductDescriptor] = &[
    ProductDescriptor {
        provider_id: "openai",
        product_id: "codex",
        provider_name: "OpenAI",
        product_name: "Codex",
        glyph: "✣",
        color: "#ff7a5c",
        capabilities: CODEX_CAPS,
        account_model: AccountModel::LocalClient,
        allowed_hosts: &[],
        sources: CODEX_SOURCES,
        local_glob: Some("sessions/**/rollout-*.jsonl"),
        local_dir_env: Some("CODEX_HOME"),
        local_dir_name: Some(".codex"),
        allow_custom_path: true,
        diagnostics_version: "codex-local-v1",
    },
    ProductDescriptor {
        provider_id: "anthropic",
        product_id: "claude-code",
        provider_name: "Anthropic",
        product_name: "Claude Code",
        glyph: "A",
        color: "#b895ff",
        capabilities: CLAUDE_CODE_CAPS,
        account_model: AccountModel::LocalClient,
        allowed_hosts: &[],
        sources: CLAUDE_SOURCES,
        local_glob: Some("projects/**/*.jsonl"),
        local_dir_env: None,
        local_dir_name: Some(".claude"),
        allow_custom_path: true,
        diagnostics_version: "claude-local-v1",
    },
    ProductDescriptor {
        provider_id: "openai",
        product_id: "openai-api",
        provider_name: "OpenAI",
        product_name: "API",
        glyph: "✣",
        color: "#46c9a7",
        capabilities: OPENAI_API_CAPS,
        account_model: AccountModel::ApiKey,
        allowed_hosts: crate::openai_api::OPENAI_API_HOSTS,
        sources: OPENAI_API_SOURCES,
        local_glob: None,
        local_dir_env: None,
        local_dir_name: None,
        allow_custom_path: false,
        diagnostics_version: "openai-api-v1",
    },
    ProductDescriptor {
        provider_id: "openai",
        product_id: "chatgpt",
        provider_name: "OpenAI",
        product_name: "ChatGPT",
        glyph: "✣",
        capabilities: NO_CAPS,
        account_model: AccountModel::DiscoveryOnly,
        allowed_hosts: &[],
        sources: NO_SOURCES,
        local_glob: None,
        local_dir_env: None,
        local_dir_name: None,
        allow_custom_path: false,
        diagnostics_version: "discovery-v1",
    },
    ProductDescriptor {
        provider_id: "anthropic",
        product_id: "claude-ai",
        provider_name: "Anthropic",
        product_name: "claude.ai",
        glyph: "A",
        capabilities: NO_CAPS,
        account_model: AccountModel::DiscoveryOnly,
        allowed_hosts: &[],
        sources: NO_SOURCES,
        local_glob: None,
        local_dir_env: None,
        local_dir_name: None,
        allow_custom_path: false,
        diagnostics_version: "discovery-v1",
    },
    ProductDescriptor {
        provider_id: "google",
        product_id: "antigravity",
        provider_name: "Google",
        product_name: "Antigravity",
        glyph: "G",
        color: "#4c82ee",
        capabilities: NO_CAPS,
        account_model: AccountModel::DiscoveryOnly,
        allowed_hosts: &[],
        sources: NO_SOURCES,
        local_glob: None,
        local_dir_env: None,
        local_dir_name: None,
        allow_custom_path: false,
        diagnostics_version: "discovery-v1",
    },
    ProductDescriptor {
        provider_id: "google",
        product_id: "gemini-api",
        provider_name: "Google",
        product_name: "Gemini API",
        glyph: "G",
        color: "#4c82ee",
        capabilities: NO_CAPS,
        account_model: AccountModel::DiscoveryOnly,
        allowed_hosts: &[],
        sources: NO_SOURCES,
        local_glob: None,
        local_dir_env: None,
        local_dir_name: None,
        allow_custom_path: false,
        diagnostics_version: "discovery-v1",
    },
    ProductDescriptor {
        provider_id: "zai",
        product_id: "glm-coding",
        provider_name: "Z.ai",
        product_name: "GLM Coding Plan",
        glyph: "Z",
        color: "#694bd7",
        capabilities: NO_CAPS,
        account_model: AccountModel::DiscoveryOnly,
        allowed_hosts: &[],
        sources: NO_SOURCES,
        local_glob: None,
        local_dir_env: None,
        local_dir_name: None,
        allow_custom_path: false,
        diagnostics_version: "discovery-v1",
    },
    ProductDescriptor {
        provider_id: "opencode",
        product_id: "zen",
        provider_name: "OpenCode",
        product_name: "Zen",
        glyph: "◈",
        color: "#2f9277",
        capabilities: NO_CAPS,
        account_model: AccountModel::DiscoveryOnly,
        allowed_hosts: &[],
        sources: NO_SOURCES,
        local_glob: None,
        local_dir_env: None,
        local_dir_name: None,
        allow_custom_path: false,
        diagnostics_version: "discovery-v1",
    },
];

pub fn all_descriptors() -> &'static [ProductDescriptor] {
    DESCRIPTORS
}

pub fn find_descriptor(provider_id: &str, product_id: &str) -> Option<&'static ProductDescriptor> {
    DESCRIPTORS
        .iter()
        .find(|d| d.provider_id == provider_id && d.product_id == product_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDto {
    pub id: String,
    pub classification: String,
    pub kill_switch: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDescriptorDto {
    pub provider_id: String,
    pub product_id: String,
    pub provider_name: String,
    pub product_name: String,
    pub glyph: String,
    pub color: String,
    pub capabilities: Vec<String>,
    pub account_model: AccountModel,
    pub allowed_hosts: Vec<String>,
    pub allow_custom_path: bool,
    pub needs_secret: bool,
    pub sources: Vec<SourceDto>,
}

fn capability_name(value: Capability) -> String {
    match value {
        Capability::SubscriptionQuota => "subscriptionQuota",
        Capability::QuotaResetTime => "quotaResetTime",
        Capability::ApiTokens => "apiTokens",
        Capability::ApiRequests => "apiRequests",
        Capability::ApiCostReported => "apiCostReported",
        Capability::ApiCostEstimated => "apiCostEstimated",
        Capability::Balance => "balance",
        Capability::Credits => "credits",
        Capability::ModelBreakdown => "modelBreakdown",
        Capability::ProjectBreakdown => "projectBreakdown",
        Capability::HistoryRemote => "historyRemote",
        Capability::HistoryLocal => "historyLocal",
        Capability::LocalSessions => "localSessions",
        Capability::MultiAccount => "multiAccount",
    }
    .into()
}

impl From<&ProductDescriptor> for ProductDescriptorDto {
    fn from(d: &ProductDescriptor) -> Self {
        Self {
            provider_id: d.provider_id.into(),
            product_id: d.product_id.into(),
            provider_name: d.provider_name.into(),
            product_name: d.product_name.into(),
            glyph: d.glyph.into(),
            color: d.color.into(),
            capabilities: d
                .capabilities
                .iter()
                .copied()
                .map(capability_name)
                .collect(),
            account_model: d.account_model,
            allowed_hosts: d.allowed_hosts.iter().map(|h| h.to_string()).collect(),
            allow_custom_path: d.allow_custom_path,
            needs_secret: d.account_model == AccountModel::ApiKey,
            sources: d
                .sources
                .iter()
                .map(|s| SourceDto {
                    id: s.id.into(),
                    classification: format!("{:?}", s.classification),
                    kill_switch: s.kill_switch.into(),
                    version: s.version.into(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_has_no_duplicate_products() {
        let mut seen = HashSet::new();
        for d in all_descriptors() {
            assert!(seen.insert((d.provider_id, d.product_id)));
        }
    }

    #[test]
    fn claude_local_declares_no_subscription_quota() {
        let d = find_descriptor("anthropic", "claude-code").unwrap();
        assert!(!d.capabilities.contains(&Capability::SubscriptionQuota));
        assert!(d.capabilities.contains(&Capability::HistoryLocal));
    }

    #[test]
    fn local_products_declare_scoped_roots() {
        for d in all_descriptors()
            .iter()
            .filter(|d| d.account_model == AccountModel::LocalClient)
        {
            assert!(d.local_glob.is_some(), "{}", d.product_id);
            assert!(d.local_dir_name.is_some(), "{}", d.product_id);
        }
    }

    #[test]
    fn frontend_dto_carries_no_secret_material() {
        let dto = ProductDescriptorDto::from(find_descriptor("openai", "openai-api").unwrap());
        let json = serde_json::to_string(&dto).unwrap();
        // Schema metadata (`needs_secret: bool`, `accountModel: "apiKey"`)
        // is fine; actual key material, tokens and passwords are not.
        for forbidden in ["sk-", "bearer", "api_key", "password", "cookie"] {
            assert!(
                !json.to_ascii_lowercase().contains(forbidden),
                "{forbidden}"
            );
        }
        assert!(dto.needs_secret);
        assert_eq!(dto.allowed_hosts, vec!["api.openai.com".to_string()]);
    }
}
