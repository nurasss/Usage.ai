use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{
    fs,
    path::{Path, PathBuf},
};
use usage_core::{Account, CostRecord, UsageRecord};

const SCHEMA_VERSION: u32 = 9;

const USAGE_UPSERT_SQL: &str = "INSERT INTO usage_records(account_id,product_id,billing_scope_id,source_record_id,period_start,period_end,model,input_tokens,output_tokens,cached_tokens,cache_creation_tokens,cache_read_tokens,reasoning_tokens,total_tokens,requests,source,coverage,connection_id,file_id,file_generation) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(account_id,product_id,source,source_record_id) DO UPDATE SET billing_scope_id=excluded.billing_scope_id,period_start=MAX(usage_records.period_start,excluded.period_start),period_end=MAX(usage_records.period_end,excluded.period_end),model=COALESCE(usage_records.model,excluded.model),input_tokens=CASE WHEN excluded.input_tokens IS NULL THEN usage_records.input_tokens WHEN usage_records.input_tokens IS NULL THEN excluded.input_tokens ELSE MAX(usage_records.input_tokens,excluded.input_tokens) END,output_tokens=CASE WHEN excluded.output_tokens IS NULL THEN usage_records.output_tokens WHEN usage_records.output_tokens IS NULL THEN excluded.output_tokens ELSE MAX(usage_records.output_tokens,excluded.output_tokens) END,cached_tokens=CASE WHEN excluded.cached_tokens IS NULL THEN usage_records.cached_tokens WHEN usage_records.cached_tokens IS NULL THEN excluded.cached_tokens ELSE MAX(usage_records.cached_tokens,excluded.cached_tokens) END,cache_creation_tokens=CASE WHEN excluded.cache_creation_tokens IS NULL THEN usage_records.cache_creation_tokens WHEN usage_records.cache_creation_tokens IS NULL THEN excluded.cache_creation_tokens ELSE MAX(usage_records.cache_creation_tokens,excluded.cache_creation_tokens) END,cache_read_tokens=CASE WHEN excluded.cache_read_tokens IS NULL THEN usage_records.cache_read_tokens WHEN usage_records.cache_read_tokens IS NULL THEN excluded.cache_read_tokens ELSE MAX(usage_records.cache_read_tokens,excluded.cache_read_tokens) END,reasoning_tokens=CASE WHEN excluded.reasoning_tokens IS NULL THEN usage_records.reasoning_tokens WHEN usage_records.reasoning_tokens IS NULL THEN excluded.reasoning_tokens ELSE MAX(usage_records.reasoning_tokens,excluded.reasoning_tokens) END,total_tokens=CASE WHEN excluded.total_tokens IS NULL THEN usage_records.total_tokens WHEN usage_records.total_tokens IS NULL THEN excluded.total_tokens ELSE MAX(usage_records.total_tokens,excluded.total_tokens) END,requests=CASE WHEN excluded.requests IS NULL THEN usage_records.requests WHEN usage_records.requests IS NULL THEN excluded.requests ELSE MAX(usage_records.requests,excluded.requests) END,coverage=excluded.coverage,connection_id=COALESCE(excluded.connection_id,usage_records.connection_id),file_id=COALESCE(excluded.file_id,usage_records.file_id),file_generation=MAX(usage_records.file_generation,excluded.file_generation)";

pub struct Storage {
    conn: Connection,
    path: Option<PathBuf>,
    import_fault: Option<StorageFault>,
}

