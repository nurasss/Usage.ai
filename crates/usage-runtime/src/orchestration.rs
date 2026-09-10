use chrono::{Duration, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use usage_core::{Account, AccountLifecycle};
use usage_host::Hosts;
use usage_providers::descriptor::{AccountModel, ProductDescriptor};
use usage_providers::strategy::FileLogParser;
use usage_storage::{ManagedAccount, Storage};
use uuid::Uuid;

use crate::history_import::{import_connection, ImportReport, ImportRequest};
use crate::refresh::{Coordinator, RefreshRequest, ScopeKey, Trigger};
use crate::registry::facade_policy_for;
use crate::root_resolver::product_root_candidates;

/// UI-neutral account label carried alongside a runtime request. The Tauri
/// layer maps this to its DTOs; provider-specific account construction stays
/// here with the refresh/import policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountMeta {
    pub alias: String,
}

/// Stable local account ids are part of the storage compatibility contract.
/// They identify a product's unattributed local bucket, not a verified user.
pub fn stable_local_account(namespace: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("usage.ai/local/{namespace}").as_bytes(),
    )
}

pub fn ensure_local_accounts(storage: &Mutex<Storage>) {
    for descriptor in usage_providers::descriptor::all_descriptors()
        .iter()
        .filter(|descriptor| descriptor.account_model == AccountModel::LocalClient)
    {
        let account = default_local_account(descriptor);
        if let Ok(mut storage) = storage.lock() {
            let _ = storage.upsert_account(&account);
        }
    }
}

/// Build one refresh request per enabled product/account scope. A descriptor
/// determines the account model and host policy; no provider-specific branch
/// is required in the application shell.
pub async fn collect_requests(
    hosts: &Hosts,
    storage: &Mutex<Storage>,
) -> Vec<(RefreshRequest, AccountMeta)> {
    ensure_local_accounts(storage);
    let managed = storage
        .lock()
        .ok()
        .and_then(|storage| storage.list_managed_accounts().ok())
        .unwrap_or_default();
    let mut out = Vec::new();

    for descriptor in usage_providers::descriptor::all_descriptors()
        .iter()
        .filter(|descriptor| descriptor.account_model != AccountModel::DiscoveryOnly)
    {
        if descriptor.account_model == AccountModel::LocalClient {
            let account = default_local_account(descriptor);
            out.push((
                request_for_account(&account, descriptor, None, None),
                AccountMeta {
                    alias: account.label.clone(),
                },
            ));
        }
    }

    for managed_account in managed {
        if !managed_account.enabled
            || matches!(managed_account.lifecycle, AccountLifecycle::Archived)
        {
            continue;
        }
        let Some(product_id) = managed_account.product_id.as_deref() else {
            continue;
        };
        let Some(descriptor) =
            usage_providers::descriptor::find_descriptor(&managed_account.provider_id, product_id)
        else {
            continue;
        };
        if descriptor.account_model == AccountModel::DiscoveryOnly {
            continue;
        }

        let custom_root = (descriptor.account_model == AccountModel::LocalClient)
            .then(|| {
                managed_account
                    .custom_path
                    .clone()
                    .filter(|path| !path.trim().is_empty())
                    .map(PathBuf::from)
            })
            .flatten();
        if descriptor.account_model == AccountModel::LocalClient && custom_root.is_none() {
            // Default local roots are represented by the stable product
            // account above; a managed row without a custom root has no
            // second source to refresh.
            continue;
        }

        let secret = if descriptor.account_model == AccountModel::ApiKey {
            let policy = facade_policy_for(descriptor.provider_id, descriptor.product_id);
            let facade = hosts.facade(&policy);
            match facade.keychain {
                Some(keychain) => keychain
                    .read(&managed_account.id.to_string())
                    .await
                    .ok()
                    .map(|secret| secret.expose().to_vec())
                    .filter(|secret| !secret.is_empty()),
                None => None,
            }
        } else {
            None
        };
        let account = account_from_managed(&managed_account);
        out.push((
            request_for_account(&account, descriptor, custom_root, secret),
            AccountMeta {
                alias: account.label.clone(),
            },
        ));
    }
    out
}

fn request_for_account(
    account: &Account,
    descriptor: &ProductDescriptor,
    custom_root: Option<PathBuf>,
    secret: Option<Vec<u8>>,
) -> RefreshRequest {
    RefreshRequest {
        scope: ScopeKey {
            account_id: account.id,
            provider_id: descriptor.provider_id.into(),
            product_id: descriptor.product_id.into(),
        },
        account: account.clone(),
        custom_root,
        secret,
        trigger: Trigger::Scheduled,
    }
}

