# Real redacted evidence corpus (P0-05 / P0-06)

Provenance: controlled local experiments + one self-selected session
per product, redacted with the approved streaming redactors
(`scripts/redact-codex.mjs`, `scripts/redact-claude.mjs`), manually
reviewed by the machine owner before sharing. Originals were never
transmitted; only these sanitized files exist here.

What each file proves (see the `*_evidence_*` tests):

- `codex-controlled-1.jsonl` (+ manifest + metadata): 3 submitted
  prompts → 3 `token_count` events with distinct, growing totals
  (14441 / 14494 / 14710). `last_token_usage` is a per-request
  delta, not a cumulative/repeated observation.
- `codex-session.jsonl` (+ metadata): 33 real `token_count` events
  with `rate_limits` snapshots (`plan_type: plus`, 300-min primary /
  10080-min secondary windows). Quota tail-scan and window parsing
  fixtures.
- `claude-thread.jsonl` (+ metadata): 72 real chunks over 27
  `(requestId, message.id)` identities with byte-identical repeated
  usage per logical response. Naive per-line summation would inflate
  totals ~3x; `message.id + requestId` dedup collapses each response
  to one record. All rows carry `cache_creation` + `cache_read`.

Constraints honored: no prompts, responses, tool content, paths,
hostnames, emails, keys, tokens, or raw UUIDs (package-local
`req_*`/`msg_*`/`uuid_*`/`lim_*` aliases only).