/// Deterministic fault injection for end-to-end import rollback tests. This
/// is never enabled by normal construction and is intentionally limited to a
/// failure boundary inside the atomic import transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFault {
    ImportBeforeCheckpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCheckpoint {
    pub path_hash: String,
    pub mtime_ms: i64,
    pub size: u64,
    pub byte_offset: u64,
    pub schema_version: u32,
    pub last_source_record_id: Option<String>,
    pub device: i64,
    pub inode: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileCandidate {
    pub root_hash: String,
    pub provider_id: String,
    pub product_id: String,
    pub root_hint: String,
    pub status: String,
    pub seen_at: String,
    pub connected_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceConnection {
    pub id: String,
    pub account_id: String,
    pub provider_id: String,
    pub product_id: String,
    pub root_hash: String,
    pub root_hint: String,
    pub kind: String,
    pub identity_fingerprint: Option<String>,
    pub identity_confidence: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFile {
    pub id: String,
    pub connection_id: String,
    pub path_hash: String,
    pub device: i64,
    pub inode: i64,
    pub generation: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileImportState {
    pub file_id: String,
    pub byte_offset: u64,
    pub schema_version: u32,
    pub last_source_record_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImportBatchCommit<'a> {
    pub file_id: &'a str,
    pub file_generation: i64,
    pub is_replacement: bool,
    pub device: i64,
    pub inode: i64,
    pub records: &'a [UsageRecord],
    pub byte_offset: u64,
    pub schema_version: u32,
    pub last_source_record_id: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitBundleResult {
    Committed(BundleReport),
    AccountInactive,
    AccountMissing,
}

pub trait PermissionHardener: Send + Sync {
    fn harden_permissions(&self, db_path: &Path) -> Result<()>;
}

#[derive(Debug, Default, Clone)]
pub struct SystemPermissionHardener;

impl PermissionHardener for SystemPermissionHardener {
    fn harden_permissions(&self, db_path: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            use anyhow::Context;
            use std::os::unix::fs::PermissionsExt;

            if let Some(parent) = db_path.parent() {
                if parent.exists() {
                    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                        .context("failed to chmod 0700 db parent directory")?;
                    let meta = parent
                        .metadata()
                        .context("failed to read parent metadata")?;
                    if meta.permissions().mode() & 0o077 != 0 {
                        anyhow::bail!(
                            "parent directory has insecure permissions: {:o}",
                            meta.permissions().mode()
                        );
                    }
                }
            }

            if db_path.exists() {
                std::fs::set_permissions(db_path, std::fs::Permissions::from_mode(0o600))
                    .context("failed to chmod 0600 db file")?;
                let meta = db_path
                    .metadata()
                    .context("failed to read db file metadata")?;
                if meta.permissions().mode() & 0o077 != 0 {
                    anyhow::bail!(
                        "db file has insecure permissions: {:o}",
                        meta.permissions().mode()
                    );
                }
            }

            for suffix in ["-wal", "-shm"] {
                let mut sidecar = db_path.as_os_str().to_os_string();
                sidecar.push(suffix);
                let sidecar_path = Path::new(&sidecar);
                if sidecar_path.exists() {
                    std::fs::set_permissions(sidecar_path, std::fs::Permissions::from_mode(0o600))
                        .context("failed to chmod 0600 sidecar file")?;
                    let meta = sidecar_path
                        .metadata()
                        .context("failed to read sidecar metadata")?;
                    if meta.permissions().mode() & 0o077 != 0 {
                        anyhow::bail!(
                            "sidecar file has insecure permissions: {:o}",
                            meta.permissions().mode()
                        );
                    }
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = db_path;
        }
        Ok(())
    }
}

impl Storage {
    pub fn open_with_hardener(
        path: impl AsRef<Path>,
        hardener: std::sync::Arc<dyn PermissionHardener>,
    ) -> Result<Self> {
        let path = path.as_ref();
        hardener.harden_permissions(path)?;
        if path.exists() {
            let probe = Connection::open(path)?;
            let version: u32 = probe
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap_or(0);
            drop(probe);
            if version > 0 && version < SCHEMA_VERSION {
                fs::copy(path, path.with_extension("db.pre-migration.bak"))
                    .context("create pre-migration backup")?;
            }
        }
        let conn = Connection::open(path)?;
        let mut storage = Self {
            conn,
            path: Some(path.to_path_buf()),
            import_fault: None,
        };
        storage.migrate()?;
        hardener.harden_permissions(path)?;
        Ok(storage)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_hardener(path, std::sync::Arc::new(SystemPermissionHardener))
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let mut s = Self {
            conn,
            path: None,
            import_fault: None,
        };
        s.migrate()?;
        Ok(s)
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Test-only control surface for proving callers observe transaction
    /// rollback at the runtime/storage boundary.
    pub fn inject_fault(&mut self, fault: StorageFault) {
        self.import_fault = Some(fault);
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.conn.pragma_update(None, "foreign_keys", true)?;
        let current: u32 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap_or(0);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if current < 1 {
            tx.execute_batch(include_str!("../migrations/001_initial.sql"))?;
        }
        if current < 2 {
            tx.execute_batch(include_str!("../migrations/002_accounts_budgets.sql"))?;
        }
        if current < 3 {
            tx.execute_batch(include_str!("../migrations/003_attempts_cooldowns.sql"))?;
        }
        if current < 4 {
            tx.execute_batch(include_str!("../migrations/004_identity_confidence.sql"))?;
        }
        if current < 5 {
            tx.execute_batch(include_str!("../migrations/005_provenance.sql"))?;
        }
        if current < 6 {
            tx.execute_batch(include_str!("../migrations/006_legacy_identity.sql"))?;
            migrate_legacy_rows(&tx)?;
        }
        if current < 7 {
            tx.execute_batch(include_str!("../migrations/007_claude_cache_buckets.sql"))?;
        }
        if current < 8 {
            tx.execute_batch(include_str!("../migrations/008_profile_candidates.sql"))?;
        }
        if current < 9 {
            tx.execute_batch(include_str!("../migrations/009_account_mute.sql"))?;
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_usage(&mut self, record: &UsageRecord) -> Result<()> {
        self.conn.execute(
            USAGE_UPSERT_SQL,
            params![
                record.account_id.to_string(),
                record.product_id,
                record.billing_scope_id,
                record.source_record_id,
                record.period_start.to_rfc3339(),
                record.period_end.to_rfc3339(),
                record.model,
                record.input_tokens,
                record.output_tokens,
                record.cached_tokens,
                record.cache_creation_tokens,
                record.cache_read_tokens,
                record.reasoning_tokens,
                record.total_tokens,
                record.requests,
                record.source,
                format!("{:?}", record.coverage),
                record.connection_id,
                record.file_id,
                record.file_generation.unwrap_or(1)
            ],
        )?;
        Ok(())
    }

    pub fn upsert_account(&mut self, account: &Account) -> Result<()> {
        self.conn.execute("INSERT INTO accounts(id,provider_id,external_identity_fingerprint,alias,connection_ref,lifecycle,created_at) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET provider_id=excluded.provider_id,external_identity_fingerprint=excluded.external_identity_fingerprint,alias=excluded.alias,connection_ref=excluded.connection_ref,lifecycle=excluded.lifecycle",params![account.id.to_string(),account.provider_id,account.external_identity,account.label,account.connection_ref,format!("{:?}",account.lifecycle),Utc::now().to_rfc3339()])?;
        Ok(())
    }

    pub fn archive_account(&mut self, account_id: uuid::Uuid) -> Result<()> {
        self.conn.execute(
            "UPDATE accounts SET lifecycle='Archived',connection_ref=NULL WHERE id=?",
            [account_id.to_string()],
        )?;
        Ok(())
    }

    pub fn delete_account_and_history(&mut self, account_id: uuid::Uuid) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Provenance children must be removed before their parent connection
        // rows because foreign keys are enabled for every storage handle.
        tx.execute(
            "DELETE FROM file_imports WHERE file_id IN (SELECT id FROM source_files WHERE connection_id IN (SELECT id FROM source_connections WHERE account_id=?))",
            [account_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM import_checkpoints WHERE path_hash IN (SELECT path_hash FROM source_files WHERE connection_id IN (SELECT id FROM source_connections WHERE account_id=?))",
            [account_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM source_files WHERE connection_id IN (SELECT id FROM source_connections WHERE account_id=?)",
            [account_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM source_connections WHERE account_id=?",
            [account_id.to_string()],
        )?;
        for table in [
            "snapshots",
            "usage_records",
            "cost_records",
            "budgets",
            "balances",
            "source_attempts",
            "cooldowns",
            "notification_deliveries",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE account_id=?"),
                [account_id.to_string()],
            )?;
        }
        tx.execute("DELETE FROM accounts WHERE id=?", [account_id.to_string()])?;
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_cost(&mut self, record: &CostRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO cost_records(account_id,product_id,billing_scope_id,source_record_id,period_start,period_end,amount_decimal,currency,kind,source,coverage) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(account_id,product_id,source,source_record_id) DO UPDATE SET amount_decimal=excluded.amount_decimal,currency=excluded.currency,kind=excluded.kind,coverage=excluded.coverage,period_start=excluded.period_start,period_end=excluded.period_end",
            params![record.account_id.to_string(),record.product_id,record.billing_scope_id,record.source_record_id,record.period_start.to_rfc3339(),record.period_end.to_rfc3339(),record.amount_decimal.to_string(),record.currency,format!("{:?}",record.kind),record.source,format!("{:?}",record.coverage)])?;
        Ok(())
    }

    pub fn set_checkpoint(&mut self, checkpoint: &ImportCheckpoint) -> Result<()> {
        self.conn.execute("INSERT INTO import_checkpoints(path_hash,mtime_ms,size,byte_offset,schema_version,last_source_record_id,device,inode,touched_at) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(path_hash) DO UPDATE SET mtime_ms=excluded.mtime_ms,size=excluded.size,byte_offset=excluded.byte_offset,schema_version=excluded.schema_version,last_source_record_id=excluded.last_source_record_id,device=excluded.device,inode=excluded.inode,touched_at=excluded.touched_at", params![checkpoint.path_hash,checkpoint.mtime_ms,checkpoint.size,checkpoint.byte_offset,checkpoint.schema_version,checkpoint.last_source_record_id,checkpoint.device,checkpoint.inode,Utc::now().to_rfc3339()])?;
        Ok(())
    }

    pub fn checkpoint(&self, path_hash: &str) -> Result<Option<ImportCheckpoint>> {
        Ok(self.conn.query_row("SELECT path_hash,mtime_ms,size,byte_offset,schema_version,last_source_record_id,device,inode FROM import_checkpoints WHERE path_hash=?", [path_hash], |r| Ok(ImportCheckpoint { path_hash:r.get(0)?,mtime_ms:r.get(1)?,size:r.get(2)?,byte_offset:r.get(3)?,schema_version:r.get(4)?,last_source_record_id:r.get(5)?,device:r.get(6)?,inode:r.get(7)? })).optional()?)
    }

    pub fn reset_checkpoint_on_rotation(
        &mut self,
        path_hash: &str,
        new_size: u64,
        new_mtime_ms: i64,
    ) -> Result<u64> {
        self.offset_for_observation(path_hash, new_size, new_mtime_ms, 0, 0)
    }

    /// Rotation/replacement-aware resume offset. Replacement is proven by
    /// device/inode change when the platform exposes them; otherwise by
    /// size shrink. A grown file with stable identity resumes at the
    /// stored offset.
    pub fn offset_for_observation(
        &mut self,
        path_hash: &str,
        new_size: u64,
        new_mtime_ms: i64,
        new_device: i64,
        new_inode: i64,
    ) -> Result<u64> {
        let Some(old) = self.checkpoint(path_hash)? else {
            return Ok(0);
        };
        let identity_changed =
            (old.device != 0 || old.inode != 0 || new_device != 0 || new_inode != 0)
                && (old.device, old.inode) != (new_device, new_inode);
        if identity_changed || new_size < old.size {
            return Ok(0);
        }
        if new_mtime_ms < old.mtime_ms && new_size == old.size {
            return Ok(0);
        }
        Ok(old.byte_offset.min(new_size))
    }

    /// Explicit source connection: one stable identity per
    /// (provider, product, root). Default and custom roots never share
    /// a connection, so their histories can never merge implicitly.
    /// The connection id is a deterministic UUID over the same triple.
    pub fn ensure_connection(
        &mut self,
        account_id: uuid::Uuid,
        provider_id: &str,
        product_id: &str,
        root_hash: &str,
        root_hint: &str,
        kind: &str,
    ) -> Result<SourceConnection> {
        let id = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("usage.ai/connection/{provider_id}/{product_id}/{root_hash}").as_bytes(),
        );
        if let Some(existing) = self.get_connection(&id.to_string())? {
            if existing.account_id != account_id.to_string() {
                anyhow::bail!(
                    "source connection already belongs to another account; explicit migration required"
                );
            }
            self.conn.execute(
                "UPDATE source_connections SET root_hint=?,kind=? WHERE id=?",
                params![root_hint, kind, id.to_string()],
            )?;
            return Ok(existing);
        }
        self.conn.execute(
            "INSERT INTO source_connections(id,account_id,provider_id,product_id,root_hash,root_hint,kind,created_at) VALUES(?,?,?,?,?,?,?,?)",
            params![
                id.to_string(),
                account_id.to_string(),
                provider_id,
                product_id,
                root_hash,
                root_hint,
                kind,
                Utc::now().to_rfc3339()
            ],
        )?;
        self.get_connection(&id.to_string())?
            .ok_or_else(|| anyhow::anyhow!("connection missing after upsert"))
    }

    pub fn get_connection(&self, id: &str) -> Result<Option<SourceConnection>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id,account_id,provider_id,product_id,root_hash,root_hint,kind,identity_fingerprint,identity_confidence FROM source_connections WHERE id=?",
                [id],
                |r| {
                    Ok(SourceConnection {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        provider_id: r.get(2)?,
                        product_id: r.get(3)?,
                        root_hash: r.get(4)?,
                        root_hint: r.get(5)?,
                        kind: r.get(6)?,
                        identity_fingerprint: r.get(7)?,
                        identity_confidence: r.get(8)?,
                    })
                },
            )
            .optional()?)
    }

    /// Binding lookup for discovery: which account (if any) already owns
    /// this physical root. Used to reject silent rebinding and to mark
    /// candidates as connected.
    pub fn connection_for_root(
        &self,
        provider_id: &str,
        product_id: &str,
        root_hash: &str,
    ) -> Result<Option<SourceConnection>> {
        let mut statement = self.conn.prepare(
            "SELECT id,account_id,provider_id,product_id,root_hash,root_hint,kind,identity_fingerprint,identity_confidence FROM source_connections WHERE provider_id=? AND product_id=? AND root_hash=? LIMIT 1",
        )?;
        let mut rows = statement.query_map(params![provider_id, product_id, root_hash], |r| {
            Ok(SourceConnection {
                id: r.get(0)?,
                account_id: r.get(1)?,
                provider_id: r.get(2)?,
                product_id: r.get(3)?,
                root_hash: r.get(4)?,
                root_hint: r.get(5)?,
                kind: r.get(6)?,
                identity_fingerprint: r.get(7)?,
                identity_confidence: r.get(8)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_connections(&self) -> Result<Vec<SourceConnection>> {
        let mut statement = self.conn.prepare(
            "SELECT id,account_id,provider_id,product_id,root_hash,root_hint,kind,identity_fingerprint,identity_confidence FROM source_connections",
        )?;
        let rows = statement
            .query_map([], |r| {
                Ok(SourceConnection {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    provider_id: r.get(2)?,
                    product_id: r.get(3)?,
                    root_hash: r.get(4)?,
                    root_hint: r.get(5)?,
                    kind: r.get(6)?,
                    identity_fingerprint: r.get(7)?,
                    identity_confidence: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_connection_identity(
        &mut self,
        connection_id: &str,
        fingerprint: Option<&str>,
        confidence: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE source_connections SET identity_fingerprint=?,identity_confidence=? WHERE id=?",
            params![fingerprint, confidence, connection_id],
        )?;
        Ok(())
    }

    /// Discovered-but-untrusted profile roots. A candidate never
    /// becomes an account/connection without an explicit user
    /// connect; `ignored` roots stay silent across rescans.
    pub fn upsert_candidate(
        &mut self,
        root_hash: &str,
        provider_id: &str,
        product_id: &str,
        root_hint: &str,
    ) -> Result<ProfileCandidate> {
        self.conn.execute(
            "INSERT INTO profile_candidates(root_hash,provider_id,product_id,root_hint,status,seen_at) VALUES(?,?,?,?,?,?) ON CONFLICT(root_hash) DO UPDATE SET root_hint=excluded.root_hint,seen_at=excluded.seen_at",
            params![
                root_hash,
                provider_id,
                product_id,
                root_hint,
                "pending",
                Utc::now().to_rfc3339()
            ],
        )?;
        // Never overwrite an explicit user decision on rescan.
        self.conn.execute(
            "UPDATE profile_candidates SET status=CASE WHEN status IN ('connected','ignored') THEN status ELSE 'pending' END WHERE root_hash=?",
            [root_hash],
        )?;
        self.get_candidate(root_hash)?
            .ok_or_else(|| anyhow::anyhow!("candidate missing after upsert"))
    }

    pub fn get_candidate(&self, root_hash: &str) -> Result<Option<ProfileCandidate>> {
        Ok(self
            .conn
            .query_row(
                "SELECT root_hash,provider_id,product_id,root_hint,status,seen_at,connected_at FROM profile_candidates WHERE root_hash=?",
                [root_hash],
                |r| {
                    Ok(ProfileCandidate {
                        root_hash: r.get(0)?,
                        provider_id: r.get(1)?,
                        product_id: r.get(2)?,
                        root_hint: r.get(3)?,
                        status: r.get(4)?,
                        seen_at: r.get(5)?,
                        connected_at: r.get(6)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn list_candidates(&self, only_pending: bool) -> Result<Vec<ProfileCandidate>> {
        let sql = if only_pending {
            "SELECT root_hash,provider_id,product_id,root_hint,status,seen_at,connected_at FROM profile_candidates WHERE status='pending' ORDER BY seen_at"
        } else {
            "SELECT root_hash,provider_id,product_id,root_hint,status,seen_at,connected_at FROM profile_candidates ORDER BY seen_at"
        };
        let mut statement = self.conn.prepare(sql)?;
        let rows = statement
            .query_map([], |r| {
                Ok(ProfileCandidate {
                    root_hash: r.get(0)?,
                    provider_id: r.get(1)?,
                    product_id: r.get(2)?,
                    root_hint: r.get(3)?,
                    status: r.get(4)?,
                    seen_at: r.get(5)?,
                    connected_at: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_candidate_status(&mut self, root_hash: &str, status: &str) -> Result<bool> {
        if !matches!(status, "pending" | "connected" | "ignored") {
            anyhow::bail!("invalid candidate status");
        }
        let changed = self.conn.execute(
            "UPDATE profile_candidates SET status=?,connected_at=CASE WHEN ?= 'connected' THEN ? ELSE connected_at END WHERE root_hash=?",
            params![status, status, Utc::now().to_rfc3339(), root_hash],
        )?;
        Ok(changed > 0)
    }

    /// Stable internal file identity per connection. The file UUID is
    /// deterministic over (connection, path_hash): the same logical
    /// file always maps to the same row, across restarts.
    pub fn ensure_source_file(
        &mut self,
        connection_id: &str,
        path_hash: &str,
        device: i64,
        inode: i64,
    ) -> Result<SourceFile> {
        let id = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("usage.ai/file/{connection_id}/{path_hash}").as_bytes(),
        );
        self.conn.execute(
            "INSERT INTO source_files(id,connection_id,path_hash,device,inode) VALUES(?,?,?,?,?) ON CONFLICT(id) DO NOTHING",
            params![id.to_string(), connection_id, path_hash, device, inode],
        )?;
        self.get_source_file(&id.to_string())?
            .ok_or_else(|| anyhow::anyhow!("source file missing after upsert"))
    }

    pub fn get_source_file(&self, id: &str) -> Result<Option<SourceFile>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id,connection_id,path_hash,device,inode,generation FROM source_files WHERE id=?",
                [id],
                |r| {
                    Ok(SourceFile {
                        id: r.get(0)?,
                        connection_id: r.get(1)?,
                        path_hash: r.get(2)?,
                        device: r.get(3)?,
                        inode: r.get(4)?,
                        generation: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn file_import_state(&self, file_id: &str) -> Result<Option<FileImportState>> {
        Ok(self
            .conn
            .query_row(
                "SELECT file_id,byte_offset,schema_version,last_source_record_id FROM file_imports WHERE file_id=?",
                [file_id],
                |r| {
                    Ok(FileImportState {
                        file_id: r.get(0)?,
                        byte_offset: r.get(1)?,
                        schema_version: r.get(2)?,
                        last_source_record_id: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Atomic file import commit (P0-04): in ONE transaction —
    /// generation bump + old-generation row rebuild + record upserts +
    /// offset/checkpoint update. A failure before commit leaves neither
    /// records nor checkpoint partially stored.
    pub fn commit_import_batch(&mut self, commit: &ImportBatchCommit<'_>) -> Result<usize> {
        let import_fault = self.import_fault.take();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if commit.is_replacement {
            tx.execute(
                "DELETE FROM usage_records WHERE file_id=? AND file_generation < ?",
                params![commit.file_id, commit.file_generation],
            )?;
            tx.execute(
                "UPDATE source_files SET device=?,inode=?,generation=? WHERE id=?",
                params![
                    commit.device,
                    commit.inode,
                    commit.file_generation,
                    commit.file_id
                ],
            )?;
        }
        let mut accepted = 0usize;
        {
            let mut statement = tx.prepare(USAGE_UPSERT_SQL)?;
            for record in commit.records {
                statement.execute(params![
                    record.account_id.to_string(),
                    record.product_id,
                    record.billing_scope_id,
                    record.source_record_id,
                    record.period_start.to_rfc3339(),
                    record.period_end.to_rfc3339(),
                    record.model,
                    record.input_tokens,
                    record.output_tokens,
                    record.cached_tokens,
                    record.cache_creation_tokens,
                    record.cache_read_tokens,
                    record.reasoning_tokens,
                    record.total_tokens,
                    record.requests,
                    record.source,
                    format!("{:?}", record.coverage),
                    record.connection_id,
                    record.file_id,
                    record.file_generation.unwrap_or(1)
                ])?;
                accepted += 1;
            }
        }
        if matches!(import_fault, Some(StorageFault::ImportBeforeCheckpoint)) {
            anyhow::bail!("injected_import_failure_before_checkpoint");
        }
        tx.execute(
            "INSERT INTO file_imports(file_id,byte_offset,schema_version,last_source_record_id,touched_at) VALUES(?,?,?,?,?) ON CONFLICT(file_id) DO UPDATE SET byte_offset=excluded.byte_offset,schema_version=excluded.schema_version,last_source_record_id=excluded.last_source_record_id,touched_at=excluded.touched_at",
            params![
                commit.file_id,
                commit.byte_offset,
                commit.schema_version,
                commit.last_source_record_id,
                Utc::now().to_rfc3339()
            ],
        )?;
        tx.commit()?;
        Ok(accepted)
    }

    /// Last-known-good snapshot store. A new provider result replaces the
    /// stored payload only after successful fetch/parse/validate/map and a
    /// transactional commit performed by the caller beforehand — a failed
    /// refresh therefore never destroys good data.
    pub fn save_snapshot(
        &mut self,
        account_id: uuid::Uuid,
        product_id: &str,
        payload_json: &str,
        observed_at: Option<DateTime<Utc>>,
        fetched_at: DateTime<Utc>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO snapshots(account_id,product_id,payload_json,observed_at,fetched_at) VALUES(?,?,?,?,?)",
            params![
                account_id.to_string(),
                product_id,
                payload_json,
                observed_at.map(|d| d.to_rfc3339()),
                fetched_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn last_known_good(
        &self,
        account_id: uuid::Uuid,
        product_id: &str,
    ) -> Result<Option<StoredSnapshot>> {
        Ok(self
            .conn
            .query_row(
                "SELECT account_id,product_id,payload_json,observed_at,fetched_at FROM snapshots WHERE account_id=? AND product_id=? ORDER BY id DESC LIMIT 1",
                params![account_id.to_string(), product_id],
                |r| {
                    Ok(StoredSnapshot {
                        account_id: r.get(0)?,
                        product_id: r.get(1)?,
                        payload_json: r.get(2)?,
                        observed_at: r.get(3)?,
                        fetched_at: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// One batch = one transaction. Either the whole file batch lands or
    /// the previous state stays intact.
    pub fn import_usage_batch(&mut self, records: &[UsageRecord]) -> Result<usize> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut accepted = 0usize;
        {
            let mut statement = tx.prepare(USAGE_UPSERT_SQL)?;
            for record in records {
                statement.execute(params![
                    record.account_id.to_string(),
                    record.product_id,
                    record.billing_scope_id,
                    record.source_record_id,
                    record.period_start.to_rfc3339(),
                    record.period_end.to_rfc3339(),
                    record.model,
                    record.input_tokens,
                    record.output_tokens,
                    record.cached_tokens,
                    record.cache_creation_tokens,
                    record.cache_read_tokens,
                    record.reasoning_tokens,
                    record.total_tokens,
                    record.requests,
                    record.source,
                    format!("{:?}", record.coverage),
                    record.connection_id,
                    record.file_id,
                    record.file_generation.unwrap_or(1)
                ])?;
                accepted += 1;
            }
        }
        tx.commit()?;
        Ok(accepted)
    }

    pub fn import_cost_batch(&mut self, records: &[CostRecord]) -> Result<usize> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut accepted = 0usize;
        {
            let mut statement = tx.prepare("INSERT INTO cost_records(account_id,product_id,billing_scope_id,source_record_id,period_start,period_end,amount_decimal,currency,kind,source,coverage) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(account_id,product_id,source,source_record_id) DO UPDATE SET amount_decimal=excluded.amount_decimal,currency=excluded.currency,kind=excluded.kind,coverage=excluded.coverage,period_start=excluded.period_start,period_end=excluded.period_end")?;
            for record in records {
                statement.execute(params![
                    record.account_id.to_string(),
                    record.product_id,
                    record.billing_scope_id,
                    record.source_record_id,
                    record.period_start.to_rfc3339(),
                    record.period_end.to_rfc3339(),
                    record.amount_decimal.to_string(),
                    record.currency,
                    format!("{:?}", record.kind),
                    record.source,
                    format!("{:?}", record.coverage)
                ])?;
                accepted += 1;
            }
        }
        tx.commit()?;
        Ok(accepted)
    }

    /// Single-transaction refresh commit: snapshot payload, usage batch
    /// and cost batch land atomically. A failure anywhere leaves the
    /// previous last-known-good state intact.
    pub fn commit_refresh_bundle_if_active(
        &mut self,
        bundle: &RefreshBundle<'_>,
    ) -> Result<CommitBundleResult> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let account_state: Option<(String, i64)> = tx
            .query_row(
                "SELECT lifecycle, enabled FROM accounts WHERE id = ?",
                params![bundle.account_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let Some((lifecycle, enabled)) = account_state else {
            return Ok(CommitBundleResult::AccountMissing);
        };

        if enabled == 0 || lifecycle == "Archived" || lifecycle == "Disconnected" {
            return Ok(CommitBundleResult::AccountInactive);
        }

        tx.execute(
            "INSERT INTO snapshots(account_id,product_id,payload_json,observed_at,fetched_at) VALUES(?,?,?,?,?)",
            params![
                bundle.account_id.to_string(),
                bundle.product_id,
                bundle.snapshot_payload_json,
                bundle.observed_at.map(|d| d.to_rfc3339()),
                bundle.fetched_at.to_rfc3339()
            ],
        )?;
        let mut usage_accepted = 0usize;
        {
            let mut statement = tx.prepare(USAGE_UPSERT_SQL)?;
            for record in bundle.usage {
                statement.execute(params![
                    record.account_id.to_string(),
                    record.product_id,
                    record.billing_scope_id,
                    record.source_record_id,
                    record.period_start.to_rfc3339(),
                    record.period_end.to_rfc3339(),
                    record.model,
                    record.input_tokens,
                    record.output_tokens,
                    record.cached_tokens,
                    record.cache_creation_tokens,
                    record.cache_read_tokens,
                    record.reasoning_tokens,
                    record.total_tokens,
                    record.requests,
                    record.source,
                    format!("{:?}", record.coverage),
                    record.connection_id,
                    record.file_id,
                    record.file_generation.unwrap_or(1)
                ])?;
                usage_accepted += 1;
            }
        }
        let mut costs_accepted = 0usize;
        {
            let mut statement = tx.prepare("INSERT INTO cost_records(account_id,product_id,billing_scope_id,source_record_id,period_start,period_end,amount_decimal,currency,kind,source,coverage) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(account_id,product_id,source,source_record_id) DO UPDATE SET amount_decimal=excluded.amount_decimal,currency=excluded.currency,kind=excluded.kind,coverage=excluded.coverage,period_start=excluded.period_start,period_end=excluded.period_end")?;
            for record in bundle.costs {
                statement.execute(params![
                    record.account_id.to_string(),
                    record.product_id,
                    record.billing_scope_id,
                    record.source_record_id,
                    record.period_start.to_rfc3339(),
                    record.period_end.to_rfc3339(),
                    record.amount_decimal.to_string(),
                    record.currency,
                    format!("{:?}", record.kind),
                    record.source,
                    format!("{:?}", record.coverage)
                ])?;
                costs_accepted += 1;
            }
        }
        tx.commit()?;
        Ok(CommitBundleResult::Committed(BundleReport {
            usage_accepted,
            costs_accepted,
        }))
    }

    pub fn commit_refresh_bundle(&mut self, bundle: &RefreshBundle<'_>) -> Result<BundleReport> {
        match self.commit_refresh_bundle_if_active(bundle)? {
            CommitBundleResult::Committed(report) => Ok(report),
            CommitBundleResult::AccountInactive => anyhow::bail!("account_inactive"),
            CommitBundleResult::AccountMissing => anyhow::bail!("account_missing"),
        }
    }

    pub fn record_attempt(&mut self, attempt: &AttemptLog<'_>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO source_attempts(account_id,product_id,source,status,safe_code,started_at,finished_at) VALUES(?,?,?,?,?,?,?)",
            params![
                attempt.account_id.to_string(),
                attempt.product_id,
                attempt.source,
                attempt.status,
                attempt.safe_code,
                attempt.started_at.to_rfc3339(),
                attempt.finished_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn recent_attempts(
        &self,
        account_id: uuid::Uuid,
        product_id: &str,
        limit: u64,
    ) -> Result<Vec<SourceAttempt>> {
        let mut statement = self.conn.prepare("SELECT account_id,product_id,source,status,safe_code,started_at,finished_at FROM source_attempts WHERE account_id=? AND product_id=? ORDER BY id DESC LIMIT ?")?;
        let rows = statement
            .query_map(
                params![account_id.to_string(), product_id, limit as i64],
                |r| {
                    Ok(SourceAttempt {
                        account_id: r.get(0)?,
                        product_id: r.get(1)?,
                        source: r.get(2)?,
                        status: r.get(3)?,
                        safe_code: r.get(4)?,
                        started_at: r.get(5)?,
                        finished_at: r.get(6)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_cooldown(
        &mut self,
        account_id: uuid::Uuid,
        source: &str,
        until_at: DateTime<Utc>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO cooldowns(account_id,source,until_at) VALUES(?,?,?) ON CONFLICT(account_id,source) DO UPDATE SET until_at=excluded.until_at",
            params![account_id.to_string(), source, until_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn load_cooldowns(&self) -> Result<Vec<(String, String, String)>> {
        let mut statement = self
            .conn
            .prepare("SELECT account_id,source,until_at FROM cooldowns WHERE until_at > ?")?;
        let rows = statement
            .query_map([Utc::now().to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn clear_cooldown(&mut self, account_id: uuid::Uuid, source: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM cooldowns WHERE account_id=? AND source=?",
            params![account_id.to_string(), source],
        )?;
        Ok(())
    }

    /// Retention covers every category explicitly. Active import
    /// checkpoints are never deleted by age — only ones untouched for
    /// longer than twice the history retention.
    pub fn prune_retention(&mut self, history_before: DateTime<Utc>) -> Result<RetentionReport> {
        let usage = self.conn.execute(
            "DELETE FROM usage_records WHERE period_end < ?",
            [history_before.to_rfc3339()],
        )?;
        let costs = self.conn.execute(
            "DELETE FROM cost_records WHERE period_end < ?",
            [history_before.to_rfc3339()],
        )?;
        // Retention must never break SWR: the newest successful
        // snapshot of every active account/product survives regardless
        // of the age cutoff.
        let snapshots = self.conn.execute(
            "DELETE FROM snapshots WHERE fetched_at < ? AND id NOT IN (SELECT MAX(id) FROM snapshots GROUP BY account_id, product_id)",
            [history_before.to_rfc3339()],
        )?;
        let attempts_cutoff = Utc::now() - chrono::Duration::days(30);
        let attempts = self.conn.execute(
            "DELETE FROM source_attempts WHERE finished_at < ?",
            [attempts_cutoff.to_rfc3339()],
        )?;
        let notifications_cutoff = Utc::now() - chrono::Duration::days(90);
        let notifications = self.conn.execute(
            "DELETE FROM notification_deliveries WHERE delivered_at < ?",
            [notifications_cutoff.to_rfc3339()],
        )?;
        let retention_days = Utc::now()
            .signed_duration_since(history_before)
            .num_days()
            .max(30);
        let stale_checkpoints_cutoff = Utc::now() - chrono::Duration::days(2 * retention_days);
        let checkpoints = self.conn.execute(
            "DELETE FROM import_checkpoints WHERE touched_at IS NOT NULL AND touched_at < ?",
            [stale_checkpoints_cutoff.to_rfc3339()],
        )?;
        self.conn.execute(
            "DELETE FROM cooldowns WHERE until_at <= ?",
            [Utc::now().to_rfc3339()],
        )?;
        Ok(RetentionReport {
            usage,
            costs,
            snapshots,
            attempts,
            notifications,
            checkpoints,
        })
    }

    pub fn export_json(&self) -> Result<String> {
        let rows = self.safe_export_rows()?;
        Ok(serde_json::to_string_pretty(
            &serde_json::json!({ "schemaVersion": 1, "history": rows }),
        )?)
    }

    pub fn export_csv(&self) -> Result<String> {
        let mut writer = csv::Writer::from_writer(vec![]);
        for row in self.safe_export_rows()? {
            writer.serialize(row)?;
        }
        Ok(String::from_utf8(writer.into_inner()?)?)
    }

    fn safe_export_rows(&self) -> Result<Vec<SafeExportRow>> {
        let mut statement = self.conn.prepare("SELECT usage_records.product_id,COALESCE(NULLIF(accounts.alias,''),'Аккаунт не определён'),usage_records.period_start,usage_records.period_end,usage_records.model,usage_records.input_tokens,usage_records.output_tokens,usage_records.total_tokens,usage_records.requests,usage_records.source,usage_records.coverage FROM usage_records LEFT JOIN accounts ON accounts.id=usage_records.account_id ORDER BY usage_records.period_start")?;
        let mut rows = statement
            .query_map([], |r| {
                Ok(SafeExportRow {
                    provider_product: r.get(0)?,
                    account_alias: r.get(1)?,
                    period_start: r.get(2)?,
                    period_end: r.get(3)?,
                    model: r.get(4)?,
                    input_tokens: r.get(5)?,
                    output_tokens: r.get(6)?,
                    total_tokens: r.get(7)?,
                    requests: r.get(8)?,
                    amount_decimal: None,
                    currency: None,
                    cost_kind: None,
                    source_type: r.get(9)?,
                    coverage: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut costs=self.conn.prepare("SELECT cost_records.product_id,COALESCE(NULLIF(accounts.alias,''),'Аккаунт не определён'),cost_records.period_start,cost_records.period_end,cost_records.amount_decimal,cost_records.currency,cost_records.kind,cost_records.source,cost_records.coverage FROM cost_records LEFT JOIN accounts ON accounts.id=cost_records.account_id ORDER BY cost_records.period_start")?;
        rows.extend(
            costs
                .query_map([], |r| {
                    Ok(SafeExportRow {
                        provider_product: r.get(0)?,
                        account_alias: r.get(1)?,
                        period_start: r.get(2)?,
                        period_end: r.get(3)?,
                        model: None,
                        input_tokens: None,
                        output_tokens: None,
                        total_tokens: None,
                        requests: None,
                        amount_decimal: r.get(4)?,
                        currency: r.get(5)?,
                        cost_kind: r.get(6)?,
                        source_type: r.get(7)?,
                        coverage: r.get(8)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
        rows.sort_by(|a, b| a.period_start.cmp(&b.period_start));
        Ok(rows)
    }

    pub fn prune_before(&mut self, before: DateTime<Utc>) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM usage_records WHERE period_end < ?",
            [before.to_rfc3339()],
        )?)
    }
    pub fn usage_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM usage_records", [], |r| r.get(0))?)
    }
    pub fn set_cache(&mut self, key: &str, payload_json: &str) -> Result<()> {
        self.conn.execute("INSERT INTO app_cache(cache_key,payload_json,updated_at) VALUES(?,?,?) ON CONFLICT(cache_key) DO UPDATE SET payload_json=excluded.payload_json,updated_at=excluded.updated_at",params![key,payload_json,Utc::now().to_rfc3339()])?;
        Ok(())
    }
    pub fn cache(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT payload_json FROM app_cache WHERE cache_key=?",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn notification_delivered(
        &self,
        account_id: &str,
        metric: &str,
        threshold: u8,
        window_id: &str,
    ) -> Result<bool> {
        Ok(self.conn.query_row("SELECT 1 FROM notification_deliveries WHERE account_id=? AND metric=? AND threshold=? AND window_id=?",params![account_id,metric,threshold,window_id],|r|r.get::<_,u8>(0)).optional()?.is_some())
    }
    pub fn mark_notification_delivered(
        &mut self,
        account_id: &str,
        metric: &str,
        threshold: u8,
        window_id: &str,
    ) -> Result<bool> {
        Ok(self.conn.execute("INSERT OR IGNORE INTO notification_deliveries(account_id,metric,threshold,window_id,delivered_at) VALUES(?,?,?,?,?)",params![account_id,metric,threshold,window_id,Utc::now().to_rfc3339()])?>0)
    }
    pub fn token_totals_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(String, u64)>> {
        let mut statement=self.conn.prepare("SELECT product_id,COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records WHERE period_start>=? AND period_start<? GROUP BY product_id ORDER BY product_id")?;
        let rows = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn verified_token_totals_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(String, u64)>> {
        let mut statement = self.conn.prepare(
            "SELECT product_id,COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records WHERE period_start>=? AND period_start<? AND coverage != 'UnverifiedSemantics' GROUP BY product_id ORDER BY product_id",
        )?;
        let rows = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn unverified_token_totals_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(String, u64)>> {
        let mut statement = self.conn.prepare(
            "SELECT product_id,COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records WHERE period_start>=? AND period_start<? AND coverage = 'UnverifiedSemantics' GROUP BY product_id ORDER BY product_id",
        )?;
        let rows = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn excluded_unverified_count_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<u64> {
        let mut statement = self.conn.prepare(
            "SELECT COUNT(DISTINCT product_id) FROM usage_records WHERE period_start>=? AND period_start<? AND coverage = 'UnverifiedSemantics'",
        )?;
        let count: u64 =
            statement.query_row(params![start.to_rfc3339(), end.to_rfc3339()], |r| r.get(0))?;
        Ok(count)
    }

    pub fn tokens_for_account_between(
        &self,
        account_id: uuid::Uuid,
        product_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<u64> {
        Ok(self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records WHERE account_id=? AND product_id=? AND period_start>=? AND period_start<?",
                params![
                    account_id.to_string(),
                    product_id,
                    start.to_rfc3339(),
                    end.to_rfc3339()
                ],
                |r| r.get(0),
            )?)
    }

    pub fn cost_records_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<CostRecord>> {
        let mut statement = self.conn.prepare("SELECT account_id,product_id,billing_scope_id,source_record_id,period_start,period_end,amount_decimal,currency,kind,source,coverage FROM cost_records WHERE period_start>=? AND period_start<?")?;
        let rows = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                let account_raw: String = r.get(0)?;
                let amount_raw: String = r.get(6)?;
                let kind_raw: String = r.get(8)?;
                let coverage_raw: String = r.get(10)?;
                Ok(CostRecord {
                    account_id: account_raw.parse().unwrap_or(uuid::Uuid::nil()),
                    product_id: r.get(1)?,
                    billing_scope_id: r.get(2)?,
                    source_record_id: r.get(3)?,
                    period_start: r.get::<_, String>(4)?.parse().unwrap_or(Utc::now()),
                    period_end: r.get::<_, String>(5)?.parse().unwrap_or(Utc::now()),
                    amount_decimal: amount_raw.parse().unwrap_or(rust_decimal::Decimal::ZERO),
                    currency: r.get(7)?,
                    kind: match kind_raw.as_str() {
                        "Estimated" => usage_core::CostKind::Estimated,
                        _ => usage_core::CostKind::Reported,
                    },
                    source: r.get(9)?,
                    coverage: match coverage_raw.as_str() {
                        "Partial" => usage_core::Coverage::Partial,
                        "LocalClientOnly" => usage_core::Coverage::LocalClientOnly,
                        "FromConnectionTime" => usage_core::Coverage::FromConnectionTime,
                        "ProviderDelayed" => usage_core::Coverage::ProviderDelayed,
                        _ if coverage_raw.contains("Complete") => usage_core::Coverage::Complete,
                        _ => usage_core::Coverage::Unknown,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn model_totals_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(Option<String>, u64)>> {
        let mut statement = self.conn.prepare("SELECT model,COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records WHERE period_start>=? AND period_start<? GROUP BY model ORDER BY model")?;
        let rows = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn verified_model_totals_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(Option<String>, u64)>> {
        let mut statement = self.conn.prepare("SELECT model,COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records WHERE period_start>=? AND period_start<? AND coverage != 'UnverifiedSemantics' GROUP BY model ORDER BY model")?;
        let rows = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let mut statement = self.conn.prepare(
            "SELECT id,provider_id,external_identity_fingerprint,alias,connection_ref,lifecycle FROM accounts ORDER BY alias",
        )?;
        let rows = statement
            .query_map([], |r| {
                let id_raw: String = r.get(0)?;
                let lifecycle_raw: String = r.get(5)?;
                Ok(Account {
                    id: id_raw.parse().unwrap_or(uuid::Uuid::nil()),
                    provider_id: r.get(1)?,
                    external_identity: r.get(2)?,
                    label: r.get(3)?,
                    connection_ref: r.get(4)?,
                    lifecycle: match lifecycle_raw.as_str() {
                        "Disconnected" => usage_core::AccountLifecycle::Disconnected,
                        "Archived" => usage_core::AccountLifecycle::Archived,
                        _ => usage_core::AccountLifecycle::Active,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_account(&self, account_id: uuid::Uuid) -> Result<Option<ManagedAccount>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id,provider_id,product_id,external_identity_fingerprint,alias,connection_ref,lifecycle,enabled,custom_path,identity_confidence,notifications_muted FROM accounts WHERE id=?",
                [account_id.to_string()],
                |r| {
                    let id_raw: String = r.get(0)?;
                    let lifecycle_raw: String = r.get(6)?;
                    let enabled_int: i64 = r.get(7)?;
                    Ok(ManagedAccount {
                        id: id_raw.parse().unwrap_or(uuid::Uuid::nil()),
                        provider_id: r.get(1)?,
                        product_id: r.get(2)?,
                        external_identity: r.get(3)?,
                        label: r.get(4)?,
                        connection_ref: r.get(5)?,
                        lifecycle: match lifecycle_raw.as_str() {
                            "Disconnected" => usage_core::AccountLifecycle::Disconnected,
                            "Archived" => usage_core::AccountLifecycle::Archived,
                            _ => usage_core::AccountLifecycle::Active,
                        },
                        enabled: enabled_int != 0,
                        custom_path: r.get(8)?,
                        identity_confidence: parse_confidence(r.get(9)?),
                        notifications_muted: r.get::<_, i64>(10).unwrap_or(0) != 0,
                    })
                },
            )
            .optional()?)
    }

    pub fn list_managed_accounts(&self) -> Result<Vec<ManagedAccount>> {
        let mut statement = self.conn.prepare(
            "SELECT id,provider_id,product_id,external_identity_fingerprint,alias,connection_ref,lifecycle,enabled,custom_path,identity_confidence,notifications_muted FROM accounts ORDER BY alias",
        )?;
        let rows = statement
            .query_map([], |r| {
                let id_raw: String = r.get(0)?;
                let lifecycle_raw: String = r.get(6)?;
                let enabled_int: i64 = r.get(7)?;
                Ok(ManagedAccount {
                    id: id_raw.parse().unwrap_or(uuid::Uuid::nil()),
                    provider_id: r.get(1)?,
                    product_id: r.get(2)?,
                    external_identity: r.get(3)?,
                    label: r.get(4)?,
                    connection_ref: r.get(5)?,
                    lifecycle: match lifecycle_raw.as_str() {
                        "Disconnected" => usage_core::AccountLifecycle::Disconnected,
                        "Archived" => usage_core::AccountLifecycle::Archived,
                        _ => usage_core::AccountLifecycle::Active,
                    },
                    enabled: enabled_int != 0,
                    custom_path: r.get(8)?,
                    identity_confidence: parse_confidence(r.get(9)?),
                    notifications_muted: r.get::<_, i64>(10).unwrap_or(0) != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn create_managed_account(&mut self, account: &ManagedAccount) -> Result<()> {
        self.conn.execute(
            "INSERT INTO accounts(id,provider_id,product_id,external_identity_fingerprint,alias,connection_ref,lifecycle,enabled,custom_path,identity_confidence,notifications_muted,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET provider_id=excluded.provider_id,product_id=excluded.product_id,external_identity_fingerprint=excluded.external_identity_fingerprint,alias=excluded.alias,connection_ref=excluded.connection_ref,lifecycle=excluded.lifecycle,enabled=excluded.enabled,custom_path=excluded.custom_path,identity_confidence=excluded.identity_confidence,notifications_muted=excluded.notifications_muted",
            params![
                account.id.to_string(),
                account.provider_id,
                account.product_id,
                account.external_identity,
                account.label,
                account.connection_ref,
                format!("{:?}", account.lifecycle),
                i64::from(account.enabled),
                account.custom_path,
                format!("{:?}", account.identity_confidence),
                i64::from(account.notifications_muted),
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// Per-profile notification mute (V13-06): silences alerts for one
    /// account without touching its refresh lifecycle.
    pub fn set_account_muted(&mut self, account_id: uuid::Uuid, muted: bool) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE accounts SET notifications_muted=? WHERE id=?",
            params![i64::from(muted), account_id.to_string()],
        )? > 0)
    }

    /// First-seen identity persistence and explicit re-bind only.
    /// Nothing here merges histories: a mismatch is resolved by the
    /// runtime router, never by rewriting another account's rows.
    pub fn set_account_identity(
        &mut self,
        account_id: uuid::Uuid,
        fingerprint: &str,
        confidence: usage_core::IdentityConfidence,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE accounts SET external_identity_fingerprint=?,identity_confidence=? WHERE id=?",
            params![
                fingerprint,
                format!("{confidence:?}"),
                account_id.to_string()
            ],
        )? > 0)
    }

    pub fn update_managed_account(
        &mut self,
        account_id: uuid::Uuid,
        alias: Option<&str>,
        enabled: Option<bool>,
        custom_path: Option<Option<&str>>,
    ) -> Result<bool> {
        let mut changed = false;
        if let Some(alias) = alias {
            changed |= self.conn.execute(
                "UPDATE accounts SET alias=? WHERE id=?",
                params![alias, account_id.to_string()],
            )? > 0;
        }
        if let Some(enabled) = enabled {
            changed |= self.conn.execute(
                "UPDATE accounts SET lifecycle=CASE WHEN ?1=1 AND lifecycle='Disconnected' THEN 'Active' WHEN ?1=0 THEN 'Disconnected' ELSE lifecycle END, enabled=?1 WHERE id=?2",
                params![i64::from(enabled), account_id.to_string()],
            )? > 0;
        }
        if let Some(path) = custom_path {
            changed |= self.conn.execute(
                "UPDATE accounts SET custom_path=? WHERE id=?",
                params![path, account_id.to_string()],
            )? > 0;
        }
        Ok(changed)
    }

    pub fn upsert_budget(&mut self, budget: &Budget) -> Result<()> {
        budget.validate()?;
        self.conn.execute(
            "INSERT INTO budgets(account_id,product_id,currency,amount_decimal,period,created_at) VALUES(?,?,?,?,?,?) ON CONFLICT(account_id,product_id,currency) DO UPDATE SET amount_decimal=excluded.amount_decimal,period=excluded.period",
            params![
                budget.account_id.to_string(),
                budget.product_id,
                budget.currency,
                budget.amount_decimal.to_string(),
                budget.period,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn delete_budget(
        &mut self,
        account_id: uuid::Uuid,
        product_id: &str,
        currency: &str,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM budgets WHERE account_id=? AND product_id=? AND currency=?",
            params![account_id.to_string(), product_id, currency],
        )? > 0)
    }

    pub fn list_budgets(&self) -> Result<Vec<Budget>> {
        let mut statement = self.conn.prepare(
            "SELECT account_id,product_id,currency,amount_decimal,period FROM budgets ORDER BY product_id,currency",
        )?;
        let rows = statement
            .query_map([], |r| {
                let account_raw: String = r.get(0)?;
                let amount_raw: String = r.get(3)?;
                Ok(Budget {
                    account_id: account_raw.parse().unwrap_or(uuid::Uuid::nil()),
                    product_id: r.get(1)?,
                    currency: r.get(2)?,
                    amount_decimal: amount_raw.parse().unwrap_or(rust_decimal::Decimal::ZERO),
                    period: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn upsert_balance(
        &mut self,
        account_id: uuid::Uuid,
        product_id: &str,
        currency: &str,
        amount: rust_decimal::Decimal,
        observed_at: DateTime<Utc>,
        source: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO balances(account_id,product_id,currency,amount_decimal,observed_at,source) VALUES(?,?,?,?,?,?) ON CONFLICT(account_id,product_id,currency) DO UPDATE SET amount_decimal=excluded.amount_decimal,observed_at=excluded.observed_at,source=excluded.source",
            params![
                account_id.to_string(),
                product_id,
                currency,
                amount.to_string(),
                observed_at.to_rfc3339(),
                source
            ],
        )?;
        Ok(())
    }

    pub fn balances_for(
        &self,
        account_id: uuid::Uuid,
        product_id: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT currency,amount_decimal FROM balances WHERE account_id=? AND product_id=? ORDER BY currency",
        )?;
        let rows = statement
            .query_map(params![account_id.to_string(), product_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn account_product_totals_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(String, String, String, u64)>> {
        let mut statement = self.conn.prepare("SELECT accounts.alias,usage_records.account_id,usage_records.product_id,COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records LEFT JOIN accounts ON accounts.id=usage_records.account_id WHERE period_start>=? AND period_start<? GROUP BY usage_records.account_id,usage_records.product_id ORDER BY accounts.alias")?;
        let mut rows: Vec<(String, String, String, u64)> = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for row in &mut rows {
            if row.0.trim().is_empty() {
                row.0 = "Аккаунт не определён".into();
            }
        }
        Ok(rows)
    }

    pub fn verified_account_product_totals_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(String, String, String, u64)>> {
        let mut statement = self.conn.prepare("SELECT accounts.alias,usage_records.account_id,usage_records.product_id,COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records LEFT JOIN accounts ON accounts.id=usage_records.account_id WHERE period_start>=? AND period_start<? AND usage_records.coverage != 'UnverifiedSemantics' GROUP BY usage_records.account_id,usage_records.product_id ORDER BY accounts.alias")?;
        let mut rows: Vec<(String, String, String, u64)> = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for row in &mut rows {
            if row.0.trim().is_empty() {
                row.0 = "Аккаунт не определён".into();
            }
        }
        Ok(rows)
    }

    pub fn project_totals_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(Option<String>, u64)>> {
        let mut statement = self.conn.prepare("SELECT billing_scope_id,COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records WHERE period_start>=? AND period_start<? GROUP BY billing_scope_id ORDER BY billing_scope_id")?;
        let rows = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn verified_project_totals_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(Option<String>, u64)>> {
        let mut statement = self.conn.prepare("SELECT billing_scope_id,COALESCE(SUM(COALESCE(total_tokens,COALESCE(input_tokens,0)+COALESCE(output_tokens,0))),0) FROM usage_records WHERE period_start>=? AND period_start<? AND coverage != 'UnverifiedSemantics' GROUP BY billing_scope_id ORDER BY billing_scope_id")?;
        let rows = statement
            .query_map(params![start.to_rfc3339(), end.to_rfc3339()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn record_attribution(&self) -> Result<Vec<RecordAttribution>> {
        let mut statement = self.conn.prepare(
            "SELECT account_id,product_id,connection_id,file_id,file_generation,source_record_id FROM usage_records ORDER BY period_start",
        )?;
        let rows = statement
            .query_map([], |r| {
                Ok(RecordAttribution {
                    account_id: r.get(0)?,
                    product_id: r.get(1)?,
                    connection_id: r.get(2)?,
                    file_id: r.get(3)?,
                    file_generation: r.get(4)?,
                    source_record_id: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn storage_status(&self) -> Result<StorageStatus> {
        let usage: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM usage_records", [], |r| r.get(0))?;
        let costs: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM cost_records", [], |r| r.get(0))?;
        let accounts: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
        let checkpoints: u64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM import_checkpoints", [], |r| r.get(0))?;
        let last_import: Option<String> = self
            .conn
            .query_row(
                "SELECT datetime(MAX(mtime_ms)/1000,'unixepoch') FROM import_checkpoints",
                [],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let db_bytes = self
            .path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len());
        Ok(StorageStatus {
            usage_records: usage,
            cost_records: costs,
            accounts,
            checkpoints,
            last_import,
            db_bytes,
        })
    }
}

/// Legacy rows have no trustworthy source provenance. Keep them available,
/// but move them into an explicit unknown-identity account and connection so
/// they cannot be mistaken for a newly confirmed local account.
///
/// CRITICAL DATA INTEGRITY INVARIANT:
/// Trustworthy records (such as OpenAI API cost records or snapshots with
/// confirmed account attribution) must NEVER have their account changed.
/// Only rows with genuinely missing or ambiguous provenance (e.g. nil UUID,
/// orphaned records, or local client records with missing connection_id)
/// are migrated.
fn migrate_legacy_rows(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    let products: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT DISTINCT product_id FROM usage_records WHERE connection_id IS NULL
             UNION
             SELECT DISTINCT product_id FROM cost_records WHERE account_id = '00000000-0000-0000-0000-000000000000' OR account_id NOT IN (SELECT id FROM accounts)
             UNION
             SELECT DISTINCT product_id FROM snapshots WHERE account_id = '00000000-0000-0000-0000-000000000000' OR account_id NOT IN (SELECT id FROM accounts)",
        )?;
        let products = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        products
    };

    for product_id in products {
        let provider_id = match product_id.as_str() {
            "codex" | "openai-api" => "openai",
            "claude-code" | "claude-ai" => "anthropic",
            "antigravity" | "gemini-api" => "google",
            "glm-coding" => "zai",
            "zen" => "opencode",
            _ => "legacy",
        };
        let root_hash = format!("legacy-unattributed/{product_id}");
        let account_id = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("usage.ai/legacy-account/{provider_id}/{product_id}").as_bytes(),
        );
        let connection_id = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("usage.ai/connection/{provider_id}/{product_id}/{root_hash}").as_bytes(),
        );
        tx.execute(
            "INSERT OR IGNORE INTO accounts(id,provider_id,product_id,external_identity_fingerprint,alias,connection_ref,lifecycle,enabled,custom_path,identity_confidence,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            params![
                account_id.to_string(),
                provider_id,
                product_id,
                Option::<String>::None,
                "Legacy / Identity unknown",
                Option::<String>::None,
                "Active",
                1i64,
                Option::<String>::None,
                "Unknown",
                Utc::now().to_rfc3339(),
            ],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO source_connections(id,account_id,provider_id,product_id,root_hash,root_hint,kind,identity_fingerprint,identity_confidence,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)",
            params![
                connection_id.to_string(),
                account_id.to_string(),
                provider_id,
                product_id,
                root_hash,
                "Legacy / Identity unknown",
                "legacy-unattributed",
                Option::<String>::None,
                "Unknown",
                Utc::now().to_rfc3339(),
            ],
        )?;
        // Only update usage_records that lack connection_id AND lack trustworthy account attribution
        tx.execute(
            "UPDATE usage_records SET account_id=?,connection_id=? WHERE product_id=? AND connection_id IS NULL AND (
                account_id = '00000000-0000-0000-0000-000000000000'
                OR account_id NOT IN (SELECT id FROM accounts)
                OR account_id IN (
                    SELECT id FROM accounts
                    WHERE external_identity_fingerprint IS NULL
                      AND (connection_ref IS NULL OR connection_ref LIKE '%-local')
                      AND identity_confidence = 'Unknown'
                )
            )",
            params![account_id.to_string(), connection_id.to_string(), product_id],
        )?;

        // If there are any usage_records for this product that belong to a valid managed account but have connection_id IS NULL,
        // attach a dedicated connection for that account without touching its account_id.
        {
            let mut managed_stmt = tx.prepare(
                "SELECT DISTINCT account_id FROM usage_records WHERE product_id=? AND connection_id IS NULL"
            )?;
            let managed_accounts = managed_stmt
                .query_map(params![product_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            for m_acc in managed_accounts {
                let m_conn_hash = format!("account-connection/{m_acc}/{product_id}");
                let m_conn_id = uuid::Uuid::new_v5(
                    &uuid::Uuid::NAMESPACE_URL,
                    format!("usage.ai/connection/{provider_id}/{product_id}/{m_conn_hash}")
                        .as_bytes(),
                );
                let account_identity: Option<(Option<String>, String)> = tx
                    .query_row(
                        "SELECT external_identity_fingerprint, identity_confidence FROM accounts WHERE id = ?",
                        params![m_acc],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;

                let (fp, confidence) = match account_identity {
                    Some((Some(ref fp_val), ref conf))
                        if !fp_val.trim().is_empty() && conf == "Verified" =>
                    {
                        (Some(fp_val.clone()), "Verified")
                    }
                    Some((Some(ref fp_val), _)) if !fp_val.trim().is_empty() => {
                        (Some(fp_val.clone()), "Weak")
                    }
                    _ => (None, "Unknown"),
                };

                tx.execute(
                    "INSERT OR IGNORE INTO source_connections(id,account_id,provider_id,product_id,root_hash,root_hint,kind,identity_fingerprint,identity_confidence,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)",
                    params![
                        m_conn_id.to_string(),
                        m_acc,
                        provider_id,
                        product_id,
                        m_conn_hash,
                        "Managed connection",
                        "managed",
                        fp,
                        confidence,
                        Utc::now().to_rfc3339(),
                    ],
                )?;
                tx.execute(
                    "UPDATE usage_records SET connection_id=? WHERE product_id=? AND account_id=? AND connection_id IS NULL",
                    params![m_conn_id.to_string(), product_id, m_acc],
                )?;
            }
        }

        // Only update cost_records that lack trustworthy account attribution
        tx.execute(
            "UPDATE cost_records SET account_id=? WHERE product_id=? AND (
                account_id = '00000000-0000-0000-0000-000000000000'
                OR account_id NOT IN (SELECT id FROM accounts)
                OR account_id IN (
                    SELECT id FROM accounts
                    WHERE external_identity_fingerprint IS NULL
                      AND (connection_ref IS NULL OR connection_ref LIKE '%-local')
                      AND identity_confidence = 'Unknown'
                )
            )",
            params![account_id.to_string(), product_id],
        )?;

        // Only update snapshots that lack trustworthy account attribution
        let mut snap_stmt = tx.prepare(
            "SELECT id, payload_json FROM snapshots WHERE product_id=? AND (
                account_id = '00000000-0000-0000-0000-000000000000'
                OR account_id NOT IN (SELECT id FROM accounts)
                OR account_id IN (
                    SELECT id FROM accounts
                    WHERE external_identity_fingerprint IS NULL
                      AND (connection_ref IS NULL OR connection_ref LIKE '%-local')
                      AND identity_confidence = 'Unknown'
                )
            )",
        )?;
        let snap_rows = snap_stmt
            .query_map(params![product_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<(i64, String)>>>()?;

        for (snap_id, payload_json) in snap_rows {
            let updated_payload =
                if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&payload_json) {
                    if let Some(map) = val.as_object_mut() {
                        map.insert(
                            "account_id".to_string(),
                            serde_json::Value::String(account_id.to_string()),
                        );
                        map.insert(
                            "accountId".to_string(),
                            serde_json::Value::String(account_id.to_string()),
                        );
                    }
                    val.to_string()
                } else {
                    payload_json
                };
            tx.execute(
                "UPDATE snapshots SET account_id=?, payload_json=? WHERE id=?",
                params![account_id.to_string(), updated_payload, snap_id],
            )?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAccount {
    pub id: uuid::Uuid,
    pub provider_id: String,
    pub product_id: Option<String>,
    pub external_identity: Option<String>,
    pub label: String,
    pub connection_ref: Option<String>,
    pub lifecycle: usage_core::AccountLifecycle,
    pub enabled: bool,
    pub custom_path: Option<String>,
    pub identity_confidence: usage_core::IdentityConfidence,
    pub notifications_muted: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub account_id: uuid::Uuid,
    pub product_id: String,
    pub currency: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub amount_decimal: rust_decimal::Decimal,
    pub period: String,
}

impl Budget {
    pub fn validate(&self) -> Result<()> {
        if self.amount_decimal <= rust_decimal::Decimal::ZERO {
            anyhow::bail!("budget amount must be positive");
        }
        if self.currency.trim().is_empty() || self.currency.len() > 8 {
            anyhow::bail!("invalid budget currency");
        }
        Ok(())
    }
}

fn parse_confidence(raw: String) -> usage_core::IdentityConfidence {
    match raw.as_str() {
        "Verified" => usage_core::IdentityConfidence::Verified,
        "Weak" => usage_core::IdentityConfidence::Weak,
        _ => usage_core::IdentityConfidence::Unknown,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStatus {
    pub usage_records: u64,
    pub cost_records: u64,
    pub accounts: u64,
    pub checkpoints: u64,
    pub last_import: Option<String>,
    pub db_bytes: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordAttribution {
    pub account_id: String,
    pub product_id: String,
    pub connection_id: Option<String>,
    pub file_id: Option<String>,
    pub file_generation: Option<i64>,
    pub source_record_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSnapshot {
    pub account_id: String,
    pub product_id: String,
    pub payload_json: String,
    pub observed_at: Option<String>,
    pub fetched_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAttempt {
    pub account_id: String,
    pub product_id: String,
    pub source: String,
    pub status: String,
    pub safe_code: Option<String>,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Debug, Clone)]
pub struct RefreshBundle<'a> {
    pub account_id: uuid::Uuid,
    pub product_id: &'a str,
    pub snapshot_payload_json: &'a str,
    pub observed_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
    pub usage: &'a [UsageRecord],
    pub costs: &'a [CostRecord],
}

#[derive(Debug, Clone)]
pub struct AttemptLog<'a> {
    pub account_id: uuid::Uuid,
    pub product_id: &'a str,
    pub source: &'a str,
    pub status: &'a str,
    pub safe_code: Option<&'a str>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleReport {
    pub usage_accepted: usize,
    pub costs_accepted: usize,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionReport {
    pub usage: usize,
    pub costs: usize,
    pub snapshots: usize,
    pub attempts: usize,
    pub notifications: usize,
    pub checkpoints: usize,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SafeExportRow {
    provider_product: String,
    account_alias: String,
    period_start: String,
    period_end: String,
    model: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    requests: Option<u64>,
    amount_decimal: Option<String>,
    currency: Option<String>,
    cost_kind: Option<String>,
    source_type: String,
    coverage: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use usage_core::{CostKind, Coverage};
    use uuid::Uuid;
    fn usage(id: &str, total: u64) -> UsageRecord {
        let now = Utc::now();
        UsageRecord {
            account_id: Uuid::nil(),
            product_id: "codex".into(),
            billing_scope_id: None,
            source_record_id: id.into(),
            period_start: now,
            period_end: now,
            model: None,
            input_tokens: None,
            output_tokens: None,
            cached_tokens: None,
            cache_creation_tokens: None,
            cache_read_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(total),
            requests: Some(1),
            source: "local".into(),
            coverage: Coverage::LocalClientOnly,
            connection_id: None,
            file_id: None,
            file_generation: None,
        }
    }
    #[test]
    fn repeated_import_is_idempotent() {
        let mut s = Storage::in_memory().unwrap();
        s.upsert_usage(&usage("same", 10)).unwrap();
        s.upsert_usage(&usage("same", 20)).unwrap();
        assert_eq!(s.usage_count().unwrap(), 1);
        assert!(s.export_json().unwrap().contains("20"));
    }

    #[test]
    fn late_claude_chunk_preserves_monotonic_totals_and_cache_buckets() {
        let mut s = Storage::in_memory().unwrap();
        let mut newer = usage("msg:m1:r1", 70);
        newer.product_id = "claude-code".into();
        newer.source = "claude-code-local-session-v1".into();
        newer.model = Some("claude-test".into());
        newer.input_tokens = Some(30);
        newer.output_tokens = Some(40);
        newer.cached_tokens = Some(5);
        newer.cache_read_tokens = Some(5);
        s.upsert_usage(&newer).unwrap();

        let mut older = newer.clone();
        older.period_start -= chrono::Duration::seconds(1);
        older.period_end = older.period_start;
        older.input_tokens = Some(10);
        older.output_tokens = Some(20);
        older.total_tokens = Some(30);
        older.cached_tokens = None;
        older.cache_read_tokens = None;
        s.upsert_usage(&older).unwrap();

        let row: (u64, Option<u64>, Option<u64>, Option<u64>) = s
            .conn
            .query_row(
                "SELECT total_tokens,cache_creation_tokens,cache_read_tokens,cached_tokens FROM usage_records WHERE source_record_id=?",
                ["msg:m1:r1"],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, (70, None, Some(5), Some(5)));
    }

    #[test]
    fn truncation_resets_offset() {
        let mut s = Storage::in_memory().unwrap();
        s.set_checkpoint(&ImportCheckpoint {
            path_hash: "x".into(),
            mtime_ms: 2,
            size: 100,
            byte_offset: 90,
            schema_version: 1,
            last_source_record_id: None,
            device: 7,
            inode: 9,
        })
        .unwrap();
        assert_eq!(s.reset_checkpoint_on_rotation("x", 10, 3).unwrap(), 0);
    }
    #[test]
    fn replacement_by_inode_resets_even_when_size_grows() {
        let mut s = Storage::in_memory().unwrap();
        s.set_checkpoint(&ImportCheckpoint {
            path_hash: "y".into(),
            mtime_ms: 2,
            size: 100,
            byte_offset: 100,
            schema_version: 1,
            last_source_record_id: None,
            device: 7,
            inode: 9,
        })
        .unwrap();
        // Same path, bigger size, but different file identity: replacement.
        assert_eq!(s.offset_for_observation("y", 200, 3, 7, 10).unwrap(), 0);
        // Same identity, appended: resume.
        assert_eq!(s.offset_for_observation("y", 200, 3, 7, 9).unwrap(), 100);
    }
    #[test]
    fn snapshot_last_known_good_survives_failed_update() {
        let mut s = Storage::in_memory().unwrap();
        s.upsert_account(&Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        })
        .unwrap();
        s.save_snapshot(Uuid::nil(), "codex", r#"{"v":1}"#, None, Utc::now())
            .unwrap();
        // A failed transactional update must leave the good snapshot intact.
        let failed: anyhow::Result<()> = (|| {
            let tx = s
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO snapshots(account_id,product_id,payload_json,observed_at,fetched_at) VALUES(?,?,?,?,?)",
                rusqlite::params!["bad", "codex", "oops", Option::<String>::None, "not-a-time"],
            )?;
            tx.execute("THIS IS NOT SQL", [])?;
            tx.commit()?;
            Ok(())
        })();
        assert!(failed.is_err());
        let lkg = s.last_known_good(Uuid::nil(), "codex").unwrap().unwrap();
        assert_eq!(lkg.payload_json, r#"{"v":1}"#);
    }
    #[test]
    fn batch_import_is_atomic_and_attempts_persist() {
        let mut s = Storage::in_memory().unwrap();
        assert_eq!(
            s.import_usage_batch(&[usage("a", 1), usage("b", 2)])
                .unwrap(),
            2
        );
        assert_eq!(s.usage_count().unwrap(), 2);
        let now = Utc::now();
        s.record_attempt(&AttemptLog {
            account_id: Uuid::nil(),
            product_id: "codex",
            source: "codex-local-jsonl",
            status: "ok",
            safe_code: None,
            started_at: now,
            finished_at: now,
        })
        .unwrap();
        s.record_attempt(&AttemptLog {
            account_id: Uuid::nil(),
            product_id: "codex",
            source: "codex-local-jsonl",
            status: "error",
            safe_code: Some("parse_error"),
            started_at: now,
            finished_at: now,
        })
        .unwrap();
        let attempts = s.recent_attempts(Uuid::nil(), "codex", 5).unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].status, "error");
        s.set_cooldown(Uuid::nil(), "openai-api", now + chrono::Duration::hours(1))
            .unwrap();
        assert_eq!(s.load_cooldowns().unwrap().len(), 1);
        s.clear_cooldown(Uuid::nil(), "openai-api").unwrap();
        assert!(s.load_cooldowns().unwrap().is_empty());
    }
    #[test]
    fn retention_covers_all_categories_but_keeps_active_checkpoints() {
        let mut s = Storage::in_memory().unwrap();
        let old = Utc::now() - chrono::Duration::days(120);
        let mut ancient = usage("old", 5);
        ancient.period_start = old;
        ancient.period_end = old;
        s.upsert_usage(&ancient).unwrap();
        s.upsert_account(&Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        })
        .unwrap();
        s.save_snapshot(Uuid::nil(), "codex", "{}", None, old)
            .unwrap();
        // A superseded-but-newer snapshot is the newest LKG and must
        // survive retention so SWR keeps its fallback.
        s.save_snapshot(
            Uuid::nil(),
            "codex",
            "{}",
            None,
            old - chrono::Duration::days(10),
        )
        .unwrap();
        s.record_attempt(&AttemptLog {
            account_id: Uuid::nil(),
            product_id: "codex",
            source: "x",
            status: "ok",
            safe_code: None,
            started_at: old,
            finished_at: old,
        })
        .unwrap();
        // Untouched checkpoint (touched_at NULL) is active: never pruned.
        s.conn
            .execute(
                "INSERT INTO import_checkpoints(path_hash,mtime_ms,size,byte_offset,schema_version) VALUES('active',1,1,1,1)",
                [],
            )
            .unwrap();
        let report = s
            .prune_retention(Utc::now() - chrono::Duration::days(90))
            .unwrap();
        assert_eq!(report.usage, 1);
        assert_eq!(report.snapshots, 1);
        assert_eq!(report.attempts, 1);
        assert_eq!(report.checkpoints, 0);
    }
    #[test]
    fn migrates_real_v1_database_without_data_loss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("../migrations/001_initial.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/002_accounts_budgets.sql"))
                .unwrap();
            conn.pragma_update(None, "user_version", 2u32).unwrap();
            conn.execute(
                "INSERT INTO accounts(id,provider_id,alias,lifecycle,created_at) VALUES('00000000-0000-0000-0000-000000000000','openai','Личный','Active','2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO usage_records(account_id,product_id,source_record_id,period_start,period_end,total_tokens,requests,source,coverage) VALUES('00000000-0000-0000-0000-000000000000','codex','r1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z',10,1,'local','LocalClientOnly')",
                [],
            )
            .unwrap();
        }
        let storage = Storage::open(&path).unwrap();
        assert_eq!(storage.usage_count().unwrap(), 1);
        assert_eq!(storage.list_managed_accounts().unwrap().len(), 2);
        let migrated = storage.record_attribution().unwrap();
        assert_eq!(migrated.len(), 1);
        assert!(migrated[0].connection_id.is_some());
        let legacy_id = migrated[0].account_id.parse::<Uuid>().unwrap();
        let legacy = storage.get_account(legacy_id).unwrap().unwrap();
        assert_eq!(legacy.label, "Legacy / Identity unknown");
        assert_eq!(
            legacy.identity_confidence,
            usage_core::IdentityConfidence::Unknown
        );
        assert!(storage
            .last_known_good(Uuid::nil(), "codex")
            .unwrap()
            .is_none());
        // Backup of the pre-migration database must exist next to it.
        assert!(path.with_extension("db.pre-migration.bak").exists());
    }
    #[test]
    fn migration_case_a_legacy_local_contamination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("../migrations/001_initial.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/002_accounts_budgets.sql"))
                .unwrap();
            conn.pragma_update(None, "user_version", 2u32).unwrap();
            conn.execute(
                "INSERT INTO accounts(id,provider_id,alias,lifecycle,created_at) VALUES('00000000-0000-0000-0000-000000000000','openai','Local','Active','2026-01-01T00:00:00Z')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO usage_records(account_id,product_id,source_record_id,period_start,period_end,total_tokens,requests,source,coverage) VALUES('00000000-0000-0000-0000-000000000000','codex','codex-1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z',100,1,'local','UnverifiedSemantics')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO usage_records(account_id,product_id,source_record_id,period_start,period_end,total_tokens,requests,source,coverage) VALUES('00000000-0000-0000-0000-000000000000','claude-code','claude-1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z',200,1,'local','UnverifiedSemantics')",
                [],
            ).unwrap();
        }
        let storage = Storage::open(&path).unwrap();
        assert_eq!(storage.usage_count().unwrap(), 2);
        let records = storage.record_attribution().unwrap();
        assert_eq!(records.len(), 2);
        for r in records {
            assert!(r.connection_id.is_some());
            let acc_id = r.account_id.parse::<Uuid>().unwrap();
            let acc = storage.get_account(acc_id).unwrap().unwrap();
            assert_eq!(acc.label, "Legacy / Identity unknown");
            assert_eq!(
                acc.identity_confidence,
                usage_core::IdentityConfidence::Unknown
            );
        }
    }
    #[test]
    fn migration_case_b_trustworthy_openai_api() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let api_acc_id = "11111111-1111-1111-1111-111111111111";
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("../migrations/001_initial.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/002_accounts_budgets.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/003_attempts_cooldowns.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/004_identity_confidence.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/005_provenance.sql"))
                .unwrap();
            conn.pragma_update(None, "user_version", 5u32).unwrap();
            conn.execute(
                "INSERT INTO accounts(id,provider_id,product_id,external_identity_fingerprint,alias,connection_ref,lifecycle,enabled,identity_confidence,created_at) VALUES(?,'openai','openai-api','key-fingerprint-abc','Work API','keychain:openai:work','Active',1,'Verified','2026-01-01T00:00:00Z')",
                [api_acc_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO cost_records(account_id,product_id,billing_scope_id,source_record_id,period_start,period_end,amount_decimal,currency,kind,source,coverage) VALUES(?,'openai-api','org-1','c1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z','19.99','USD','reported','official-api','Complete')",
                [api_acc_id],
            ).unwrap();
            let payload = format!(
                r#"{{"account_id":"{api_acc_id}","provider_id":"openai","product_id":"openai-api","connection_state":"Connected","freshness":{{"kind":"Fresh"}},"coverage":"Complete","fetched_at":"2026-09-01T00:00:00Z","capabilities":[],"quotas":[],"balances":[]}}"#
            );
            conn.execute(
                "INSERT INTO snapshots(account_id,product_id,payload_json,fetched_at) VALUES(?,'openai-api',?,'2026-09-01T00:00:00Z')",
                params![api_acc_id, payload],
            ).unwrap();
        }
        let storage = Storage::open(&path).unwrap();
        let api_uuid = api_acc_id.parse::<Uuid>().unwrap();
        let acc = storage.get_account(api_uuid).unwrap().unwrap();
        assert_eq!(acc.label, "Work API");
        assert_eq!(acc.connection_ref.as_deref(), Some("keychain:openai:work"));
        assert_eq!(
            acc.identity_confidence,
            usage_core::IdentityConfidence::Verified
        );

        let costs = storage
            .cost_records_between(
                Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap(),
            )
            .unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].account_id, api_uuid);
        assert_eq!(costs[0].amount_decimal, Decimal::from_str("19.99").unwrap());

        let lkg = storage.last_known_good(api_uuid, "openai-api").unwrap();
        assert!(lkg.is_some(), "LKG for account A must be preserved");
        assert_eq!(lkg.unwrap().account_id, api_acc_id);
    }
    #[test]
    fn migration_case_c_mixed_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let api_acc_id = "22222222-2222-2222-2222-222222222222";
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("../migrations/001_initial.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/002_accounts_budgets.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/003_attempts_cooldowns.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/004_identity_confidence.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/005_provenance.sql"))
                .unwrap();
            conn.pragma_update(None, "user_version", 5u32).unwrap();

            conn.execute(
                "INSERT INTO accounts(id,provider_id,alias,lifecycle,created_at) VALUES('00000000-0000-0000-0000-000000000000','openai','Local','Active','2026-01-01T00:00:00Z')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO usage_records(account_id,product_id,source_record_id,period_start,period_end,total_tokens,requests,source,coverage) VALUES('00000000-0000-0000-0000-000000000000','codex','codex-1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z',100,1,'local','UnverifiedSemantics')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO usage_records(account_id,product_id,source_record_id,period_start,period_end,total_tokens,requests,source,coverage) VALUES('00000000-0000-0000-0000-000000000000','claude-code','claude-1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z',200,1,'local','UnverifiedSemantics')",
                [],
            ).unwrap();

            conn.execute(
                "INSERT INTO accounts(id,provider_id,product_id,external_identity_fingerprint,alias,connection_ref,lifecycle,enabled,identity_confidence,created_at) VALUES(?,'openai','openai-api','key-fp','Work API','keychain:openai:work','Active',1,'Verified','2026-01-01T00:00:00Z')",
                [api_acc_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO cost_records(account_id,product_id,billing_scope_id,source_record_id,period_start,period_end,amount_decimal,currency,kind,source,coverage) VALUES(?,'openai-api','org-1','c1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z','49.95','USD','reported','official-api','Complete')",
                [api_acc_id],
            ).unwrap();
            let payload = format!(
                r#"{{"account_id":"{api_acc_id}","provider_id":"openai","product_id":"openai-api","connection_state":"Connected","freshness":{{"kind":"Fresh"}},"coverage":"Complete","fetched_at":"2026-09-01T00:00:00Z","capabilities":[],"quotas":[],"balances":[]}}"#
            );
            conn.execute(
                "INSERT INTO snapshots(account_id,product_id,payload_json,fetched_at) VALUES(?,'openai-api',?,'2026-09-01T00:00:00Z')",
                params![api_acc_id, payload],
            ).unwrap();
        }
        let storage = Storage::open(&path).unwrap();

        assert_eq!(storage.usage_count().unwrap(), 2);
        let records = storage.record_attribution().unwrap();
        for r in records {
            let acc = storage
                .get_account(r.account_id.parse().unwrap())
                .unwrap()
                .unwrap();
            assert_eq!(acc.label, "Legacy / Identity unknown");
        }
        let api_uuid = api_acc_id.parse::<Uuid>().unwrap();
        let costs = storage
            .cost_records_between(
                Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap(),
            )
            .unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].account_id, api_uuid);
        assert_eq!(costs[0].amount_decimal, Decimal::from_str("49.95").unwrap());

        let lkg = storage
            .last_known_good(api_uuid, "openai-api")
            .unwrap()
            .unwrap();
        assert_eq!(lkg.account_id, api_acc_id);
    }
    #[test]
    fn migration_case_d_idempotence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let api_acc_id = "33333333-3333-3333-3333-333333333333";
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("../migrations/001_initial.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/002_accounts_budgets.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/003_attempts_cooldowns.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/004_identity_confidence.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/005_provenance.sql"))
                .unwrap();
            conn.pragma_update(None, "user_version", 5u32).unwrap();
            conn.execute(
                "INSERT INTO accounts(id,provider_id,product_id,external_identity_fingerprint,alias,connection_ref,lifecycle,enabled,identity_confidence,created_at) VALUES(?,'openai','openai-api','key-fp','Work API','keychain:openai:work','Active',1,'Verified','2026-01-01T00:00:00Z')",
                [api_acc_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO cost_records(account_id,product_id,billing_scope_id,source_record_id,period_start,period_end,amount_decimal,currency,kind,source,coverage) VALUES(?,'openai-api','org-1','c1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z','10.50','USD','reported','official-api','Complete')",
                [api_acc_id],
            ).unwrap();
        }
        let storage1 = Storage::open(&path).unwrap();
        let counts1 = storage1.storage_status().unwrap();
        drop(storage1);

        let storage2 = Storage::open(&path).unwrap();
        let counts2 = storage2.storage_status().unwrap();
        assert_eq!(counts1.accounts, counts2.accounts);
        assert_eq!(counts1.cost_records, counts2.cost_records);
        assert_eq!(counts1.usage_records, counts2.usage_records);
        assert_eq!(counts1.checkpoints, counts2.checkpoints);

        let api_uuid = api_acc_id.parse::<Uuid>().unwrap();
        let costs = storage2
            .cost_records_between(
                Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap(),
            )
            .unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].account_id, api_uuid);
    }
    #[test]
    fn migration_managed_api_account_records_never_change_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let api_acc_id = "44444444-4444-4444-4444-444444444444";
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("../migrations/001_initial.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/002_accounts_budgets.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/003_attempts_cooldowns.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/004_identity_confidence.sql"))
                .unwrap();
            conn.execute_batch(include_str!("../migrations/005_provenance.sql"))
                .unwrap();
            conn.pragma_update(None, "user_version", 5u32).unwrap();

            conn.execute(
                "INSERT INTO accounts(id,provider_id,product_id,external_identity_fingerprint,alias,connection_ref,lifecycle,enabled,identity_confidence,created_at) VALUES(?,'openai','openai-api','key-fingerprint-prod','Managed Production API','keychain:openai:prod','Active',1,'Verified','2026-01-01T00:00:00Z')",
                [api_acc_id],
            ).unwrap();

            // Insert usage record with connection_id IS NULL (simulating pre-v5 / v5 state before migration 006)
            conn.execute(
                "INSERT INTO usage_records(account_id,product_id,source_record_id,period_start,period_end,total_tokens,requests,source,coverage) VALUES(?,'openai-api','openai-usage-1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z',500,5,'api','Complete')",
                [api_acc_id],
            ).unwrap();

            // Insert cost record
            conn.execute(
                "INSERT INTO cost_records(account_id,product_id,billing_scope_id,source_record_id,period_start,period_end,amount_decimal,currency,kind,source,coverage) VALUES(?,'openai-api','org-prod','c-prod','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z','123.45','USD','reported','official-api','Complete')",
                [api_acc_id],
            ).unwrap();

            // Insert snapshot
            let payload = format!(
                r#"{{"account_id":"{api_acc_id}","provider_id":"openai","product_id":"openai-api","connection_state":"Connected","freshness":{{"kind":"Fresh"}},"coverage":"Complete","fetched_at":"2026-09-01T00:00:00Z","capabilities":[],"quotas":[],"balances":[]}}"#
            );
            conn.execute(
                "INSERT INTO snapshots(account_id,product_id,payload_json,fetched_at) VALUES(?,'openai-api',?,'2026-09-01T00:00:00Z')",
                params![api_acc_id, payload],
            ).unwrap();
        }

        // Open and migrate to v6/v7
        let storage = Storage::open(&path).unwrap();
        let api_uuid = api_acc_id.parse::<Uuid>().unwrap();

        // Check account itself
        let acc = storage
            .get_account(api_uuid)
            .unwrap()
            .expect("account must exist");
        assert_eq!(acc.label, "Managed Production API");
        assert_eq!(
            acc.identity_confidence,
            usage_core::IdentityConfidence::Verified
        );

        // Check usage records: account_id MUST NOT have changed to legacy account
        let usage_records = storage.record_attribution().unwrap();
        let matched_usage = usage_records
            .iter()
            .find(|u| u.source_record_id == "openai-usage-1")
            .expect("usage record must exist");
        assert_eq!(
            matched_usage.account_id, api_acc_id,
            "Usage record account_id must NOT change during migration"
        );
        assert!(
            matched_usage.connection_id.is_some(),
            "Connection ID must have been attached"
        );

        // Check cost records: account_id MUST NOT have changed
        let costs = storage
            .cost_records_between(
                Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap(),
            )
            .unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(
            costs[0].account_id, api_uuid,
            "Cost record account_id must NOT change during migration"
        );
        assert_eq!(
            costs[0].amount_decimal,
            Decimal::from_str("123.45").unwrap()
        );

        // Check snapshot: account_id and LKG MUST remain with the managed account
        let lkg = storage
            .last_known_good(api_uuid, "openai-api")
            .unwrap()
            .expect("LKG must be preserved");
        assert_eq!(
            lkg.account_id, api_acc_id,
            "Snapshot account_id must NOT change during migration"
        );
    }
    #[test]
    fn sqlite_exact_decimal_roundtrip() {
        let mut storage = Storage::in_memory().unwrap();
        let acc_id = Uuid::new_v4();
        let test_values = [
            "0.1",
            "0.2",
            "0.3",
            "0.00000001",
            "123456789.123456789",
            "999999999999.999999",
        ];
        let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        for (i, val_str) in test_values.iter().enumerate() {
            let decimal = Decimal::from_str(val_str).unwrap();
            let record = CostRecord {
                account_id: acc_id,
                product_id: "openai-api".into(),
                billing_scope_id: None,
                source_record_id: format!("rec-{i}"),
                period_start: start,
                period_end: start,
                amount_decimal: decimal,
                currency: "USD".into(),
                kind: CostKind::Reported,
                source: "official-api".into(),
                coverage: Coverage::Complete,
            };
            storage.upsert_cost(&record).unwrap();
        }

        let fetched = storage
            .cost_records_between(
                Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap(),
            )
            .unwrap();
        assert_eq!(fetched.len(), test_values.len());

        for (i, val_str) in test_values.iter().enumerate() {
            let found = fetched
                .iter()
                .find(|r| r.source_record_id == format!("rec-{i}"))
                .unwrap();
            let expected = Decimal::from_str(val_str).unwrap();
            assert_eq!(found.amount_decimal, expected);
            assert_eq!(found.amount_decimal.to_string(), *val_str);
        }

        let d1 = Decimal::from_str("0.1").unwrap();
        let d2 = Decimal::from_str("0.2").unwrap();
        let d3 = Decimal::from_str("0.3").unwrap();
        assert_eq!(d1 + d2, d3);
    }
    #[test]
    fn export_uses_safe_alias_and_no_identity() {
        let mut s = Storage::in_memory().unwrap();
        s.upsert_usage(&usage("one", 10)).unwrap();
        let out = s.export_json().unwrap();
        assert!(out.contains("Аккаунт не определён"));
        assert!(!out.to_ascii_lowercase().contains("email"));
    }
    #[test]
    fn account_archive_preserves_history_and_delete_removes_it() {
        let mut s = Storage::in_memory().unwrap();
        let account = Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: Some("fingerprint".into()),
            label: "Личный".into(),
            connection_ref: Some("keychain:item".into()),
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        s.upsert_account(&account).unwrap();
        s.upsert_usage(&usage("one", 10)).unwrap();
        let connection = s
            .ensure_connection(
                account.id,
                "openai",
                "codex",
                "delete-root",
                "root",
                "default-local",
            )
            .unwrap();
        let file = s
            .ensure_source_file(&connection.id, "delete-file", 1, 2)
            .unwrap();
        let record = provenanced("one", 10, &connection.id, &file.id, 1);
        s.commit_import_batch(&ImportBatchCommit {
            file_id: &file.id,
            file_generation: 1,
            is_replacement: false,
            device: 1,
            inode: 2,
            records: &[record],
            byte_offset: 4,
            schema_version: 1,
            last_source_record_id: Some("one"),
        })
        .unwrap();
        s.archive_account(account.id).unwrap();
        assert_eq!(s.usage_count().unwrap(), 1);
        s.delete_account_and_history(account.id).unwrap();
        assert_eq!(s.usage_count().unwrap(), 0);
        assert!(s.get_connection(&connection.id).unwrap().is_none());
        assert!(s.get_source_file(&file.id).unwrap().is_none());
        assert!(s.file_import_state(&file.id).unwrap().is_none());
    }
    #[test]
    fn budgets_validate_and_roundtrip() {
        use rust_decimal::Decimal;
        let mut s = Storage::in_memory().unwrap();
        let bad = Budget {
            account_id: Uuid::nil(),
            product_id: "openai-api".into(),
            currency: "USD".into(),
            amount_decimal: Decimal::ZERO,
            period: "monthly".into(),
        };
        assert!(s.upsert_budget(&bad).is_err());
        let good = Budget {
            account_id: Uuid::nil(),
            product_id: "openai-api".into(),
            currency: "USD".into(),
            amount_decimal: Decimal::new(20, 0),
            period: "monthly".into(),
        };
        s.upsert_budget(&good).unwrap();
        assert_eq!(s.list_budgets().unwrap().len(), 1);
        assert!(s.delete_budget(Uuid::nil(), "openai-api", "USD").unwrap());
        assert!(s.list_budgets().unwrap().is_empty());
    }
    #[test]
    fn managed_accounts_carry_enabled_and_custom_path() {
        let mut s = Storage::in_memory().unwrap();
        let account = ManagedAccount {
            id: Uuid::new_v4(),
            provider_id: "openai".into(),
            product_id: Some("openai-api".into()),
            external_identity: Some("fp".into()),
            label: "API".into(),
            connection_ref: Some("keychain:item".into()),
            lifecycle: usage_core::AccountLifecycle::Active,
            enabled: true,
            custom_path: None,
            identity_confidence: usage_core::IdentityConfidence::Weak,
            notifications_muted: false,
        };
        s.create_managed_account(&account).unwrap();
        assert!(s.get_account(account.id).unwrap().unwrap().enabled);
        s.update_managed_account(account.id, None, Some(false), Some(Some("/tmp/x")))
            .unwrap();
        let updated = s.get_account(account.id).unwrap().unwrap();
        assert!(!updated.enabled);
        assert_eq!(updated.custom_path.as_deref(), Some("/tmp/x"));
        let status = s.storage_status().unwrap();
        assert_eq!(status.accounts, 1);
    }
    #[test]
    fn tokens_costs_and_models_aggregate_without_fabrication() {
        use chrono::TimeZone;
        let mut s = Storage::in_memory().unwrap();
        let start = Utc.with_ymd_and_hms(2026, 9, 8, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 9, 0, 0, 0).unwrap();
        let mut named = usage("m1", 40);
        named.model = Some("gpt-x".into());
        named.period_start = start + chrono::Duration::hours(1);
        named.period_end = named.period_start;
        s.upsert_usage(&named).unwrap();
        let mut unknown = usage("m2", 10);
        unknown.period_start = start + chrono::Duration::hours(2);
        unknown.period_end = unknown.period_start;
        s.upsert_usage(&unknown).unwrap();
        assert_eq!(
            s.tokens_for_account_between(Uuid::nil(), "codex", start, end)
                .unwrap(),
            50
        );
        let models = s.model_totals_between(start, end).unwrap();
        assert_eq!(models.len(), 2);
        assert!(models
            .iter()
            .any(|(m, v)| m.as_deref() == Some("gpt-x") && *v == 40));
        assert!(models.iter().any(|(m, v)| m.is_none() && *v == 10));
        assert!(s.cost_records_between(start, end).unwrap().is_empty());
        assert_eq!(s.list_accounts().unwrap().len(), 0);
    }
    fn provenanced(
        id: &str,
        total: u64,
        connection: &str,
        file: &str,
        generation: i64,
    ) -> UsageRecord {
        let mut record = usage(id, total);
        record.connection_id = Some(connection.into());
        record.file_id = Some(file.into());
        record.file_generation = Some(generation);
        record
    }
    #[test]
    fn connections_isolate_default_and_custom_roots() {
        let mut s = Storage::in_memory().unwrap();
        s.upsert_account(&Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        })
        .unwrap();
        let default = s
            .ensure_connection(
                Uuid::nil(),
                "openai",
                "codex",
                "roothash-default",
                "default",
                "default-local",
            )
            .unwrap();
        let custom = s
            .ensure_connection(
                Uuid::nil(),
                "openai",
                "codex",
                "roothash-custom",
                "work",
                "custom-local",
            )
            .unwrap();
        assert_ne!(default.id, custom.id);
        // Same triple re-ensured maps to the same stable connection.
        let again = s
            .ensure_connection(
                Uuid::nil(),
                "openai",
                "codex",
                "roothash-default",
                "default",
                "default-local",
            )
            .unwrap();
        assert_eq!(again.id, default.id);
        let file_a = s
            .ensure_source_file(&default.id, "pathhash-a", 7, 9)
            .unwrap();
        let file_b = s
            .ensure_source_file(&custom.id, "pathhash-a", 7, 9)
            .unwrap();
        // Same path hash under different connections: distinct file rows.
        assert_ne!(file_a.id, file_b.id);
    }

    #[test]
    fn same_root_cannot_silently_rebind_between_accounts() {
        let mut s = Storage::in_memory().unwrap();
        let account_a = Account {
            id: Uuid::new_v4(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "A".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        };
        let account_b = Account {
            id: Uuid::new_v4(),
            ..account_a.clone()
        };
        s.upsert_account(&account_a).unwrap();
        s.upsert_account(&account_b).unwrap();
        let original = s
            .ensure_connection(
                account_a.id,
                "openai",
                "codex",
                "same-root",
                "default",
                "default-local",
            )
            .unwrap();
        let rebinding = s.ensure_connection(
            account_b.id,
            "openai",
            "codex",
            "same-root",
            "custom",
            "custom-local",
        );
        assert!(rebinding.is_err());
        assert_eq!(
            s.get_connection(&original.id).unwrap().unwrap().account_id,
            account_a.id.to_string()
        );
    }
    #[test]
    fn replacement_rebuilds_old_generation_rows_atomically() {
        let mut s = Storage::in_memory().unwrap();
        s.upsert_account(&Account {
            id: Uuid::nil(),
            provider_id: "openai".into(),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
        })
        .unwrap();
        let connection = s
            .ensure_connection(
                Uuid::nil(),
                "openai",
                "codex",
                "rh",
                "default",
                "default-local",
            )
            .unwrap();
        let file = s.ensure_source_file(&connection.id, "ph", 7, 9).unwrap();
        let v1 = vec![
            provenanced("r1", 10, &connection.id, &file.id, 1),
            provenanced("r2", 20, &connection.id, &file.id, 1),
        ];
        assert_eq!(
            s.commit_import_batch(&ImportBatchCommit {
                file_id: &file.id,
                file_generation: 1,
                is_replacement: false,
                device: 7,
                inode: 9,
                records: &v1,
                byte_offset: 100,
                schema_version: 1,
                last_source_record_id: Some("r2"),
            })
            .unwrap(),
            2
        );
        assert_eq!(s.usage_count().unwrap(), 2);
        // Replacement with a new device/inode generation: old rows for
        // this file vanish in the same transaction as the new import.
        let v2 = vec![provenanced("r3", 5, &connection.id, &file.id, 2)];
        assert_eq!(
            s.commit_import_batch(&ImportBatchCommit {
                file_id: &file.id,
                file_generation: 2,
                is_replacement: true,
                device: 7,
                inode: 10,
                records: &v2,
                byte_offset: 50,
                schema_version: 1,
                last_source_record_id: Some("r3"),
            })
            .unwrap(),
            1
        );
        assert_eq!(s.usage_count().unwrap(), 1);
        let state = s.file_import_state(&file.id).unwrap().unwrap();
        assert_eq!(state.byte_offset, 50);
        assert_eq!(s.get_source_file(&file.id).unwrap().unwrap().generation, 2);
        assert_eq!(s.get_source_file(&file.id).unwrap().unwrap().inode, 10);
    }
    #[test]
    fn failed_import_batch_leaves_db_unchanged() {
        // Fault injection at the SQL boundary: a transaction that writes
        // rows and checkpoints, then fails, must roll everything back.
        let mut s = Storage::in_memory().unwrap();
        let before = s.usage_count().unwrap();
        let failed: anyhow::Result<()> = (|| {
            let tx = s
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO usage_records(account_id,product_id,source_record_id,period_start,period_end,source,coverage) VALUES(?,?,?,?,?,?,?)",
                rusqlite::params![
                    Uuid::nil().to_string(),
                    "codex",
                    "fault-1",
                    "2026-09-08T00:00:00Z",
                    "2026-09-08T00:00:00Z",
                    "local",
                    "LocalClientOnly"
                ],
            )?;
            tx.execute(
                "INSERT INTO file_imports(file_id,byte_offset) VALUES(?,?)",
                rusqlite::params!["file-1", 64],
            )?;
            tx.execute("THIS IS NOT SQL", [])?;
            tx.commit()?;
            Ok(())
        })();
        assert!(failed.is_err());
        assert_eq!(s.usage_count().unwrap(), before);
        assert!(s.file_import_state("file-1").unwrap().is_none());
    }
    #[cfg(unix)]
    #[test]
    fn database_file_is_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let mut storage = Storage::open(&path).unwrap();
        storage.upsert_usage(&usage("test-perm", 10)).unwrap();
        let parent_mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(parent_mode, 0o700);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(suffix);
            let sidecar_path = std::path::PathBuf::from(sidecar);
            if sidecar_path.exists() {
                let sidecar_mode = std::fs::metadata(&sidecar_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(sidecar_mode, 0o600, "sidecar {suffix} must be 0600");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn wal_and_shm_sidecars_are_hardened_and_fail_closed_if_insecure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let storage = Storage::open(&path).unwrap();
        drop(storage);

        // Manually create both -wal and -shm sidecars with insecure permissions (0o666)
        let wal_path = dir.path().join("usage.db-wal");
        let shm_path = dir.path().join("usage.db-shm");
        std::fs::write(&wal_path, b"dummy wal content").unwrap();
        std::fs::write(&shm_path, b"dummy shm content").unwrap();
        std::fs::set_permissions(&wal_path, std::fs::Permissions::from_mode(0o666)).unwrap();
        std::fs::set_permissions(&shm_path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert_ne!(
            std::fs::metadata(&wal_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_ne!(
            std::fs::metadata(&shm_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // Harden permissions must fix both to 0o600
        let hardener = SystemPermissionHardener;
        hardener.harden_permissions(&path).unwrap();
        let wal_mode = std::fs::metadata(&wal_path).unwrap().permissions().mode() & 0o777;
        let shm_mode = std::fs::metadata(&shm_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            wal_mode, 0o600,
            "insecure wal sidecar must be hardened to 0600"
        );
        assert_eq!(
            shm_mode, 0o600,
            "insecure shm sidecar must be hardened to 0600"
        );
    }

    struct FailingHardener;
    impl PermissionHardener for FailingHardener {
        fn harden_permissions(&self, _db_path: &Path) -> Result<()> {
            anyhow::bail!("forced permission chmod failure")
        }
    }

    #[test]
    fn forced_permission_hardening_failure_aborts_storage_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let res = Storage::open_with_hardener(&path, std::sync::Arc::new(FailingHardener));
        assert!(
            res.is_err(),
            "Storage::open must fail closed on chmod error"
        );
        let err_msg = res.err().unwrap().to_string();
        assert!(err_msg.contains("forced permission chmod failure"));
    }

    struct InsecureModeHardener;
    impl PermissionHardener for InsecureModeHardener {
        fn harden_permissions(&self, _db_path: &Path) -> Result<()> {
            anyhow::bail!("db file has insecure permissions: 666")
        }
    }

    #[test]
    fn bad_resulting_mode_aborts_storage_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let res = Storage::open_with_hardener(&path, std::sync::Arc::new(InsecureModeHardener));
        assert!(
            res.is_err(),
            "Storage::open must fail closed when mode is insecure"
        );
    }

    #[test]
    fn migration_backfilled_unknown_account_connection_remains_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v5.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../migrations/001_initial.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/002_accounts_budgets.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/003_attempts_cooldowns.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/004_identity_confidence.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/005_provenance.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", 5).unwrap();

        let local_acc_id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO accounts(id,provider_id,product_id,external_identity_fingerprint,alias,connection_ref,lifecycle,enabled,identity_confidence,created_at) VALUES(?,'openai','codex',NULL,'Local Codex','local','Active',1,'Unknown','2026-01-01T00:00:00Z')",
            params![local_acc_id.to_string()],
        ).unwrap();

        // Insert usage row with connection_id IS NULL belonging to local account
        conn.execute(
            "INSERT INTO usage_records(account_id,product_id,source_record_id,period_start,period_end,total_tokens,requests,source,coverage) VALUES(?,'codex','r1','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z',50,1,'local','UnverifiedSemantics')",
            params![local_acc_id.to_string()],
        ).unwrap();
        drop(conn);

        let storage = Storage::open(&path).unwrap();
        let conns = storage.list_connections().unwrap();
        let backfilled = conns
            .iter()
            .find(|c| c.account_id == local_acc_id.to_string())
            .unwrap();
        assert_eq!(
            backfilled.identity_confidence, "Unknown",
            "Backfilled local connection without fingerprint must remain Unknown"
        );
    }
}
#[test]
fn candidate_decisions_survive_rescan_and_migration() {
    let mut s = Storage::in_memory().unwrap();
    let first = s
        .upsert_candidate("aabbcc", "openai", "codex", ".codex-work")
        .unwrap();
    assert_eq!(first.status, "pending");
    assert!(s.set_candidate_status("aabbcc", "ignored").unwrap());
    // Rescan refreshes metadata but never overwrites the decision.
    let again = s
        .upsert_candidate("aabbcc", "openai", "codex", ".codex-work-renamed")
        .unwrap();
    assert_eq!(again.status, "ignored");
    assert!(s.list_candidates(true).unwrap().is_empty());
    assert_eq!(s.list_candidates(false).unwrap().len(), 1);
    assert!(s.get_candidate("missing").unwrap().is_none());
}

#[test]
fn migrates_v7_database_with_candidates_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        for migration in [
            "migrations/001_initial.sql",
            "migrations/002_accounts_budgets.sql",
            "migrations/003_attempts_cooldowns.sql",
            "migrations/004_identity_confidence.sql",
            "migrations/005_provenance.sql",
            "migrations/006_legacy_identity.sql",
            "migrations/007_claude_cache_buckets.sql",
        ] {
            let sql = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(migration),
            )
            .unwrap();
            conn.execute_batch(&sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 7u32).unwrap();
    }
    let mut storage = Storage::open(&path).unwrap();
    // v8 table works immediately after the upgrade path.
    storage
        .upsert_candidate("ff00", "anthropic", "claude-code", ".claude-work")
        .unwrap();
    assert_eq!(storage.list_candidates(true).unwrap().len(), 1);
}

#[test]
fn migrates_to_v9_with_account_mute_default_off() {
    let mut storage = Storage::in_memory().unwrap();
    let id = uuid::Uuid::new_v4();
    storage
        .create_managed_account(&ManagedAccount {
            id,
            provider_id: "openai".into(),
            product_id: Some("codex".into()),
            external_identity: None,
            label: "T".into(),
            connection_ref: None,
            lifecycle: usage_core::AccountLifecycle::Active,
            enabled: true,
            custom_path: None,
            identity_confidence: usage_core::IdentityConfidence::Unknown,
            notifications_muted: false,
        })
        .unwrap();
    // Default off, set/get roundtrip, surviving re-read.
    assert!(
        !storage
            .get_account(id)
            .unwrap()
            .unwrap()
            .notifications_muted
    );
    assert!(storage.set_account_muted(id, true).unwrap());
    assert!(
        storage
            .get_account(id)
            .unwrap()
            .unwrap()
            .notifications_muted
    );
    assert!(
        storage
            .list_managed_accounts()
            .unwrap()
            .iter()
            .find(|a| a.id == id)
            .unwrap()
            .notifications_muted
    );
    assert!(storage.set_account_muted(id, false).unwrap());
    assert!(
        !storage
            .get_account(id)
            .unwrap()
            .unwrap()
            .notifications_muted
    );
    // Unknown account: no row touched.
    assert!(!storage
        .set_account_muted(uuid::Uuid::new_v4(), true)
        .unwrap());
}
