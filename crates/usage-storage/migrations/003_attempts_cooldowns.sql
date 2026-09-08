CREATE TABLE IF NOT EXISTS source_attempts (
  id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL, product_id TEXT NOT NULL,
  source TEXT NOT NULL, status TEXT NOT NULL, safe_code TEXT,
  started_at TEXT NOT NULL, finished_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_attempts_account ON source_attempts(account_id, product_id, finished_at);
CREATE TABLE IF NOT EXISTS cooldowns (
  account_id TEXT NOT NULL, source TEXT NOT NULL, until_at TEXT NOT NULL,
  PRIMARY KEY(account_id, source)
);
ALTER TABLE import_checkpoints ADD COLUMN device INTEGER NOT NULL DEFAULT 0;
ALTER TABLE import_checkpoints ADD COLUMN inode INTEGER NOT NULL DEFAULT 0;
ALTER TABLE import_checkpoints ADD COLUMN touched_at TEXT;
CREATE INDEX IF NOT EXISTS idx_snapshots_account ON snapshots(account_id, product_id, fetched_at);
