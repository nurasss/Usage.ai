use usage_core::AccountLifecycle;
use usage_providers::descriptor::{find_descriptor, AccountModel};
use usage_storage::ManagedAccount;
use uuid::Uuid;

use crate::appstate::AppState;
use crate::dto::{managed_account_to_dto, AccountDto, DiagnosticsDto};
use crate::platform::hosts::KEYCHAIN_SERVICE;

/// Typed connection setup for an Admin API key. The backend validates
/// the product/connection type; there is no generic secret command.
#[tauri::command]
pub async fn configure_openai_admin_connection(
    label: String,
    secret: String,
    state: tauri::State<'_, AppState>,
) -> Result<AccountDto, String> {
    let alias = label.trim();
    if alias.is_empty() || alias.len() > 64 {
        return Err("invalid_alias".into());
    }
    if secret.trim().is_empty() || secret.len() > 4096 {
        return Err("invalid_secret_input".into());
    }
    let Some(descriptor) = find_descriptor("openai", "openai-api") else {
        return Err("unsupported_product".into());
    };
    if descriptor.account_model != AccountModel::ApiKey {
        return Err("unsupported_product".into());
    }
    let id = Uuid::new_v4();
    state
        .hosts
        .keychain
        .write(KEYCHAIN_SERVICE, &id.to_string(), secret.trim().as_bytes())
        .await
        .map_err(|_| "keychain_write_denied".to_string())?;
    let managed = ManagedAccount {
        id,
        provider_id: "openai".into(),
        product_id: Some("openai-api".into()),
        external_identity: Some(usage_host::fingerprint(
            "openai-api",
            secret.trim().as_bytes(),
        )),
        label: alias.into(),
        connection_ref: Some("keychain:com.nurasss.usageai".into()),
        lifecycle: AccountLifecycle::Active,
        enabled: true,
        custom_path: None,
        identity_confidence: usage_core::IdentityConfidence::Weak,
    };
    state
        .storage
        .lock()
        .map_err(|_| "storage_lock".to_string())?
        .create_managed_account(&managed)
        .map_err(|_| "account_create_failed".to_string())?;
    Ok(managed_account_to_dto(&managed))
}

/// Removes the connection secret and disconnects the account. Provider
/// state and foreign client credentials are never touched.
#[tauri::command]
pub async fn remove_connection_secret(
    account_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let id: Uuid = account_id
        .parse()
        .map_err(|_| "invalid_account_id".to_string())?;
    let _ = state
        .hosts
        .keychain
        .delete(KEYCHAIN_SERVICE, &account_id)
        .await;
    state.coordinator.cancel_account(id);
    let mut storage = state
        .storage
        .lock()
        .map_err(|_| "storage_lock".to_string())?;
    storage
        .update_managed_account(id, None, Some(false), None)
        .map_err(|_| "account_update_failed".to_string())?;
    Ok("Секрет удалён, подключение отключено".into())
}

#[tauri::command]
pub fn list_accounts(state: tauri::State<'_, AppState>) -> Result<Vec<AccountDto>, String> {
    let storage = state.storage.lock().map_err(|_| "storage_lock")?;
    Ok(storage
        .list_managed_accounts()
        .map_err(|_| "accounts_unavailable")?
        .iter()
        .map(managed_account_to_dto)
        .collect())
}

