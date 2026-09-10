use std::path::PathBuf;
use std::sync::Mutex;
use usage_core::Account;
use usage_host::{policy, CancellationToken, Hosts};
use usage_providers::strategy::FileLogParser;
use usage_storage::Storage;

use crate::account_router::attribute_local_import;

/// Outcome counters for import diagnostics UI. Raw paths never leave
/// this module — only counts and safe warning codes.
#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    pub files_discovered: usize,
    pub files_imported: usize,
    pub records_accepted: usize,
    pub malformed: usize,
    pub checkpoint_resets: usize,
    pub warnings: Vec<String>,
}

/// All inputs for one connection import. Keeping the source, storage and
/// import policy together makes it difficult to accidentally mix roots or
/// accounts when adding another import call site.
pub struct ImportRequest<'a> {
    pub hosts: &'a Hosts,
    pub storage: &'a Mutex<Storage>,
    pub parser: &'a dyn FileLogParser,
    pub account: &'a Account,
    pub connection_id: &'a str,
    pub root: PathBuf,
    pub glob_pattern: &'a str,
    pub schema_version: u32,
    pub cancel: &'a CancellationToken,
}

/// One explicit source connection import: a single product root is
/// discovered, parsed and committed under exactly one account. Roots
/// are never merged; each connection carries its own file rows,
/// generations and offsets, so custom roots cannot pollute the
/// default account and replacement in one file cannot inflate totals.
pub async fn import_connection(request: ImportRequest<'_>) -> ImportReport {
    let ImportRequest {
        hosts,
        storage,
        parser,
        account,
        connection_id,
        root,
        glob_pattern,
        schema_version,
        cancel,
    } = request;
    let mut report = ImportReport::default();
    // Blocking filesystem discovery stays off the async executor.
    let files = hosts.files.clone();
    let file_scope = hosts.file_scope.clone();
    let pattern = glob_pattern.to_string();
    let cancel_probe = cancel.clone();
    let discovered = tokio::task::spawn_blocking(move || {
        let scoped = file_scope.scope_root(&root, &cancel_probe)?;
        files.list_files(&scoped, &pattern, &cancel_probe)
    })
    .await
    .unwrap_or(Err(usage_host::HostError::Cancelled))
    .unwrap_or_default();
    report.files_discovered = discovered.len();
    for file in discovered {
        if cancel.is_cancelled() {
            break;
        }
        // Per-connection identity routing: unattributed local data
        // flows only into explicit unattributed connections; rows are
        // never written into a previously confirmed account.
        if !connection_accepts_import(storage, connection_id, account.id) {
            report.warnings.push("identity_not_attributable".into());
            continue;
        }
        let identity = file.identity().clone();
        let file_row = match storage.lock() {
            Ok(mut storage) => storage.ensure_source_file(
                connection_id,
                &identity.path_hash,
                identity.device as i64,
                identity.inode as i64,
            ),
            Err(_) => continue,
        };
        let Ok(file_row) = file_row else { continue };
        let (old_offset, resume_offset, is_replacement, new_generation) = match storage.lock() {
            Ok(storage) => {
                let state = storage.file_import_state(&file_row.id).ok().flatten();
                let old_offset = state.as_ref().map(|s| s.byte_offset).unwrap_or(0);
                let replaced = file_replaced(&file_row, old_offset, &identity);
                if replaced {
                    (old_offset, 0, true, file_row.generation + 1)
                } else {
                    (
                        old_offset,
                        old_offset.min(identity.size),
                        false,
                        file_row.generation,
                    )
                }
            }
            Err(_) => continue,
        };
        if is_replacement && old_offset > 0 {
            report.checkpoint_resets += 1;
        }
        let bytes = match hosts.files.read_scoped_range(
            &file,
            resume_offset,
            policy::MAX_FILE_READ_BYTES,
            cancel,
        ) {
            Ok(bytes) => bytes,
            Err(usage_host::HostError::Cancelled) => break,
            Err(_) => continue,
        };
        let chunk = parser.parse_chunk(account, &identity.path_hash, resume_offset, &bytes);
        // Stamp provenance before the atomic commit: every row knows
        // its connection, file and generation.
        let mut records = chunk.records;
        for record in &mut records {
            record.connection_id = Some(connection_id.to_string());
            record.file_id = Some(file_row.id.clone());
            record.file_generation = Some(new_generation);
        }
        let next_offset = resume_offset + chunk.consumed;
        let committed = match storage.lock() {
            Ok(mut storage) => storage.commit_import_batch(&usage_storage::ImportBatchCommit {
                file_id: &file_row.id,
                file_generation: new_generation,
                is_replacement,
                device: identity.device as i64,
                inode: identity.inode as i64,
                records: &records,
                byte_offset: next_offset,
                schema_version,
                last_source_record_id: records.last().map(|r| r.source_record_id.as_str()),
            }),
            Err(_) => continue,
        };
        match committed {
            Ok(accepted) => {
                report.files_imported += 1;
                report.records_accepted += accepted;
            }
            Err(_) => continue,
        }
        report.malformed += chunk.malformed;
        report.warnings.extend(chunk.warnings);
    }
    report.warnings.truncate(64);
    report
}

