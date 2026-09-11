# OpenAI Codex Integration Spec

Status: **Partial / UnverifiedSemantics** — parser strategy `codex-local-jsonl` (`LocalStructuredData`) is implemented for the observed `token_count` shape; authoritative request/delta semantics still require a redacted corpus reconciliation.

0. Source order (v1.3): `codex-app-server` (`SupportedClientApi`, kill switch `codex-app-server`) first, `codex-local-jsonl` fallback second. The App Server is spawned per scope with `CODEX_HOME` set to that scope's own root (account-scoped quota, §11.5): a custom profile's server reads that profile's auth only. Proven live 2026-09-11: an empty root reports an empty account (no type/email, `requiresOpenaiAuth:true`) — never the default login — so cross-profile fallback is structurally impossible, and an identity-less account classifies as `AuthenticationRequired`, never an empty success. Fallback quotas render with an explicit locally-observed banner and never become authoritative.

0.0. Executable selection is health-gated, not first-exists-wins: a stale system shim can exist as a file yet never answer RPC (observed: root-owned `/usr/local/bin/codex` with a broken vendor payload EOFs the handshake). Candidates are tried in static order; the first to answer a bounded `initialize` handshake serves. Positive results are cached per process; negatives are never cached so a cold/flaky first launch recovers next refresh. No healthy candidate ⇒ source skipped ⇒ JSONL fallback with banner. Verified live 2026-09-10 against codex-cli 0.144.5 (broken system shim auto-skipped, `~/.npm-global` install served).

0.1. App Server protocol (verified live against codex-cli 0.144.5, read-only): stdio JSON-RPC `initialize` → `account/read` (`refreshToken: false`, never refresh) → `account/rateLimits/read`. Response carries `account{type,planType}` (email never used/stored) and `rateLimits{limitId,primary,secondary,credits,planType}` + `rateLimitsByLimitId` multi-bucket view. Windows carry integer `usedPercent`, `windowDurationMins`, `resetsAt` and are `Fixed` by construction. Exactly one owned child process per logical fetch, killed on timeout/cancel/drop; executable from a static allowlist, no PATH/shell.

0. Current policy: a source `requestId` is used when present, then observed event `ordinal`, then stable `path:offset`; only the first two provide a source identity and neither proves repeated-request semantics. `Coverage::UnverifiedSemantics` is retained until reconciliation. Quota is scanned across all session tails (never newest-file-only); `window_kind` stays `Unknown` until window semantics are proven; `CODEX_HOME` is shared by refresh and import; custom roots are product-scoped.

1. Data: last request tokens, dynamic quota windows, reset timestamp, plan label; credits remain unexposed pending semantics.
2. Source: read-only local Codex JSONL session events.
3. Classification: local client data, observed schema; not a public provider API. A real local session shape probe passed on the host without checking the corpus into the repository.
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
16. Fixtures: synthetic envelope, explicit synthetic request identities, token categories, dynamic pools, plan type. The opt-in real-corpus test accepts only a user-supplied path and never stores it.
17. Forbidden: prompts/responses, auth file, raw event, path, email, tokens/credentials.
18. Closed client: historical files work; freshness may become stale.
19. Auth mutation: none; `auth.json` is not opened by the adapter.
20. Manual verification: compare pool, used/remaining percentage, reset, and plan with Codex UI at the same timestamp/account.
