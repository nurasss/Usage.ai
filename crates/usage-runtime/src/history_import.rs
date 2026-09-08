use std::path::PathBuf;
use std::sync::Mutex;
use usage_core::{Account, ProviderAdapter};
use usage_host::Hosts;
use usage_storage::{ImportCheckpoint, Storage};

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

/// Scoped history import for one local product:
/// FilesHost discovery over product roots, rotation-aware resume
/// offsets, single-transaction batch commits, aggregated warnings.
pub async fn import_product(
    hosts: &Hosts,
    storage: &Mutex<Storage>,
    adapter: &dyn ProviderAdapter,
    account: &Account,
    roots: Vec<PathBuf>,
    glob_pattern: &str,
    schema_version: u32,
) -> ImportReport {
    let mut report = ImportReport::default();
    for root in roots {
        // Blocking filesystem discovery stays off the async executor.
        let files = hosts.files.clone();
        let pattern = glob_pattern.to_string();
        let discovered = tokio::task::spawn_blocking(move || {
            files.discover(&root, &pattern, false).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        report.files_discovered += discovered.len();
        for file in discovered {
            let identity = &file.identity;
            let (old_offset, offset) = match storage.lock() {
                Ok(mut storage) => {
                    let old = storage
                        .checkpoint(&identity.path_hash)
                        .ok()
                        .flatten()
                        .map(|c| c.byte_offset)
                        .unwrap_or(0);
                    let offset = storage
                        .offset_for_observation(
                            &identity.path_hash,
                            identity.size,
                            identity.mtime_ms,
                            identity.device as i64,
                            identity.inode as i64,
                        )
                        .unwrap_or(0);
                    (old, offset)
                }
                Err(_) => continue,
            };
            if offset == 0 && old_offset > 0 {
                report.checkpoint_resets += 1;
            }
            let batch = match adapter.import_local(account, &file.path, offset).await {
                Ok(batch) => batch,
                Err(_) => continue,
            };
            let accepted = match storage.lock() {
                Ok(mut storage) => storage.import_usage_batch(&batch.records).unwrap_or(0),
                Err(_) => continue,
            };
            report.files_imported += 1;
            report.records_accepted += accepted;
            report.malformed += batch
                .warnings
                .iter()
                .filter(|w| w.starts_with("invalid_json"))
                .count();
            report.warnings.extend(batch.warnings);
            if let Ok(mut storage) = storage.lock() {
                let _ = storage.set_checkpoint(&ImportCheckpoint {
                    path_hash: identity.path_hash.clone(),
                    mtime_ms: identity.mtime_ms,
                    size: identity.size,
                    byte_offset: batch.next_offset,
                    schema_version,
                    last_source_record_id: batch.records.last().map(|r| r.source_record_id.clone()),
                    device: identity.device as i64,
                    inode: identity.inode as i64,
                });
            }
        }
    }
    report.warnings.truncate(64);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use usage_core::AccountLifecycle;
    use usage_host::{MemoryKeychain, NullLogger, ObservedNetwork, ScopedFiles, SystemClock};
    use usage_providers::claude::ClaudeLocalAdapter;
    use uuid::Uuid;

    fn test_hosts() -> Hosts {
        use std::sync::Arc;
        Hosts {
            clock: Arc::new(SystemClock),
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http: Arc::new(usage_host::ReqwestHttpHost::default()),
            files: Arc::new(ScopedFiles),
            network: Arc::new(ObservedNetwork::default()),
        }
    }

    #[tokio::test]
    async fn rotation_replacement_reimports_from_zero() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects").join("p");
        std::fs::create_dir_all(&projects).unwrap();
        let file = projects.join("session.jsonl");
        let line = "{\"timestamp\":\"2026-09-08T12:00:00Z\",\"uuid\":\"u1\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20}}}\n";
        std::fs::write(&file, line).unwrap();
        let storage = Mutex::new(Storage::in_memory().unwrap());
        let account = Account {
            id: Uuid::nil(),
            provider_id: "anthropic".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: AccountLifecycle::Active,
        };
        let adapter = ClaudeLocalAdapter::default();
        let first = import_product(
            &test_hosts(),
            &storage,
            &adapter,
            &account,
            vec![dir.path().join("projects")],
            "p/*.jsonl",
            1,
        )
        .await;
        assert_eq!(first.files_imported, 1);
        assert_eq!(first.records_accepted, 1);
        // Replace the file with different content but same name.
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, line.replace("u1", "u2")).unwrap();
        let second = import_product(
            &test_hosts(),
            &storage,
            &adapter,
            &account,
            vec![dir.path().join("projects")],
            "p/*.jsonl",
            1,
        )
        .await;
        assert_eq!(second.records_accepted, 1);
        // Both records coexist: different stable identities, no overwrite.
        assert_eq!(storage.lock().unwrap().usage_count().unwrap(), 2);
    }
}