/// Replacement is proven by file identity change (device/inode where
/// the platform exposes them) or by truncation below the stored
/// offset. Mtime alone never proves anything.
fn file_replaced(
    stored: &usage_storage::SourceFile,
    old_offset: u64,
    current: &usage_host::FileIdentity,
) -> bool {
    let identity_changed =
        (stored.device != 0 || stored.inode != 0 || current.device != 0 || current.inode != 0)
            && (stored.device, stored.inode) != (current.device as i64, current.inode as i64);
    identity_changed || current.size < old_offset
}

fn connection_accepts_import(
    storage: &Mutex<Storage>,
    connection_id: &str,
    account_id: uuid::Uuid,
) -> bool {
    let row = storage
        .lock()
        .ok()
        .and_then(|s| s.get_connection(connection_id).ok())
        .flatten();
    let Some(row) = row else { return false };
    if row.account_id != account_id.to_string() {
        return false;
    }
    attribute_local_import(
        row.identity_fingerprint.as_deref(),
        row.identity_confidence.as_str(),
        None,
    ) == crate::account_router::LocalAttribution::Proceed
}

#[cfg(test)]
mod connection_tests {
    use super::*;
    use std::io::Write;
    use usage_core::AccountLifecycle;
    use usage_host::{
        CancellationToken, MemoryKeychain, NullLogger, ObservedNetwork, ReqwestHttpHost,
        ScopedFiles, SystemClock,
    };
    use usage_providers::{claude::ClaudeJsonlStrategy, codex::CodexJsonlStrategy};
    use uuid::Uuid;

