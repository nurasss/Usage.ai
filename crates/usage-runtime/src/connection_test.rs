use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use usage_core::{Account, ConnectionState, Coverage, Freshness};
use usage_host::policy::SOURCE_TIMEOUT;
use usage_host::{CancellationToken, HostError, Hosts};
use usage_providers::descriptor::{find_descriptor, AccountModel, ProductDescriptor};
use usage_providers::strategy::{Availability, FetchContext, FetchStrategy};
use usage_storage::Storage;
use uuid::Uuid;

use crate::registry::{facade_policy_for, strategies_for, StrategyKind};
use crate::root_resolver::product_root_candidates;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionTestDiagnostics {
    pub provider: String,
    pub product: String,
    pub account_alias: String,
    pub selected_source: Option<String>,
    pub connection_state: ConnectionState,
    pub last_refresh_attempt: Option<String>,
    pub last_successful_refresh: Option<String>,
    pub last_data_observed_at: Option<String>,
    pub freshness: Freshness,
    pub coverage: Coverage,
    pub status_class: Option<String>,
    pub connector_version: String,
    pub parser_version: String,
    pub schema_fingerprint: Option<String>,
    pub capabilities_detected: Vec<String>,
    pub cooldown_until: Option<String>,
    pub last_safe_error_code: Option<String>,
    pub warnings: Vec<String>,
}

pub async fn test_connection(
    hosts: &Hosts,
    storage: &Mutex<Storage>,
    account_id: Uuid,
) -> Result<ConnectionTestDiagnostics, String> {
    let managed = storage
        .lock()
        .map_err(|_| "storage_lock".to_string())?
        .get_account(account_id)
        .map_err(|_| "accounts_unavailable".to_string())?
        .ok_or_else(|| "account_not_found".to_string())?;

    let product = managed.product_id.clone().unwrap_or_default();
    let descriptor = find_descriptor(&managed.provider_id, &product)
        .ok_or_else(|| "unsupported_source".to_string())?;

    let now = Utc::now();
    let capabilities: Vec<String> = descriptor
        .capabilities
        .iter()
        .copied()
        .map(|cap| format!("{cap:?}"))
        .collect();

    match descriptor.account_model {
        AccountModel::ApiKey => {
            test_api_connection(hosts, &managed, descriptor, &product, &capabilities, now).await
        }
        AccountModel::LocalClient => {
            test_local_connection(hosts, &managed, descriptor, &product, &capabilities, now).await
        }
        AccountModel::DiscoveryOnly => Ok(ConnectionTestDiagnostics {
            provider: managed.provider_id,
            product,
            account_alias: managed.label,
            selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
            connection_state: ConnectionState::UnsupportedSource,
            last_refresh_attempt: Some(now.to_rfc3339()),
            last_successful_refresh: None,
            last_data_observed_at: None,
            freshness: Freshness::Unknown,
            coverage: Coverage::UnverifiedSemantics,
            status_class: Some("error".into()),
            connector_version: descriptor.diagnostics_version.into(),
            parser_version: descriptor.diagnostics_version.into(),
            schema_fingerprint: None,
            capabilities_detected: capabilities,
            cooldown_until: None,
            last_safe_error_code: Some("unsupported_source".into()),
            warnings: vec!["discovery_only_source".into()],
        }),
    }
}

