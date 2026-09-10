-- Preserve Claude's source-provided cache creation/read counters separately.
-- `cached_tokens` remains the derived sum for existing aggregate consumers.
ALTER TABLE usage_records ADD COLUMN cache_creation_tokens INTEGER;
ALTER TABLE usage_records ADD COLUMN cache_read_tokens INTEGER;