    fn test_hosts() -> Hosts {
        use std::sync::Arc;
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

    fn local_account(id: Uuid, provider: &str, label: &str) -> Account {
        Account {
            id,
            provider_id: provider.into(),
            external_identity: None,
            label: label.into(),
            connection_ref: None,
            lifecycle: AccountLifecycle::Active,
        }
    }

    fn copy_fixture(source: &str, dest: &std::path::Path) {
        let origin =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../usage-providers/fixtures");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(origin.join(source), dest).unwrap();
    }

    fn token_sum(storage: &Mutex<Storage>) -> u64 {
        use chrono::TimeZone;
        let start = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let end = chrono::Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        let storage = storage.lock().unwrap();
        storage
            .token_totals_between(start, end)
            .unwrap()
            .into_iter()
            .map(|(_, v)| v)
            .sum()
    }

    fn seed_account(storage: &Mutex<Storage>, account: &Account) {
        storage.lock().unwrap().upsert_account(account).unwrap();
    }

    #[tokio::test]
    async fn codex_fixture_end_to_end_matches_expected_totals() {
        // session-a (510) + session-b (15) + archived (10) = 535.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex-home");
        let sessions = home.join("sessions");
        copy_fixture(
            "codex/session-a.jsonl",
            &sessions.join("a").join("rollout-a.jsonl"),
        );
        copy_fixture(
            "codex/session-b.jsonl",
            &sessions.join("b").join("rollout-b.jsonl"),
        );
        copy_fixture(
            "codex/archived/old.jsonl",
            &sessions.join("archived").join("rollout-old.jsonl"),
        );
        let hosts = test_hosts();
        let storage = Mutex::new(Storage::in_memory().unwrap());
        let account = local_account(Uuid::new_v4(), "openai", "T");
        seed_account(&storage, &account);
        let connection = storage
            .lock()
            .unwrap()
            .ensure_connection(
                account.id,
                "openai",
                "codex",
                "roothash",
                "default",
                "default-local",
            )
            .unwrap();
        let report = import_connection(ImportRequest {
            hosts: &hosts,
            storage: &storage,
            parser: &CodexJsonlStrategy::new(None),
            account: &account,
            connection_id: &connection.id,
            root: home,
            glob_pattern: "sessions/**/rollout-*.jsonl",
            schema_version: 1,
            cancel: &CancellationToken::new(),
        })
        .await;
        assert_eq!(report.files_imported, 3);
        assert_eq!(report.records_accepted, 5);
        assert_eq!(token_sum(&storage), 535);
        // Re-import resumes at offsets: totals do not move.
        let again = import_connection(ImportRequest {
            hosts: &hosts,
            storage: &storage,
            parser: &CodexJsonlStrategy::new(None),
            account: &account,
            connection_id: &connection.id,
            root: dir.path().join("codex-home"),
            glob_pattern: "sessions/**/rollout-*.jsonl",
            schema_version: 1,
            cancel: &CancellationToken::new(),
        })
        .await;
        assert_eq!(again.records_accepted, 0);
        assert_eq!(token_sum(&storage), 535);
    }

    #[tokio::test]
    async fn claude_streaming_fixture_collapses_to_final_values() {
        // m1 final (30/40) + m2 (5/5) = 80 total tokens in storage.
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        copy_fixture("claude/thread-a.jsonl", &projects.join("p").join("t.jsonl"));
        let hosts = test_hosts();
        let storage = Mutex::new(Storage::in_memory().unwrap());
        let account = local_account(Uuid::new_v4(), "anthropic", "T");
        seed_account(&storage, &account);
        let connection = storage
            .lock()
            .unwrap()
            .ensure_connection(
                account.id,
                "anthropic",
                "claude-code",
                "roothash",
                "default",
                "default-local",
            )
            .unwrap();
        let report = import_connection(ImportRequest {
            hosts: &hosts,
            storage: &storage,
            parser: &ClaudeJsonlStrategy::new(None),
            account: &account,
            connection_id: &connection.id,
            root: projects,
            glob_pattern: "p/*.jsonl",
            schema_version: 1,
            cancel: &CancellationToken::new(),
        })
        .await;
        assert_eq!(report.records_accepted, 2);
        assert_eq!(token_sum(&storage), 80);
    }

    #[tokio::test]
    async fn runtime_import_fault_rolls_back_rows_checkpoint_and_generation() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex-home");
        let file = home.join("sessions").join("s").join("rollout-fault.jsonl");
        let first_line = "{\"timestamp\":\"2026-09-08T12:00:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}}\n";
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, first_line).unwrap();
        let hosts = test_hosts();
        let storage = Mutex::new(Storage::in_memory().unwrap());
        let account = local_account(Uuid::new_v4(), "openai", "T");
        seed_account(&storage, &account);
        let connection = storage
            .lock()
            .unwrap()
            .ensure_connection(
                account.id,
                "openai",
                "codex",
                "fault-root",
                "root",
                "default-local",
            )
            .unwrap();
        let parser = CodexJsonlStrategy::new(None);
        let cancel = CancellationToken::new();
        let import = || async {
            import_connection(ImportRequest {
                hosts: &hosts,
                storage: &storage,
                parser: &parser,
                account: &account,
                connection_id: &connection.id,
                root: home.clone(),
                glob_pattern: "sessions/**/rollout-*.jsonl",
                schema_version: 1,
                cancel: &cancel,
            })
            .await
        };
        let initial = import().await;
        assert_eq!(initial.records_accepted, 1);
        let path_hash = usage_host::files::path_hash(&file.canonicalize().unwrap());
        let file_row = storage
            .lock()
            .unwrap()
            .ensure_source_file(&connection.id, &path_hash, 0, 0)
            .unwrap();
        let before_state = storage
            .lock()
            .unwrap()
            .file_import_state(&file_row.id)
            .unwrap()
            .unwrap();
        let before_generation = storage
            .lock()
            .unwrap()
            .get_source_file(&file_row.id)
            .unwrap()
            .unwrap()
            .generation;

