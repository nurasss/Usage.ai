# OpenAI Codex Integration Spec

Status: **Implemented (v1.1)** — strategy `codex-local-jsonl` (`LocalStructuredData`), schema `codex-jsonl-token_count-v1`.

0. v1.1 semantics: every `token_count` event is one request delta keyed by stable `path:offset` identity; quota is scanned across all session tails (never newest-file-only); `window_kind` stays `Unknown` until window semantics are proven; `CODEX_HOME` is shared by refresh and import; custom roots are product-scoped.

1. Data: last request tokens, dynamic quota windows, reset timestamp, plan label; credits remain unexposed pending semantics.
2. Source: read-only local Codex JSONL session events.
3. Classification: local client data, observed schema; not a public provider API.
4. Permissions: read selected Codex sessions directory.
5. Identity: not proven by session file; each configured root remains a separate account and is never auto-merged.
6. Pools: dynamic primary and optional secondary under `limit_id`.
7. Units: integer tokens and validated percentage.
8. Windows: `window_minutes` + Unix `resets_at`; countdown expiry remains unconfirmed until a new snapshot.
9. API billing: not supplied by this source.
10. Delay: local event time; freshness is computed from source timestamp.
11. History: yes, per-event token usage.
12. Pagination: byte checkpoint per JSONL path; append/partial line supported.
13. Rate limits: data source is local; no monitoring request.
14. 401/403/429: not applicable to local source; provider-neutral scheduler behavior is tested.
15. Stable fields: only observed `token_count` envelope is accepted; absent/new fields are tolerated.
16. Fixtures: synthetic envelope, token categories, dynamic pools, plan type.
17. Forbidden: prompts/responses, auth file, raw event, path, email, tokens/credentials.
18. Closed client: historical files work; freshness may become stale.
19. Auth mutation: none; `auth.json` is not opened by the adapter.
20. Manual verification: compare pool, used/remaining percentage, reset, and plan with Codex UI at the same timestamp/account.

