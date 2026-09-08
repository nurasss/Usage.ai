CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, external_identity_fingerprint TEXT,
  alias TEXT NOT NULL, connection_ref TEXT, lifecycle TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL, product_id TEXT NOT NULL,
  payload_json TEXT NOT NULL, observed_at TEXT, fetched_at TEXT NOT NULL,
  FOREIGN KEY(account_id) REFERENCES accounts(id)
);
CREATE TABLE IF NOT EXISTS usage_records (
  id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL, product_id TEXT NOT NULL,
  billing_scope_id TEXT, source_record_id TEXT NOT NULL, period_start TEXT NOT NULL, period_end TEXT NOT NULL,
  model TEXT, input_tokens INTEGER, output_tokens INTEGER, cached_tokens INTEGER, reasoning_tokens INTEGER,
  total_tokens INTEGER, requests INTEGER, source TEXT NOT NULL, coverage TEXT NOT NULL,
  UNIQUE(account_id, product_id, source, source_record_id)
);
CREATE TABLE IF NOT EXISTS cost_records (
  id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL, product_id TEXT NOT NULL,
  billing_scope_id TEXT, source_record_id TEXT NOT NULL, period_start TEXT NOT NULL, period_end TEXT NOT NULL,
  amount_decimal TEXT NOT NULL, currency TEXT NOT NULL, kind TEXT NOT NULL, source TEXT NOT NULL, coverage TEXT NOT NULL,
  UNIQUE(account_id, product_id, source, source_record_id)
);
CREATE TABLE IF NOT EXISTS import_checkpoints (
  path_hash TEXT PRIMARY KEY, mtime_ms INTEGER NOT NULL, size INTEGER NOT NULL, byte_offset INTEGER NOT NULL,
  schema_version INTEGER NOT NULL, last_source_record_id TEXT
);
CREATE TABLE IF NOT EXISTS notification_deliveries (
  account_id TEXT NOT NULL, metric TEXT NOT NULL, threshold INTEGER NOT NULL, window_id TEXT NOT NULL,
  delivered_at TEXT NOT NULL, PRIMARY KEY(account_id, metric, threshold, window_id)
);
CREATE TABLE IF NOT EXISTS app_cache (
  cache_key TEXT PRIMARY KEY, payload_json TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_usage_period ON usage_records(period_start, period_end);
CREATE INDEX IF NOT EXISTS idx_cost_period ON cost_records(period_start, period_end);
