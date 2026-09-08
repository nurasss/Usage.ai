CREATE TABLE IF NOT EXISTS source_connections (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  provider_id TEXT NOT NULL,
  product_id TEXT NOT NULL,
  root_hash TEXT NOT NULL,
  root_hint TEXT NOT NULL,
  kind TEXT NOT NULL,
  identity_fingerprint TEXT,
  identity_confidence TEXT NOT NULL DEFAULT 'Unknown',
  created_at TEXT NOT NULL,
  UNIQUE(provider_id, product_id, root_hash)
);
CREATE TABLE IF NOT EXISTS source_files (
  id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL REFERENCES source_connections(id),
  path_hash TEXT NOT NULL,
  device INTEGER NOT NULL DEFAULT 0,
  inode INTEGER NOT NULL DEFAULT 0,
  generation INTEGER NOT NULL DEFAULT 1,
  UNIQUE(connection_id, path_hash)
);
CREATE TABLE IF NOT EXISTS file_imports (
  file_id TEXT PRIMARY KEY REFERENCES source_files(id),
  byte_offset INTEGER NOT NULL DEFAULT 0,
  schema_version INTEGER NOT NULL DEFAULT 1,
  last_source_record_id TEXT,
  touched_at TEXT
);
ALTER TABLE usage_records ADD COLUMN connection_id TEXT;
ALTER TABLE usage_records ADD COLUMN file_id TEXT;
ALTER TABLE usage_records ADD COLUMN file_generation INTEGER NOT NULL DEFAULT 1;
CREATE INDEX IF NOT EXISTS idx_usage_file ON usage_records(file_id, file_generation);