        let second_line = "{\"timestamp\":\"2026-09-08T12:01:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":2,\"output_tokens\":2,\"total_tokens\":4}}}}\n";
        std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(second_line.as_bytes())
            .unwrap();
        storage
            .lock()
            .unwrap()
            .inject_fault(usage_storage::StorageFault::ImportBeforeCheckpoint);
        let failed = import().await;
        assert_eq!(failed.files_imported, 0);
        assert_eq!(token_sum(&storage), 2);
        assert_eq!(
            storage
                .lock()
                .unwrap()
                .file_import_state(&file_row.id)
                .unwrap()
                .unwrap()
                .byte_offset,
            before_state.byte_offset
        );
        assert_eq!(
            storage
                .lock()
                .unwrap()
                .get_source_file(&file_row.id)
                .unwrap()
                .unwrap()
                .generation,
            before_generation
        );
    }

    #[tokio::test]
    async fn default_and_custom_roots_never_share_rows() {
        // B-01: two roots, two accounts, two connections. No row may
        // reference the other connection; re-import changes nothing.
        let dir = tempfile::tempdir().unwrap();
        let home_a = dir.path().join("home-a");
        let home_b = dir.path().join("home-b");
        copy_fixture(
            "codex/session-a.jsonl",
            &home_a.join("sessions").join("x").join("rollout-a.jsonl"),
        );
        copy_fixture(
            "codex/session-b.jsonl",
            &home_b.join("sessions").join("y").join("rollout-b.jsonl"),
        );
        let hosts = test_hosts();
        let storage = Mutex::new(Storage::in_memory().unwrap());
        let acc_a = local_account(Uuid::new_v4(), "openai", "A");
        let acc_b = local_account(Uuid::new_v4(), "openai", "B");
        seed_account(&storage, &acc_a);
        seed_account(&storage, &acc_b);
        let conn_a = storage
            .lock()
            .unwrap()
            .ensure_connection(acc_a.id, "openai", "codex", "hash-a", "a", "default-local")
            .unwrap();
        let conn_b = storage
            .lock()
            .unwrap()
            .ensure_connection(acc_b.id, "openai", "codex", "hash-b", "b", "custom-local")
            .unwrap();
        let parser = CodexJsonlStrategy::new(None);
        let ra = import_connection(ImportRequest {
            hosts: &hosts,
            storage: &storage,
            parser: &parser,
            account: &acc_a,
            connection_id: &conn_a.id,
            root: home_a,
            glob_pattern: "sessions/**/rollout-*.jsonl",
            schema_version: 1,
            cancel: &CancellationToken::new(),
        })
        .await;
        let rb = import_connection(ImportRequest {
            hosts: &hosts,
            storage: &storage,
            parser: &parser,
            account: &acc_b,
            connection_id: &conn_b.id,
            root: home_b,
            glob_pattern: "sessions/**/rollout-*.jsonl",
            schema_version: 1,
            cancel: &CancellationToken::new(),
        })
        .await;
        assert_eq!((ra.records_accepted, rb.records_accepted), (3, 1));
        let attribution = storage.lock().unwrap().record_attribution().unwrap();
        assert!(!attribution.is_empty());
        for row in attribution {
            if row.account_id == acc_a.id.to_string() {
                assert_eq!(row.connection_id.as_deref(), Some(conn_a.id.as_str()));
            } else {
                assert_eq!(row.connection_id.as_deref(), Some(conn_b.id.as_str()));
            }
        }
    }

    #[tokio::test]
    async fn replacement_rebuilds_totals_without_inflation() {
        // P0-01: v1 rows vanish in the same transaction that lands v2.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let file = home.join("sessions").join("s").join("rollout-r.jsonl");
        copy_fixture("codex/duplicates.jsonl", &file);
        let hosts = test_hosts();
        let storage = Mutex::new(Storage::in_memory().unwrap());
        let account = local_account(Uuid::new_v4(), "openai", "T");
        seed_account(&storage, &account);
        let connection = storage
            .lock()
            .unwrap()
            .ensure_connection(
                account.id,
                "openai",
                "codex",
                "rh",
                "default",
                "default-local",
            )
            .unwrap();
        let parser = CodexJsonlStrategy::new(None);
        let import = |home: std::path::PathBuf| async {
            let cancel = CancellationToken::new();
            import_connection(ImportRequest {
                hosts: &hosts,
                storage: &storage,
                parser: &parser,
                account: &account,
                connection_id: &connection.id,
                root: home,
                glob_pattern: "sessions/**/rollout-*.jsonl",
                schema_version: 1,
                cancel: &cancel,
            })
            .await
        };
        import(home.clone()).await;
        assert_eq!(token_sum(&storage), 30);
        let first_attribution = storage.lock().unwrap().record_attribution().unwrap();
        assert_eq!(first_attribution.len(), 2);
        let file_id = first_attribution[0].file_id.clone().unwrap();
        assert_eq!(
            first_attribution[0].connection_id.as_deref(),
            Some(connection.id.as_str())
        );
        let first_state = storage
            .lock()
            .unwrap()
            .file_import_state(&file_id)
            .unwrap()
            .unwrap();
        assert!(first_state.byte_offset > 0);
        // Replace the transcript with shorter, different content.
        std::fs::remove_file(&file).unwrap();
        std::fs::write(
            &file,
            "{\"timestamp\":\"2026-09-08T12:00:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}}\n",
        )
        .unwrap();
        let report = import(home).await;
        assert_eq!(report.checkpoint_resets, 1);
        assert_eq!(token_sum(&storage), 2);
        let second_attribution = storage.lock().unwrap().record_attribution().unwrap();
        assert_eq!(second_attribution.len(), 1);
        assert_eq!(
            second_attribution[0].file_id.as_deref(),
            Some(file_id.as_str())
        );
        assert_eq!(second_attribution[0].file_generation, Some(2));
        assert_eq!(
            second_attribution[0].connection_id.as_deref(),
            Some(connection.id.as_str())
        );
        let second_state = storage
            .lock()
            .unwrap()
            .file_import_state(&file_id)
            .unwrap()
            .unwrap();
        assert!(second_state.byte_offset < first_state.byte_offset);
        assert_eq!(
            storage
                .lock()
                .unwrap()
                .get_source_file(&file_id)
                .unwrap()
                .unwrap()
                .generation,
            2
        );
    }

    #[tokio::test]
    async fn confirmed_connection_rejects_unattributable_rows() {
        // B-02: a Weak-confirmed connection refuses rows from a source
        // that cannot prove identity.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        copy_fixture(
            "codex/session-a.jsonl",
            &home.join("sessions").join("s").join("rollout-a.jsonl"),
        );
        let hosts = test_hosts();
        let storage = Mutex::new(Storage::in_memory().unwrap());
        let account = local_account(Uuid::new_v4(), "openai", "T");
        seed_account(&storage, &account);
        let connection = storage
            .lock()
            .unwrap()
            .ensure_connection(
                account.id,
                "openai",
                "codex",
                "rh",
                "default",
                "default-local",
            )
            .unwrap();
        storage
            .lock()
            .unwrap()
            .set_connection_identity(&connection.id, Some("fp-confirmed"), "Weak")
            .unwrap();
        let report = import_connection(ImportRequest {
            hosts: &hosts,
            storage: &storage,
            parser: &CodexJsonlStrategy::new(None),
            account: &account,
            connection_id: &connection.id,
            root: home,
            glob_pattern: "sessions/**/rollout-*.jsonl",
            schema_version: 1,
            cancel: &CancellationToken::new(),
        })
        .await;
        assert_eq!(report.records_accepted, 0);
        assert!(report
            .warnings
            .contains(&"identity_not_attributable".to_string()));
        assert_eq!(token_sum(&storage), 0);
    }

    #[tokio::test]
    async fn archived_sessions_sibling_is_discovered_and_imported() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let archived = home.join("archived_sessions");
        copy_fixture(
            "codex/archived/old.jsonl",
            &archived.join("archive-1").join("rollout-archived.jsonl"),
        );
        let hosts = test_hosts();
        let storage = Mutex::new(Storage::in_memory().unwrap());
        let account = local_account(Uuid::new_v4(), "openai", "T");
        seed_account(&storage, &account);
        let connection = storage
            .lock()
            .unwrap()
            .ensure_connection(
                account.id,
                "openai",
                "codex",
                "archived-root",
                "default",
                "default-local",
            )
            .unwrap();
        let descriptor = usage_providers::descriptor::find_descriptor("openai", "codex").unwrap();
        let report = import_connection(ImportRequest {
            hosts: &hosts,
            storage: &storage,
            parser: &CodexJsonlStrategy::new(None),
            account: &account,
            connection_id: &connection.id,
            root: home,
            glob_pattern: descriptor.local_glob.unwrap(),
            schema_version: 1,
            cancel: &CancellationToken::new(),
        })
        .await;
        assert_eq!(report.files_discovered, 1);
        assert_eq!(report.records_accepted, 1);
        assert_eq!(token_sum(&storage), 10);
    }
}
