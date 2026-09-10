CREATE TABLE IF NOT EXISTS profile_candidates (
  root_hash TEXT PRIMARY KEY,
  provider_id TEXT NOT NULL,
  product_id TEXT NOT NULL,
  root_hint TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  seen_at TEXT NOT NULL,
  connected_at TEXT
);