async fn test_api_connection(
    hosts: &Hosts,
    managed: &usage_storage::ManagedAccount,
    descriptor: &'static ProductDescriptor,
    product: &str,
    capabilities: &[String],
    now: chrono::DateTime<Utc>,
) -> Result<ConnectionTestDiagnostics, String> {
    let policy = facade_policy_for(descriptor.provider_id, descriptor.product_id);
    let facade = hosts.facade(&policy);
    let secret = match &facade.keychain {
        Some(kc) => kc
            .read(&managed.id.to_string())
            .await
            .map_err(|_| "authentication_required".to_string())?,
        None => return Err("authentication_required".into()),
    };

    let probe_account = Account {
        id: managed.id,
        provider_id: managed.provider_id.clone(),
        external_identity: managed.external_identity.clone(),
        label: managed.label.clone(),
        connection_ref: managed.connection_ref.clone(),
        lifecycle: managed.lifecycle,
    };

    let strategy_kind = strategies_for(descriptor.provider_id, descriptor.product_id)
        .into_iter()
        .next()
        .ok_or_else(|| "unsupported_source".to_string())?;

    let strategy = build_strategy(strategy_kind, None);
    let ctx = FetchContext {
        account: &probe_account,
        facade,
        timeout: SOURCE_TIMEOUT,
        local_root: None,
        secret: Some(secret.expose().to_vec()),
        cancel: CancellationToken::new(),
        descriptor,
    };

    let availability = strategy.availability(&ctx).await;
    if availability != Availability::Ready {
        return Err("authentication_required".into());
    }

    match tokio::time::timeout(SOURCE_TIMEOUT, strategy.fetch(&ctx)).await {
        Ok(Ok(payload)) => Ok(ConnectionTestDiagnostics {
            provider: managed.provider_id.clone(),
            product: product.to_string(),
            account_alias: managed.label.clone(),
            selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
            connection_state: ConnectionState::Connected,
            last_refresh_attempt: Some(now.to_rfc3339()),
            last_successful_refresh: None,
            last_data_observed_at: None,
            freshness: Freshness::Unknown,
            coverage: payload.coverage,
            status_class: Some("ok".into()),
            connector_version: descriptor.diagnostics_version.into(),
            parser_version: descriptor.diagnostics_version.into(),
            schema_fingerprint: Some(payload.schema_fingerprint.to_string()),
            capabilities_detected: capabilities.to_vec(),
            cooldown_until: None,
            last_safe_error_code: None,
            warnings: payload.warnings,
        }),
        Ok(Err(err)) => Err(err.safe_code().into()),
        Err(_) => Err("source_timeout".into()),
    }
}