/// Import every configured local root into its owning connection. The
/// returned map contains only safe counters/warnings so the application shell
/// does not need to know parser or provider details.
pub async fn import_all_history(
    hosts: &Arc<Hosts>,
    storage: &Arc<Mutex<Storage>>,
    coordinator: &Coordinator,
    retention_days: i64,
) -> HashMap<String, ImportReport> {
    let managed = storage
        .lock()
        .ok()
        .and_then(|storage| storage.list_managed_accounts().ok())
        .unwrap_or_default();
    let mut reports = HashMap::new();

    for descriptor in usage_providers::descriptor::all_descriptors()
        .iter()
        .filter(|descriptor| descriptor.account_model == AccountModel::LocalClient)
    {
        let Some(glob_pattern) = descriptor.local_glob else {
            continue;
        };
        let Some(parser) = parser_for(descriptor) else {
            continue;
        };
        let mut targets: Vec<(Account, &'static str, Vec<PathBuf>)> = Vec::new();
        let default_roots = product_root_candidates(hosts, descriptor, None);
        if !default_roots.is_empty() {
            targets.push((
                default_local_account(descriptor),
                "default-local",
                default_roots,
            ));
        }
        for managed_account in managed.iter().filter(|account| {
            account.enabled
                && !matches!(account.lifecycle, AccountLifecycle::Archived)
                && account.provider_id == descriptor.provider_id
                && account.product_id.as_deref() == Some(descriptor.product_id)
                && account
                    .custom_path
                    .as_deref()
                    .is_some_and(|path| !path.trim().is_empty())
        }) {
            let Some(custom_path) = managed_account.custom_path.as_deref().map(Path::new) else {
                continue;
            };
            let roots = product_root_candidates(hosts, descriptor, Some(custom_path));
            if roots.is_empty() {
                continue;
            }
            targets.push((account_from_managed(managed_account), "custom-local", roots));
        }

        let mut product_report = ImportReport::default();
        for (account, kind, roots) in targets {
            for root in roots {
                let resolved = root.canonicalize().unwrap_or_else(|_| root.clone());
                let root_hash = usage_host::files::path_hash(&resolved);
                let root_hint = resolved
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "default".into());
                let connection_id = match storage.lock() {
                    Ok(mut storage) => storage
                        .ensure_connection(
                            account.id,
                            descriptor.provider_id,
                            descriptor.product_id,
                            &root_hash,
                            &root_hint,
                            kind,
                        )
                        .map(|connection| connection.id)
                        .ok(),
                    Err(_) => None,
                };
                let Some(connection_id) = connection_id else {
                    continue;
                };
                let cancel = coordinator.cancellation_token_for(account.id);
                let report = import_connection(ImportRequest {
                    hosts,
                    storage,
                    parser: parser.as_ref(),
                    account: &account,
                    connection_id: &connection_id,
                    root,
                    glob_pattern,
                    schema_version: 1,
                    cancel: &cancel,
                })
                .await;
                merge_reports(&mut product_report, report);
            }
        }
        reports.insert(descriptor.product_id.to_string(), product_report);
    }

    if let Ok(mut storage) = storage.lock() {
        let cutoff = Utc::now() - Duration::days(retention_days.max(30));
        let _ = storage.prune_retention(cutoff);
    }
    reports
}

fn parser_for(descriptor: &ProductDescriptor) -> Option<Box<dyn FileLogParser>> {
    match descriptor.product_id {
        "codex" => Some(Box::new(usage_providers::codex::CodexJsonlStrategy::new(
            None,
        ))),
        "claude-code" => Some(Box::new(usage_providers::claude::ClaudeJsonlStrategy::new(
            None,
        ))),
        _ => None,
    }
}

fn merge_reports(total: &mut ImportReport, report: ImportReport) {
    total.files_discovered += report.files_discovered;
    total.files_imported += report.files_imported;
    total.records_accepted += report.records_accepted;
    total.malformed += report.malformed;
    total.checkpoint_resets += report.checkpoint_resets;
    total.warnings.extend(report.warnings);
}

fn default_local_account(descriptor: &ProductDescriptor) -> Account {
    Account {
        id: stable_local_account(descriptor.product_id),
        provider_id: descriptor.provider_id.into(),
        external_identity: None,
        label: "Локальная история".into(),
        connection_ref: Some(format!("{}-local", descriptor.product_id)),
        lifecycle: AccountLifecycle::Active,
    }
}

fn account_from_managed(account: &ManagedAccount) -> Account {
    Account {
        id: account.id,
        provider_id: account.provider_id.clone(),
        external_identity: account.external_identity.clone(),
        label: account.label.clone(),
        connection_ref: account.connection_ref.clone(),
        lifecycle: account.lifecycle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_local_account_is_descriptor_driven_and_unknown() {
        let descriptor = usage_providers::descriptor::find_descriptor("openai", "codex").unwrap();
        let account = default_local_account(descriptor);
        assert_eq!(account.label, "Локальная история");
        assert!(account.external_identity.is_none());
        assert_eq!(account.id, stable_local_account("codex"));
    }
}
