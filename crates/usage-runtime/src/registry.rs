use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use usage_host::Hosts;
use usage_providers::descriptor::{all_descriptors, find_descriptor, ProductDescriptorDto};

/// Which strategy implementations serve a product. The ORDER comes
/// from the product descriptor (`sources`); this mapping only resolves
/// a declared source id to its constructor — there is no second list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrategyKind {
    CodexAppServer,
    CodexJsonl,
    ClaudePtyUsage,
    ClaudeOAuthUsage,
    ClaudeJsonl,
    OpenAiCosts,
}

pub fn strategy_kind(source_id: &str) -> Option<StrategyKind> {
    match source_id {
        "codex-app-server" => Some(StrategyKind::CodexAppServer),
        "codex-local-jsonl" => Some(StrategyKind::CodexJsonl),
        "claude-pty-usage" => Some(StrategyKind::ClaudePtyUsage),
        "claude-oauth-usage" => Some(StrategyKind::ClaudeOAuthUsage),
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

/// Resolve the Codex CLI executables through scoped hosts only: static
/// candidate locations under well-known prefixes plus the user's home.
/// No PATH lookup, no shell. Returns EVERY candidate proven to exist
/// as a file inside its scoped parent directory, in static order —
/// existence is not health, so the App Server strategy health-gates
/// (`initialize` handshake) and serves the first HEALTHY one. A stale
/// system shim must never shadow a working user install.
pub fn resolve_codex_executables(hosts: &Hosts) -> Vec<PathBuf> {
    use usage_host::CancellationToken;
    let Some(home) = hosts.file_scope.home_dir() else {
        return vec![];
    };
    let cancel = CancellationToken::new();
    let mut found_all = vec![];
    for candidate in usage_providers::codex_appserver::codex_executable_candidates(&home) {
        let Some(parent) = candidate.parent() else {
            continue;
        };
        let Ok(root) = hosts.file_scope.scope_root(parent, &cancel) else {
            continue;
        };
        let Some(name) = candidate.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let found = hosts
            .files
            .list_files(&root, name, &cancel)
            .unwrap_or_default();
        if !found.is_empty() {
            found_all.push(candidate);
        }
    }
    found_all
}

/// First existing candidate, for display/diagnostics only. Never use
/// for serving: serve via the strategy's health-gated selection.
pub fn resolve_codex_executable(hosts: &Hosts) -> Option<PathBuf> {
    resolve_codex_executables(hosts).into_iter().next()
}

/// Static Claude Code CLI locations, tried in order. No PATH lookup,
/// no shell. Unlike Codex there is no cheap RPC handshake: the
/// PTY-usage strategy gates on `claude auth status` (fast,
/// non-interactive, $0) instead, so existence here is only the
/// static candidate set — every entry must additionally pass the
/// process allow-list at spawn time.
pub fn resolve_claude_executables(hosts: &Hosts) -> Vec<PathBuf> {
    use usage_host::CancellationToken;
    let Some(home) = hosts.file_scope.home_dir() else {
        return vec![];
    };
    let cancel = CancellationToken::new();
    let mut found_all = vec![];
    for candidate in usage_providers::claude::claude_executable_candidates(&home) {
        let Some(parent) = candidate.parent() else {
            continue;
        };
        let Ok(root) = hosts.file_scope.scope_root(parent, &cancel) else {
            continue;
        };
        let Some(name) = candidate.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let found = hosts
            .files
            .list_files(&root, name, &cancel)
            .unwrap_or_default();
        if !found.is_empty() {
            found_all.push(candidate);
        }
    }
    found_all
}

/// First existing Claude candidate, for display/diagnostics only.
pub fn resolve_claude_executable(hosts: &Hosts) -> Option<PathBuf> {
    resolve_claude_executables(hosts).into_iter().next()
}

/// Host facade policy derived from exactly one descriptor: a strategy
/// physically cannot reach hosts its product was not granted.
pub fn facade_policy_for(provider_id: &str, product_id: &str) -> usage_host::facade::FacadePolicy {
    use usage_host::facade::FacadePolicy;
    match usage_providers::descriptor::find_descriptor(provider_id, product_id) {
        None => FacadePolicy {
            files: false,
            http: false,
            process: false,
            pty: false,
            allowed_hosts: &[],
            keychain_service: None,
        },
        Some(d) => FacadePolicy {
            files: d.local_glob.is_some(),
            http: !d.allowed_hosts.is_empty(),
            process: d.requires_process,
            pty: d.requires_pty,
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

    // collect() over cloned() is intentional here: HashSet has no
    // to_vec with the Vec target type.
    #[allow(clippy::iter_cloned_collect)]
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
    fn codex_app_server_leads_with_jsonl_fallback() {
        // Verified provider source first, local observation fallback
        // second. The runtime walks this order; the fallback never
        // promotes itself to authoritative.
        assert_eq!(
            strategies_for("openai", "codex"),
            vec![StrategyKind::CodexAppServer, StrategyKind::CodexJsonl]
        );
    }
    #[test]
    fn facade_policy_matches_descriptor_model() {
        let codex = facade_policy_for("openai", "codex");
        assert!(codex.files && !codex.http && codex.keychain_service.is_none());
        assert!(codex.process);
        assert!(!codex.pty);
        let claude = facade_policy_for("anthropic", "claude-code");
        assert!(claude.files && !claude.http);
        assert!(claude.process && claude.pty);
        let api = facade_policy_for("openai", "openai-api");
        assert!(!api.files && api.http && !api.process);
        assert_eq!(api.keychain_service, Some("com.nurasss.usageai"));
        let blocked = facade_policy_for("google", "antigravity");
        assert!(!blocked.files && !blocked.http && blocked.keychain_service.is_none());
    }
    #[test]
    fn claude_strategies_follow_descriptor_order() {
        // §8/§10.3: quota ends ahead of the history end; the refresh
        // loop builds in exactly this order.
        assert_eq!(
            strategies_for("anthropic", "claude-code"),
            vec![
                StrategyKind::ClaudePtyUsage,
                StrategyKind::ClaudeOAuthUsage,
                StrategyKind::ClaudeJsonl,
            ]
        );
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

#[cfg(test)]
mod executable_resolution_tests {
    use super::*;
    use std::sync::Arc;
    use usage_host::{
        FileScopeHost, FilesHost, MemoryKeychain, NullLogger, ObservedNetwork, ReqwestHttpHost,
        SystemClock,
    };

    struct SandboxFiles {
        home: PathBuf,
    }

    impl FilesHost for SandboxFiles {}
    impl FileScopeHost for SandboxFiles {
        fn home_dir(&self) -> Option<PathBuf> {
            Some(self.home.clone())
        }
        fn env_var(&self, _name: &str) -> Option<String> {
            None
        }
    }

    fn sandbox_hosts(home: PathBuf) -> Hosts {
        let files: Arc<SandboxFiles> = Arc::new(SandboxFiles { home });
        Hosts {
            clock: Arc::new(SystemClock),
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http: Arc::new(ReqwestHttpHost::default()),
            files: files.clone(),
            file_scope: files,
            network: Arc::new(ObservedNetwork::default()),
            process: Arc::new(usage_host::AllowlistedProcess::default()),
            pty: Arc::new(usage_host::AllowlistedPty::default()),
        }
    }

    #[test]
    fn resolves_real_binary_through_scoped_hosts() {
        // Uses the real HOME: on a machine with the Codex CLI installed
        // in a standard location this finds it; elsewhere it is None.
        // No PATH lookup, no shell, no execution.
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let hosts = sandbox_hosts(home);
        let _ = resolve_codex_executable(&hosts);
    }

    #[test]
    fn missing_binaries_resolve_to_none() {
        let home = tempfile::tempdir().unwrap();
        let hosts = sandbox_hosts(home.path().to_path_buf());
        assert!(resolve_codex_executable(&hosts).is_none());
    }

    #[test]
    fn sandbox_binary_resolves_when_present() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".npm-global").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("codex"), "#!/bin/sh\nexit 0\n").unwrap();
        let hosts = sandbox_hosts(home.path().to_path_buf());
        let found = resolve_codex_executable(&hosts).expect("sandbox binary");
        assert!(found.ends_with(".npm-global/bin/codex"));
    }
}