async fn test_local_connection(
    hosts: &Hosts,
    managed: &usage_storage::ManagedAccount,
    descriptor: &'static ProductDescriptor,
    product: &str,
    capabilities: &[String],
    now: chrono::DateTime<Utc>,
) -> Result<ConnectionTestDiagnostics, String> {
    let custom_path = managed
        .custom_path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from);

    let root_path = if let Some(custom) = custom_path {
        Some(custom)
    } else {
        product_root_candidates(hosts, descriptor, None)
            .into_iter()
            .next()
    };

    let Some(root) = root_path else {
        return Ok(ConnectionTestDiagnostics {
            provider: managed.provider_id.clone(),
            product: product.to_string(),
            account_alias: managed.label.clone(),
            selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
            connection_state: ConnectionState::Unavailable,
            last_refresh_attempt: Some(now.to_rfc3339()),
            last_successful_refresh: None,
            last_data_observed_at: None,
            freshness: Freshness::Unknown,
            coverage: Coverage::UnverifiedSemantics,
            status_class: Some("error".into()),
            connector_version: descriptor.diagnostics_version.into(),
            parser_version: descriptor.diagnostics_version.into(),
            schema_fingerprint: None,
            capabilities_detected: capabilities.to_vec(),
            cooldown_until: None,
            last_safe_error_code: Some("local_root_missing".into()),
            warnings: vec!["local_root_missing".into()],
        });
    };

    let cancel = CancellationToken::new();
    let scoped_root = match hosts.file_scope.scope_root(&root, &cancel) {
        Ok(sr) => sr,
        Err(HostError::NotFound) => {
            return Ok(ConnectionTestDiagnostics {
                provider: managed.provider_id.clone(),
                product: product.to_string(),
                account_alias: managed.label.clone(),
                selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
                connection_state: ConnectionState::Unavailable,
                last_refresh_attempt: Some(now.to_rfc3339()),
                last_successful_refresh: None,
                last_data_observed_at: None,
                freshness: Freshness::Unknown,
                coverage: Coverage::UnverifiedSemantics,
                status_class: Some("error".into()),
                connector_version: descriptor.diagnostics_version.into(),
                parser_version: descriptor.diagnostics_version.into(),
                schema_fingerprint: None,
                capabilities_detected: capabilities.to_vec(),
                cooldown_until: None,
                last_safe_error_code: Some("local_root_missing".into()),
                warnings: vec!["local_root_missing".into()],
            });
        }
        Err(_) => {
            return Ok(ConnectionTestDiagnostics {
                provider: managed.provider_id.clone(),
                product: product.to_string(),
                account_alias: managed.label.clone(),
                selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
                connection_state: ConnectionState::PermissionDenied,
                last_refresh_attempt: Some(now.to_rfc3339()),
                last_successful_refresh: None,
                last_data_observed_at: None,
                freshness: Freshness::Unknown,
                coverage: Coverage::UnverifiedSemantics,
                status_class: Some("error".into()),
                connector_version: descriptor.diagnostics_version.into(),
                parser_version: descriptor.diagnostics_version.into(),
                schema_fingerprint: None,
                capabilities_detected: capabilities.to_vec(),
                cooldown_until: None,
                last_safe_error_code: Some("root_inaccessible".into()),
                warnings: vec!["root_inaccessible".into()],
            });
        }
    };

    let policy = facade_policy_for(descriptor.provider_id, descriptor.product_id);
    let facade = hosts.facade(&policy);
    let files_host = match facade.files {
        Some(f) => f,
        None => {
            return Ok(ConnectionTestDiagnostics {
                provider: managed.provider_id.clone(),
                product: product.to_string(),
                account_alias: managed.label.clone(),
                selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
                connection_state: ConnectionState::Unavailable,
                last_refresh_attempt: Some(now.to_rfc3339()),
                last_successful_refresh: None,
                last_data_observed_at: None,
                freshness: Freshness::Unknown,
                coverage: Coverage::UnverifiedSemantics,
                status_class: Some("error".into()),
                connector_version: descriptor.diagnostics_version.into(),
                parser_version: descriptor.diagnostics_version.into(),
                schema_fingerprint: None,
                capabilities_detected: capabilities.to_vec(),
                cooldown_until: None,
                last_safe_error_code: Some("files_host_unavailable".into()),
                warnings: vec!["files_host_unavailable".into()],
            });
        }
    };

    let glob = descriptor.local_glob.unwrap_or("*");
    let candidate_files = match files_host.list_files(&scoped_root, glob, &cancel) {
        Ok(files) => files,
        Err(_) => {
            return Ok(ConnectionTestDiagnostics {
                provider: managed.provider_id.clone(),
                product: product.to_string(),
                account_alias: managed.label.clone(),
                selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
                connection_state: ConnectionState::Unavailable,
                last_refresh_attempt: Some(now.to_rfc3339()),
                last_successful_refresh: None,
                last_data_observed_at: None,
                freshness: Freshness::Unknown,
                coverage: Coverage::UnverifiedSemantics,
                status_class: Some("error".into()),
                connector_version: descriptor.diagnostics_version.into(),
                parser_version: descriptor.diagnostics_version.into(),
                schema_fingerprint: None,
                capabilities_detected: capabilities.to_vec(),
                cooldown_until: None,
                last_safe_error_code: Some("enumeration_failed".into()),
                warnings: vec!["enumeration_failed".into()],
            });
        }
    };

    if candidate_files.is_empty() {
        return Ok(ConnectionTestDiagnostics {
            provider: managed.provider_id.clone(),
            product: product.to_string(),
            account_alias: managed.label.clone(),
            selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
            connection_state: ConnectionState::Unavailable,
            last_refresh_attempt: Some(now.to_rfc3339()),
            last_successful_refresh: None,
            last_data_observed_at: None,
            freshness: Freshness::Unknown,
            coverage: Coverage::UnverifiedSemantics,
            status_class: Some("error".into()),
            connector_version: descriptor.diagnostics_version.into(),
            parser_version: descriptor.diagnostics_version.into(),
            schema_fingerprint: None,
            capabilities_detected: capabilities.to_vec(),
            cooldown_until: None,
            last_safe_error_code: Some("no_candidate_files_found".into()),
            warnings: vec![
                "no_candidate_files_found".into(),
                "unverified_local_semantics".into(),
            ],
        });
    }

    // Probe the first readable candidate file for schema validity
    let mut probe_chunk = None;
    for file in &candidate_files {
        if let Ok(bytes) = files_host.read_scoped_range(file, 0, 4096, &cancel) {
            if !bytes.is_empty() {
                probe_chunk = Some(bytes);
                break;
            }
        }
    }

    let Some(sample_bytes) = probe_chunk else {
        return Ok(ConnectionTestDiagnostics {
            provider: managed.provider_id.clone(),
            product: product.to_string(),
            account_alias: managed.label.clone(),
            selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
            connection_state: ConnectionState::Unavailable,
            last_refresh_attempt: Some(now.to_rfc3339()),
            last_successful_refresh: None,
            last_data_observed_at: None,
            freshness: Freshness::Unknown,
            coverage: Coverage::UnverifiedSemantics,
            status_class: Some("error".into()),
            connector_version: descriptor.diagnostics_version.into(),
            parser_version: descriptor.diagnostics_version.into(),
            schema_fingerprint: None,
            capabilities_detected: capabilities.to_vec(),
            cooldown_until: None,
            last_safe_error_code: Some("files_empty".into()),
            warnings: vec!["files_empty".into(), "unverified_local_semantics".into()],
        });
    };

    // Test parser against sample
    let probe_account = Account {
        id: managed.id,
        provider_id: managed.provider_id.clone(),
        external_identity: managed.external_identity.clone(),
        label: managed.label.clone(),
        connection_ref: managed.connection_ref.clone(),
        lifecycle: managed.lifecycle,
    };

    let (records_count, malformed_count) = match descriptor.product_id {
        "codex" => {
            let parsed = usage_providers::codex::parse_chunk_bytes(
                &probe_account,
                "probe",
                0,
                &sample_bytes,
                now,
            );
            (parsed.records.len(), parsed.malformed)
        }
        "claude-code" => {
            let parsed = usage_providers::claude::parse_chunk_bytes(
                &probe_account,
                "probe",
                0,
                &sample_bytes,
                now,
            );
            (parsed.records.len(), parsed.malformed)
        }
        _ => (0, 0),
    };

    if records_count == 0 {
        return Ok(ConnectionTestDiagnostics {
            provider: managed.provider_id.clone(),
            product: product.to_string(),
            account_alias: managed.label.clone(),
            selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
            connection_state: if malformed_count > 0 {
                ConnectionState::ParseError
            } else {
                ConnectionState::UnsupportedSource
            },
            last_refresh_attempt: Some(now.to_rfc3339()),
            last_successful_refresh: None,
            last_data_observed_at: None,
            freshness: Freshness::Unknown,
            coverage: Coverage::UnverifiedSemantics,
            status_class: Some("error".into()),
            connector_version: descriptor.diagnostics_version.into(),
            parser_version: descriptor.diagnostics_version.into(),
            schema_fingerprint: None,
            capabilities_detected: capabilities.to_vec(),
            cooldown_until: None,
            last_safe_error_code: Some(if malformed_count > 0 {
                "schema_mismatch".into()
            } else {
                "no_recognized_events".into()
            }),
            warnings: vec![if malformed_count > 0 {
                "schema_mismatch".into()
            } else {
                "no_recognized_events".into()
            }],
        });
    }

    Ok(ConnectionTestDiagnostics {
        provider: managed.provider_id.clone(),
        product: product.to_string(),
        account_alias: managed.label.clone(),
        selected_source: descriptor.sources.first().map(|s| s.id.to_string()),
        connection_state: ConnectionState::Connected,
        last_refresh_attempt: Some(now.to_rfc3339()),
        last_successful_refresh: None,
        last_data_observed_at: None,
        freshness: Freshness::Unknown,
        coverage: Coverage::UnverifiedSemantics,
        status_class: Some("warning".into()),
        connector_version: descriptor.diagnostics_version.into(),
        parser_version: descriptor.diagnostics_version.into(),
        schema_fingerprint: Some(descriptor.diagnostics_version.into()),
        capabilities_detected: capabilities.to_vec(),
        cooldown_until: None,
        last_safe_error_code: None,
        warnings: vec!["unverified_local_semantics".into()],
    })
}