#[tauri::command]
pub fn add_account(
    provider_id: String,
    product_id: String,
    alias: String,
    secret: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<AccountDto, String> {
    let Some(descriptor) = find_descriptor(&provider_id, &product_id) else {
        return Err("unsupported_product".into());
    };
    if descriptor.account_model == AccountModel::DiscoveryOnly {
        return Err("unsupported_source".into());
    }
    let alias = alias.trim();
    if alias.is_empty() || alias.len() > 64 {
        return Err("invalid_alias".into());
    }
    let needs_secret = descriptor.account_model == AccountModel::ApiKey;
    if needs_secret && secret.as_deref().unwrap_or("").trim().is_empty() {
        return Err("secret_required".into());
    }
    if let Some(secret) = secret.as_deref() {
        if secret.len() > 4096 {
            return Err("invalid_secret_input".into());
        }
    }
    if !needs_secret && secret.as_deref().is_some_and(|s| !s.is_empty()) {
        return Err("secret_not_supported".into());
    }
    let id = Uuid::new_v4();
    let external_identity = if needs_secret {
        Some(usage_host::fingerprint(
            descriptor.product_id,
            secret.as_deref().unwrap_or_default().trim().as_bytes(),
        ))
    } else {
        None
    };
    let connection_ref = if needs_secret {
        Some("keychain:com.nurasss.usageai".into())
    } else {
        descriptor
            .local_glob
            .is_some()
            .then(|| format!("{}-local", descriptor.product_id))
    };
    let managed = ManagedAccount {
        id,
        provider_id: provider_id.clone(),
        product_id: Some(product_id.clone()),
        external_identity,
        label: alias.into(),
        connection_ref,
        lifecycle: AccountLifecycle::Active,
        enabled: true,
        custom_path: None,
        identity_confidence: if needs_secret {
            usage_core::IdentityConfidence::Weak
        } else {
            usage_core::IdentityConfidence::Unknown
        },
    };
    if needs_secret {
        // Synchronous Keychain write through the blocking host path is
        // avoided: the typed async command above is the supported flow.
        // This legacy path keeps compatibility for local accounts only.
        return Err("use_configure_connection".into());
    }
    state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .create_managed_account(&managed)
        .map_err(|_| "account_create_failed")?;
    Ok(managed_account_to_dto(&managed))
}

#[tauri::command]
pub fn update_account(
    account_id: String,
    alias: Option<String>,
    enabled: Option<bool>,
    custom_path: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<AccountDto, String> {
    let id: Uuid = account_id.parse().map_err(|_| "invalid_account_id")?;
    if let Some(alias) = alias.as_deref() {
        if alias.trim().is_empty() || alias.len() > 64 {
            return Err("invalid_alias".into());
        }
    }
    if let Some(path) = custom_path.as_deref() {
        if path.len() > 1024 {
            return Err("invalid_custom_path".into());
        }
        // Custom paths must resolve inside the product scope at use
        // time; obviously hostile values are rejected early.
        if path.contains("..") {
            return Err("invalid_custom_path".into());
        }
    }
    let mut storage = state.storage.lock().map_err(|_| "storage_lock")?;
    let custom = custom_path.map(|p| {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    storage
        .update_managed_account(
            id,
            alias.as_deref(),
            enabled,
            custom.as_ref().map(|o| o.as_deref()),
        )
        .map_err(|_| "account_update_failed")?;
    let updated = storage
        .get_account(id)
        .map_err(|_| "accounts_unavailable")?
        .map(|a| managed_account_to_dto(&a))
        .ok_or_else(|| "account_not_found".into());
    drop(storage);
    if let Some(enabled) = enabled {
        if enabled {
            state.coordinator.reset_account_cancellation(id);
        } else {
            state.coordinator.cancel_account(id);
        }
    }
    updated
}

#[tauri::command]
pub async fn test_connection(
    account_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<DiagnosticsDto, String> {
    let id: Uuid = account_id
        .parse()
        .map_err(|_| "invalid_account_id".to_string())?;

    let diag = usage_runtime::test_connection(&state.hosts, &state.storage, id).await?;

    Ok(DiagnosticsDto {
        provider: diag.provider,
        product: diag.product,
        account_alias: diag.account_alias,
        account_id: id.to_string(),
        selected_source: diag.selected_source,
        connection_state: diag.connection_state,
        last_refresh_attempt: diag.last_refresh_attempt,
        last_successful_refresh: diag.last_successful_refresh,
        last_data_observed_at: diag.last_data_observed_at,
        freshness: diag.freshness,
        coverage: diag.coverage,
        status_class: diag.status_class,
        connector_version: diag.connector_version,
        parser_version: diag.parser_version,
        schema_fingerprint: diag.schema_fingerprint,
        capabilities_detected: diag.capabilities_detected,
        cooldown_until: diag.cooldown_until,
        last_safe_error_code: diag.last_safe_error_code,
        warnings: diag.warnings,
        recent_attempts: vec![],
    })
}

#[tauri::command]
pub fn remove_account(
    account_id: String,
    delete_history: bool,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let id: Uuid = account_id.parse().map_err(|_| "invalid_account_id")?;
    state.coordinator.cancel_account(id);
    // Own Keychain secret is removed in both cases; the provider-side
    // account and foreign client credentials are never touched.
    let hosts = state.hosts.clone();
    let account_id_owned = account_id.clone();
    tauri::async_runtime::spawn(async move {
        let _ = hosts
            .keychain
            .delete(KEYCHAIN_SERVICE, &account_id_owned)
            .await;
    });
    let mut storage = state.storage.lock().map_err(|_| "storage_lock")?;
    if delete_history {
        storage
            .delete_account_and_history(id)
            .map_err(|_| "account_delete_failed")?;
        Ok("Подключение и история удалены".into())
    } else {
        storage
            .archive_account(id)
            .map_err(|_| "account_archive_failed")?;
        Ok("Подключение отключено, история сохранена как архив".into())
    }
}

/// Discover profile roots for local products. Candidates are validated
/// (marker layout) but never trusted: nothing is created, connected or
/// imported by a scan. Custom roots come from enabled managed accounts.
#[tauri::command]
pub fn scan_candidates(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<crate::dto::CandidateDto>, String> {
    use usage_host::CancellationToken;
    use usage_runtime::discovery::{discover_profile_roots, DiscoveredKind};

    let managed: Vec<ManagedAccount> = state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .list_managed_accounts()
        .map_err(|_| "accounts_unavailable")?;
    let mut out = vec![];
    for descriptor in usage_providers::descriptor::all_descriptors()
        .iter()
        .filter(|d| d.account_model == AccountModel::LocalClient)
    {
        let custom_roots: Vec<std::path::PathBuf> = managed
            .iter()
            .filter(|a| {
                a.enabled
                    && a.product_id.as_deref() == Some(descriptor.product_id)
                    && a.custom_path
                        .as_deref()
                        .is_some_and(|p| !p.trim().is_empty())
            })
            .filter_map(|a| a.custom_path.clone().map(std::path::PathBuf::from))
            .collect();
        for found in discover_profile_roots(
            &state.hosts,
            descriptor,
            &custom_roots,
            &CancellationToken::new(),
        ) {
            if !found.valid {
                continue;
            }
            let kind = match found.kind {
                DiscoveredKind::Default => "default",
                DiscoveredKind::Slug => "slug",
                DiscoveredKind::Custom => "custom",
            };
            let (status, bound_account) = match state.storage.lock() {
                Ok(mut storage) => {
                    let candidate = storage
                        .upsert_candidate(
                            &found.root_hash,
                            descriptor.provider_id,
                            descriptor.product_id,
                            &found.hint,
                        )
                        .map_err(|_| "candidate_store_failed")?;
                    let bound = storage
                        .connection_for_root(
                            descriptor.provider_id,
                            descriptor.product_id,
                            &found.root_hash,
                        )
                        .map_err(|_| "candidate_store_failed")?
                        .map(|c| c.account_id);
                    (candidate.status, bound)
                }
                Err(_) => continue,
            };
            // The default root of a fresh install auto-binds to the
            // stable default account on first sight; every other root
            // waits for explicit user connect.
            out.push(crate::dto::CandidateDto {
                root_hash: found.root_hash,
                provider_id: descriptor.provider_id.into(),
                product_id: descriptor.product_id.into(),
                root_hint: found.hint,
                kind: kind.into(),
                status,
                bound_account_id: bound_account,
            });
        }
    }
    Ok(out)
}

/// Explicit user connect: bind a discovered root to a fresh managed
/// account. Fails if the physical root already belongs to another
/// account — rebinding is never silent.
#[tauri::command]
pub fn connect_candidate(
    root_hash: String,
    alias: String,
    state: tauri::State<'_, AppState>,
) -> Result<AccountDto, String> {
    use usage_host::CancellationToken;
    use usage_runtime::discovery::discover_profile_roots;

    let alias = alias.trim();
    if alias.is_empty() || alias.len() > 64 {
        return Err("invalid_alias".into());
    }
    if root_hash.len() != 64 || !root_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid_root".into());
    }
    // Re-resolve the hash to a path with a fresh bounded scan: raw
    // paths are never persisted, so connect re-derives them.
    let mut resolved: Option<(String, String, String, std::path::PathBuf)> = None;
    let managed: Vec<ManagedAccount> = state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .list_managed_accounts()
        .map_err(|_| "accounts_unavailable")?;
    for descriptor in usage_providers::descriptor::all_descriptors()
        .iter()
        .filter(|d| d.account_model == AccountModel::LocalClient)
    {
        let custom_roots: Vec<std::path::PathBuf> = managed
            .iter()
            .filter(|a| a.product_id.as_deref() == Some(descriptor.product_id))
            .filter_map(|a| a.custom_path.clone().map(std::path::PathBuf::from))
            .collect();
        for found in discover_profile_roots(
            &state.hosts,
            descriptor,
            &custom_roots,
            &CancellationToken::new(),
        ) {
            if found.valid && found.root_hash == root_hash {
                resolved = Some((
                    descriptor.provider_id.into(),
                    descriptor.product_id.into(),
                    found.hint.clone(),
                    found.path,
                ));
                break;
            }
        }
        if resolved.is_some() {
            break;
        }
    }
    let Some((provider_id, product_id, hint, path)) = resolved else {
        return Err("candidate_not_found".into());
    };
    let _ = hint;
    let id = Uuid::new_v4();
    let managed_account = ManagedAccount {
        id,
        provider_id: provider_id.clone(),
        product_id: Some(product_id.clone()),
        external_identity: None,
        label: alias.into(),
        connection_ref: Some(format!("local:{root_hash:.12}")),
        lifecycle: AccountLifecycle::Active,
        enabled: true,
        custom_path: Some(path.to_string_lossy().into_owned()),
        identity_confidence: usage_core::IdentityConfidence::Unknown,
    };
    {
        let mut storage = state.storage.lock().map_err(|_| "storage_lock")?;
        storage
            .create_managed_account(&managed_account)
            .map_err(|_| "account_create_failed")?;
        // ensure_connection rejects silently rebinding a root that
        // another account already owns.
        let root_hint = managed_account
            .custom_path
            .as_deref()
            .unwrap_or("custom")
            .rsplit('/')
            .next()
            .unwrap_or("custom")
            .to_string();
        storage
            .ensure_connection(
                id,
                &provider_id,
                &product_id,
                &root_hash,
                &root_hint,
                "custom-local",
            )
            .map_err(|_| "root_already_bound")?;
        storage
            .set_candidate_status(&root_hash, "connected")
            .map_err(|_| "candidate_store_failed")?;
    }
    Ok(managed_account_to_dto(&managed_account))
}

/// Explicit user ignore: the root stays silent across rescans.
#[tauri::command]
pub fn ignore_candidate(
    root_hash: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    if root_hash.len() != 64 || !root_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid_root".into());
    }
    Ok(state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .set_candidate_status(&root_hash, "ignored")
        .map_err(|_| "candidate_store_failed")?)
}
