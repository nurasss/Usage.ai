-- Claude legacy trust reclassification (v10): RC1 persisted synthetic
-- Claude quota as authoritative `Partial`. The rewrite itself lives in
-- Rust (`migrate_claude_trust`) because it reparses JSON payloads with
-- typed rules; this file only versions the step. See Master Spec §2.6.
SELECT 1;
