ALTER TABLE accounts ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE accounts ADD COLUMN custom_path TEXT;
ALTER TABLE accounts ADD COLUMN product_id TEXT;
CREATE TABLE IF NOT EXISTS budgets (
  id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL, product_id TEXT NOT NULL,
  currency TEXT NOT NULL, amount_decimal TEXT NOT NULL, period TEXT NOT NULL DEFAULT 'monthly',
  created_at TEXT NOT NULL,
  UNIQUE(account_id, product_id, currency)
);
CREATE TABLE IF NOT EXISTS balances (
  account_id TEXT NOT NULL, product_id TEXT NOT NULL, currency TEXT NOT NULL,
  amount_decimal TEXT NOT NULL, observed_at TEXT NOT NULL, source TEXT NOT NULL,
  PRIMARY KEY(account_id, product_id, currency)
);
CREATE INDEX IF NOT EXISTS idx_usage_account_product ON usage_records(account_id, product_id, period_start);
CREATE INDEX IF NOT EXISTS idx_usage_scope ON usage_records(billing_scope_id, period_start);