fn build_strategy(kind: StrategyKind, custom_root: Option<PathBuf>) -> Box<dyn FetchStrategy> {
    match kind {
        StrategyKind::CodexAppServer => {
            // Test connections exercise the JSONL observation source;
            // the App Server is covered by provider-level stub tests.
            Box::new(usage_providers::codex::CodexJsonlStrategy::new(custom_root))
        }
        StrategyKind::CodexJsonl => {
            Box::new(usage_providers::codex::CodexJsonlStrategy::new(custom_root))
        }
        StrategyKind::ClaudeJsonl => Box::new(usage_providers::claude::ClaudeJsonlStrategy::new(
            custom_root,
        )),
        StrategyKind::OpenAiCosts => Box::new(usage_providers::openai_api::OpenAiCostsStrategy {
            base_url: custom_root
                .as_ref()
                .and_then(|p| p.to_str())
                .map(str::to_string),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;
    use tempfile::tempdir;
    use usage_core::{AccountLifecycle, IdentityConfidence};
    use usage_host::{
        MemoryKeychain, NullLogger, ObservedNetwork, ReqwestHttpHost, ScopedFiles, SystemClock,
    };
    use usage_storage::{ManagedAccount, Storage};

    fn test_hosts() -> Hosts {
        let files = Arc::new(ScopedFiles);
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

    #[tokio::test]
    async fn local_test_connection_missing_root_returns_unavailable() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("usage.db");
        let storage = Mutex::new(Storage::open(&db_path).unwrap());
        let hosts = test_hosts();

        let account_id = Uuid::new_v4();
        let managed = ManagedAccount {
            id: account_id,
            provider_id: "openai".into(),
            product_id: Some("codex".into()),
            external_identity: None,
            label: "Codex Test".into(),
            lifecycle: AccountLifecycle::Active,
            connection_ref: None,
            custom_path: Some("/path/that/does/not/exist/ever/12345".into()),
            enabled: true,
            identity_confidence: IdentityConfidence::Unknown,
        };
        storage
            .lock()
            .unwrap()
            .create_managed_account(&managed)
            .unwrap();

        let diag = test_connection(&hosts, &storage, account_id).await.unwrap();
        assert_eq!(diag.connection_state, ConnectionState::Unavailable);
        assert_eq!(diag.coverage, Coverage::UnverifiedSemantics);
        assert_eq!(diag.status_class.as_deref(), Some("error"));
        assert!(diag.warnings.contains(&"local_root_missing".to_string()));
    }

    #[tokio::test]
    async fn local_test_connection_existing_root_with_valid_files_returns_unverified_semantics_warning(
    ) {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("usage.db");
        let storage = Mutex::new(Storage::open(&db_path).unwrap());
        let hosts = test_hosts();

        let local_root = temp.path().join("codex_home");
        let sessions_dir = local_root.join("sessions").join("sub");
        fs::create_dir_all(&sessions_dir).unwrap();
        let log_file = sessions_dir.join("rollout-test.jsonl");
        fs::write(
            &log_file,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":10,\"output_tokens\":10}}}}\n",
        )
        .unwrap();

        let account_id = Uuid::new_v4();
        let managed = ManagedAccount {
            id: account_id,
            provider_id: "openai".into(),
            product_id: Some("codex".into()),
            external_identity: None,
            label: "Codex Test".into(),
            lifecycle: AccountLifecycle::Active,
            connection_ref: None,
            custom_path: Some(local_root.to_str().unwrap().into()),
            enabled: true,
            identity_confidence: IdentityConfidence::Unknown,
        };
        storage
            .lock()
            .unwrap()
            .create_managed_account(&managed)
            .unwrap();

        let diag = test_connection(&hosts, &storage, account_id).await.unwrap();
        assert_eq!(diag.connection_state, ConnectionState::Connected);
        assert_eq!(diag.coverage, Coverage::UnverifiedSemantics);
        // Honest warning, NOT ok!
        assert_eq!(diag.status_class.as_deref(), Some("warning"));
        assert!(diag
            .warnings
            .contains(&"unverified_local_semantics".to_string()));
    }

    #[tokio::test]
    async fn local_test_connection_empty_root_returns_unavailable() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("usage.db");
        let storage = Mutex::new(Storage::open(&db_path).unwrap());
        let hosts = test_hosts();

        let local_root = temp.path().join("codex_home");
        fs::create_dir_all(&local_root).unwrap();

        let account_id = Uuid::new_v4();
        let managed = ManagedAccount {
            id: account_id,
            provider_id: "openai".into(),
            product_id: Some("codex".into()),
            external_identity: None,
            label: "Codex Test".into(),
            lifecycle: AccountLifecycle::Active,
            connection_ref: None,
            custom_path: Some(local_root.to_str().unwrap().into()),
            enabled: true,
            identity_confidence: IdentityConfidence::Unknown,
        };
        storage
            .lock()
            .unwrap()
            .create_managed_account(&managed)
            .unwrap();

        let diag = test_connection(&hosts, &storage, account_id).await.unwrap();
        assert_eq!(diag.connection_state, ConnectionState::Unavailable);
        assert_eq!(diag.status_class.as_deref(), Some("error"));
        assert!(diag
            .warnings
            .contains(&"no_candidate_files_found".to_string()));
    }

    #[tokio::test]
    async fn local_test_connection_files_without_recognized_events_returns_unsupported_source() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("usage.db");
        let storage = Mutex::new(Storage::open(&db_path).unwrap());
        let hosts = test_hosts();

        let local_root = temp.path().join("codex_home");
        let sessions_dir = local_root.join("sessions").join("sub");
        fs::create_dir_all(&sessions_dir).unwrap();
        let log_file = sessions_dir.join("rollout-test.jsonl");
        fs::write(&log_file, "{\"type\":\"session_init\",\"info\":{}}\n").unwrap();

        let account_id = Uuid::new_v4();
        let managed = ManagedAccount {
            id: account_id,
            provider_id: "openai".into(),
            product_id: Some("codex".into()),
            external_identity: None,
            label: "Codex Test".into(),
            lifecycle: AccountLifecycle::Active,
            connection_ref: None,
            custom_path: Some(local_root.to_str().unwrap().into()),
            enabled: true,
            identity_confidence: IdentityConfidence::Unknown,
        };
        storage
            .lock()
            .unwrap()
            .create_managed_account(&managed)
            .unwrap();

        let diag = test_connection(&hosts, &storage, account_id).await.unwrap();
        assert_eq!(diag.connection_state, ConnectionState::UnsupportedSource);
        assert_eq!(diag.status_class.as_deref(), Some("error"));
        assert_eq!(
            diag.last_safe_error_code.as_deref(),
            Some("no_recognized_events")
        );
    }

    #[tokio::test]
    async fn local_test_connection_claude_missing_root_returns_unavailable() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("usage.db");
        let storage = Mutex::new(Storage::open(&db_path).unwrap());
        let hosts = test_hosts();

        let account_id = Uuid::new_v4();
        let managed = ManagedAccount {
            id: account_id,
            provider_id: "anthropic".into(),
            product_id: Some("claude-code".into()),
            external_identity: None,
            label: "Claude Test".into(),
            lifecycle: AccountLifecycle::Active,
            connection_ref: None,
            custom_path: Some("/nonexistent/path/claude/test".into()),
            enabled: true,
            identity_confidence: IdentityConfidence::Unknown,
        };
        storage
            .lock()
            .unwrap()
            .create_managed_account(&managed)
            .unwrap();

        let diag = test_connection(&hosts, &storage, account_id).await.unwrap();
        assert_eq!(diag.connection_state, ConnectionState::Unavailable);
        assert_eq!(diag.status_class.as_deref(), Some("error"));
        assert!(diag.warnings.contains(&"local_root_missing".to_string()));
    }

    #[tokio::test]
    async fn local_test_connection_with_malformed_json_returns_parse_error() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("usage.db");
        let storage = Mutex::new(Storage::open(&db_path).unwrap());
        let hosts = test_hosts();

        let local_root = temp.path().join("codex_home");
        let sessions_dir = local_root.join("sessions").join("sub");
        fs::create_dir_all(&sessions_dir).unwrap();
        let log_file = sessions_dir.join("rollout-test.jsonl");
        fs::write(&log_file, "{\ninvalid json block without closing brace\n").unwrap();

        let account_id = Uuid::new_v4();
        let managed = ManagedAccount {
            id: account_id,
            provider_id: "openai".into(),
            product_id: Some("codex".into()),
            external_identity: None,
            label: "Codex Test".into(),
            lifecycle: AccountLifecycle::Active,
            connection_ref: None,
            custom_path: Some(local_root.to_str().unwrap().into()),
            enabled: true,
            identity_confidence: IdentityConfidence::Unknown,
        };
        storage
            .lock()
            .unwrap()
            .create_managed_account(&managed)
            .unwrap();

        let diag = test_connection(&hosts, &storage, account_id).await.unwrap();
        assert_eq!(diag.connection_state, ConnectionState::ParseError);
        assert_eq!(diag.coverage, Coverage::UnverifiedSemantics);
        assert_eq!(diag.status_class.as_deref(), Some("error"));
        assert_eq!(
            diag.last_safe_error_code.as_deref(),
            Some("schema_mismatch")
        );
    }
}
