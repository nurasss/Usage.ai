-- Data policy migration: legacy rows without provenance are moved by the
-- host-language migration into explicit unknown-identity buckets.
CREATE INDEX IF NOT EXISTS idx_usage_connection ON usage_records(connection_id, product_id);
