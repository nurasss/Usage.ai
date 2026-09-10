# 1. Metadata

- Audit: final independent P0 Gate Audit after P0 Remediation Round 2.
- Audit time: `2026-09-09T10:23:19+05:00` (Asia/Almaty).
- Repository: `/Users/nuras/Desktop/usage.ai`.
- Scope: production code, tests, runtime/architecture behavior, and Master Spec v1.1. Remediation reports were treated as claims only.
- Source documents read:
  - `Usage.ai_MASTER_SPEC_v1.1.md` — 2,225 lines, SHA-256 `63976e26a1494f2cf10ea49f0cddffc945f4aa3170e2b31b70defe5862daaab0`.
  - `docs/TECHNICAL_AUDIT_V1.1_IMPLEMENTATION.md` — 365 lines, SHA-256 `e2f91020dc905db99ede5e21eb20a066bcd902001714f1d3ac6ee24a4f6a8cf9`.
  - `TECHNICAL_AUDIT_P0_REAUDIT.md` — 407 lines, SHA-256 `3349ccfea2f90685a66a30aa17948c49dc43ca5366e7d51326ca123759d7ad57`.
  - `P0_REMEDIATION_REPORT.md` — 124 lines, SHA-256 `03af1d6b13918347eee86bef43689a29c5bb7b14e43f8e720e894ef525c62a8b`.
  - `P0_REMEDIATION_ROUND2_REPORT.md` — 90 lines, SHA-256 `bdb2280d4df58f68bb5fe98e485e17aa5471fc48f14c8cac6614cd5c31492b4f`.
- No production, test, fixture, spec, or remediation-report file was changed. This report is the sole audit-created file.

## Audit conclusion

**Verdict D — DATA INTEGRITY / SECURITY RISK.** The green build is genuine, but Round 2 does not meet Gate B. There are reproducible code paths for secret leakage from the advertised redaction utility and for unverified local metrics to be presented/aggregated as authoritative without a visible limitation. The external-evidence path for Codex is also structurally incompatible with the production parser.

# 2. Repository state

Captured before creating this authorized report:

- Branch: `main`.
- HEAD: `d99554e289c40df6a393fb8d5ad71132ff353b1d`.
- Upstream: ahead of `origin/main` by 3 commits.
- State: dirty; no staged changes.
- Round 2 is not committed. HEAD is unchanged from the prior audit; Round 2 changes and its report are in the working tree.
- Tracked modifications: 44 files, `4,858 insertions`, `1,555 deletions`.
- Untracked material includes both remediation reports, prior re-audit, provider fixtures, new runtime modules, migrations 006/007, scripts, `src/App.test.ts`, and `src/lib/redact.test.ts`.

Modified tracked files reported by Git:

```text
Cargo.lock, Cargo.toml, README.md,
crates/usage-core/src/models.rs,
crates/usage-host/src/{cancel,facade,files,http,lib,process}.rs,
crates/usage-providers/Cargo.toml,
crates/usage-providers/src/{claude,codex,descriptor,lib,openai_api,strategy}.rs,
crates/usage-runtime/src/{account_router,history_import,lib,refresh,registry}.rs,
crates/usage-storage/src/lib.rs,
docs/INTEGRATION_MATRIX.md,
docs/TECHNICAL_AUDIT_V1.1_IMPLEMENTATION.md,
docs/integrations/OpenAI-Codex-Integration-Spec.md,
package-lock.json, package.json,
src-tauri/src/commands/accounts.rs,
src-tauri/src/{composition,dto,refresh_flow}.rs,
src-tauri/src/platform/hosts.rs, src-tauri/tauri.conf.json,
src/App.svelte,
src/lib/{api,demo,snapshot.test,types,ui.test,ui}.ts,
src/styles.css, tsconfig.json, vite.config.ts
```

Untracked paths reported by Git:

```text
P0_REMEDIATION_REPORT.md
P0_REMEDIATION_ROUND2_REPORT.md
TECHNICAL_AUDIT_P0_REAUDIT.md
crates/usage-providers/fixtures/{claude,codex}/
crates/usage-runtime/src/{aggregation,orchestration,root_resolver}.rs
crates/usage-storage/migrations/{006_legacy_identity,007_claude_cache_buckets}.sql
scripts/
src/App.test.ts
src/lib/redact.test.ts
```

Last five commits:

```text
d99554e C2 hosts: cancellation, opaque scoped files, Process/PTY, facade, descriptor sources
b267d73 C1 storage provenance: connections/files/generations, atomic import batch
750a0cd P0-02: official OpenAI page/next_page pagination, exact decimals
b4c04de v1.1: wire account router into production refresh path
0428edd v1.1 P0 stabilization: runtime/hosts/descriptors, SWR, one-fetch, narrow IPC, decomposed src-tauri
```

Toolchain:

```text
rustc 1.98.1 (48a229cea 2026-09-01)
cargo 1.98.1 (797e8a9bc 2026-08-05)
node v25.9.0
npm 11.12.1
```

Version claim is verified: workspace `Cargo.toml`, `package.json`, and `src-tauri/tauri.conf.json` all contain `1.0.0`; `node scripts/check-version.mjs` returned `version consistency: 1.0.0`.

# 3. Verification gates

All commands were run independently during this audit, in the requested order.

| Command | Actual result |
|---|---|
| `npm run check` | PASS; Svelte diagnostics: 0 errors, 0 warnings; version check PASS. |
| `npm test -- --run` | PASS; 5 files, **19 passed**, 0 failed. |
| `npm run build` | PASS; Vite 7.3.6, 144 modules, production bundle emitted. The `node:async_hooks` browser-externalization message is informational, not a failed gate. |
| `cargo check --workspace` | PASS. |
| `cargo test --workspace` | PASS; **163 passed**, 0 failed, **1 ignored**. Counts: 2 app + 16 core + 39 host + 37 provider + 45 runtime + 24 storage. |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS; 0 warnings. |

The ignored test is `codex::tests::real_codex_corpus_probe_is_opt_in_and_never_checked_in` (`crates/usage-providers/src/codex.rs:536-550`). It requires `CODEX_REAL_FIXTURE` pointing to an explicitly supplied real/redacted session. Its opt-in nature is appropriate for private external evidence, but it is a mandatory P0 semantic proof and therefore cannot count as passed evidence. There is no equivalent accepted real Claude corpus test.

# 4. Round 2 claim verification

| Claim | Production code | Test | Evidence quality | Verdict |
|---|---|---|---|---|
| Migration 006 preserves modern records | Guarded SQL preserves verified API account IDs; missing usage connection is backfilled | A/B/C/D plus managed API regression use pre-v6 schemas | Moderate | Mostly verified; trust elevation and boundary cases remain (§5). |
| SQLite parent/DB/WAL/SHM permissions | chmod helper runs after migration/WAL activation | Test asserts parent/DB; sidecars only if they happen to exist | Weak–Moderate | Partial, not an enforcement guarantee. |
| HTTP default 2 MiB and streaming cap | Single reqwest host streams `response.chunk()` with cumulative bound | exact limit, +1, Content-Length, chunked, 4 KiB and ~1 MiB | Strong | Verified. |
| HTTP cancellation | `tokio::select!` before headers and per body chunk, sharing one deadline | real delayed chunked servers in host and provider tests | Strong | Verified. |
| Disable-before-refresh | tombstone plus pre-semaphore/post-semaphore/storage checks | pre-first, queued, active, delete and shutdown tests | Moderate | Main paths verified; pre-commit TOCTOU remains (§9). |
| Exact Decimal | arbitrary-precision JSON number to `Decimal::from_str_exact`, text SQLite | exact long literal in real HTTP provider test; SQLite values round-trip | Strong | Verified for the production money path. |
| SWR component integration | runtime LKG -> DTO stale/current error -> real App card | `App.test.ts` mounts `App.svelte`, receives 87% DTO | Strong for SWR | Verified for P0-07, not for unverified-coverage honesty. |
| OpenAI integration | strategy -> HostFacade -> real Reqwest host -> TCP -> parser/mapper | two pages, exact decimals, bearer and cursor asserted | Strong | Verified through normalized provider result; SQLite is covered separately. |
| Redaction scripts are safe/ready | allowlist-shaped objects, but unsafe allowed values and schema drift | two happy-path tests only | Weak | **False; reproducible leak and unusable Codex evidence output.** |
| P0-05/P0-06 only need external evidence | parsers mark records unverified | Codex real probe ignored; synthetic tests only | Weak | **False as a Gate-B claim:** UI/aggregation and collection code still have code defects. |

# 5. Migration

`Storage::migrate` enables WAL and foreign keys, runs migrations 001–007 in one immediate transaction, calls `migrate_legacy_rows`, sets `user_version = 7`, and commits (`crates/usage-storage/src/lib.rs:159-193`). Pre-migration file databases are backed up (`:90-99`).

Migration 006 behavior:

- Candidate products include usage rows with `connection_id IS NULL` and cost/snapshot rows with nil/orphan accounts (`:1397-1408`).
- Usage account IDs are changed only for nil/orphan accounts or an Unknown, non-fingerprinted local-style account (`:1460-1473`).
- Remaining legitimate usage rows with no connection preserve `account_id` and receive a managed connection (`:1475-1509`).
- Cost and snapshot account IDs are changed only under the analogous nil/orphan/Unknown-local predicate (`:1512-1566`). Snapshot JSON is updated in the same transaction.

The earlier destructive migration defect is fixed for the covered modern API case. Cases B/C and the dedicated managed API test create a real v5 layout, not a post-v7 database with flags edited afterward. Case A uses v2 legacy state. Case D proves that reopening a completed v7 database does not change counts; this is useful but weaker than invoking the migration body twice.

Residual concerns:

1. A legitimate historical usage row with `connection_id IS NULL` is not automatically moved to Legacy if its managed account is trustworthy; this is the correct behavior and is explicitly tested at `:2179-2244`.
2. The backfilled managed connection is nevertheless hard-coded to `identity_confidence = 'Verified'` with no source identity (`:1490-1503`). Preserving the row owner is correct, but manufacturing verified provenance is not proven by the legacy row. This can misstate provenance even though it does not change totals/account ID in the covered scenario.
3. The Unknown-local predicate is broader than nil/orphan and intentionally moves existing rows. Its safety depends on the old account conventions (`external_identity IS NULL`, local/null ref, confidence Unknown); the tests cover the known schema shapes but not arbitrary partially upgraded production databases.
4. No deletion is performed; foreign keys remain enabled during the transaction. Mixed DB and data-count preservation are covered.

Migration verdict: prior account-rewrite regression fixed; **moderate confidence**, with provenance-confidence overstatement remaining.

# 6. Storage permissions

`Storage::open` migrates first, then calls `harden_db_permissions` (`crates/usage-storage/src/lib.rs:87-109`). On Unix it requests parent `0700`, DB `0600`, and existing `-wal`/`-shm` `0600` (`:123-143`). WAL is enabled before migration writes (`:159-168`), so normal tested opens produce sidecars in time for hardening.

The test `database_file_is_user_only` verifies actual metadata for parent and DB, and verifies each sidecar only under `if sidecar_path.exists()` (`:2639-2657`). It does not assert that WAL/SHM exist, does not test sidecar recreation, and all `set_permissions` errors are discarded. Consequently the report's unconditional “permissions enforced” wording is too strong: a failed chmod still returns a usable `Storage`, and a later-created sidecar is not re-hardened by this code. Parent `0700` provides an important containment layer, but the exact lifecycle claim is only partially evidenced.

# 7. HTTP safety

The production HTTP implementation is `ReqwestHttpHost`; repository search found no second unrestricted production reqwest client. Matches in `crates/usage-host/src/http.rs` are the intended host. `raw.bytes()` in `openai_api.rs:290` iterates a string for URL percent-encoding and is unrelated to response buffering.

Verified properties:

- Default hard cap is `2 * 1024 * 1024` bytes.
- Caller value `0` selects that cap; caller values cannot exceed the host cap.
- non-HTTPS is rejected in release code; test/debug loopback is explicitly limited to loopback hosts.
- `Content-Length > limit` is rejected before reading.
- body is consumed chunk-by-chunk; `bytes.len() + chunk.len() > limit` rejects overflow before append.
- exactly-limit succeeds; limit+1 fails.
- redirect policy is disabled and hostname allowlisting is checked before transport.
- cancellation and one absolute deadline are selected both around `send()` and every `response.chunk()`.

The two cancellation tests genuinely send headers and a partial chunk, wait 50 ms, cancel, then keep the server side open for another 500 ms. They do not accidentally complete a valid response before cancellation. A cancelled partial body never reaches JSON parsing or provider commit.

# 8. Money semantics

The audited path is exact:

```text
Reqwest chunk bytes
-> serde_json with arbitrary_precision
-> Number::to_string()
-> Decimal::from_str_exact
-> CostRecord.amount_decimal
-> SQLite TEXT
-> Decimal::from_str on read
-> Decimal aggregation
-> string DTO/export
```

Workspace features include `serde_json/arbitrary_precision` and `rust_decimal/serde-with-arbitrary-precision`. Search found no `as_f64` or `from_f64` in the money path. Remaining `f64` values are quota percentages/notification pacing, not money.

The real-host OpenAI integration asserts exact equality for `0.1234567890123456789` and other high-precision values. The SQLite test uses separate exact values and proves textual round-trip; it does not repeat that exact literal, but production storage is a generic Decimal-to-string path, so the combined evidence is strong.

# 9. Runtime/coalescing/cancellation

Coalescing is correctly registered before semaphore acquisition. A per-scope watch sender is inserted once; followers subscribe. `InflightGuard::drop` removes the entry and publishes the outcome, while a closed channel returns Cancelled instead of hanging (`crates/usage-runtime/src/refresh.rs:199-271`). The production test overlaps scheduled and manual triggers and observes one HTTP call. The leader-abort test proves follower completion and later recovery.

Lifecycle positives:

- `cancel_account` creates and retains a cancelled token even before first refresh.
- a queued task selects cancellation while waiting and rechecks after permit acquisition.
- `execute` checks cancellation and current stored account lifecycle before fetching.
- host operations receive the linked account/shutdown token.
- shutdown cancels active work.
- re-enable explicitly removes the tombstone through `reset_account_cancellation`; recreation with the same UUID is safe only if the normal enable/update path calls this method.

Residual race: `commit_payload` checks the cancellation token and account state, releases both locks, serializes the snapshot, then reacquires storage and calls `commit_refresh_bundle` (`refresh.rs:501-543`). A disable can commit between the final state check and transaction acquisition; `commit_refresh_bundle` itself does not recheck `accounts.enabled` or the token. Delete is normally protected by foreign-key failure, but disable has a real time-of-check/time-of-use window. Existing active-disable tests cancel while the source is delayed and do not force the race at the pre-commit boundary. This is a code-level lifecycle gap.

Persisted cooldown is strongly tested: coordinator A writes storage state, is dropped, coordinator B starts with fresh in-memory scope state, the fake clock proves zero calls before expiry and one new attempt after advancing 61 seconds.

# 10. SWR

The production chain exists:

```text
successful refresh -> transactional snapshot storage -> later source failure
-> runtime last-known-good stale outcome -> Tauri current-error overlay
-> App.svelte stale banner + retained value
```

`src/App.test.ts` is a real Svelte 5 component test. It mounts `App.svelte`; mocked `loadSnapshot` returns a production-compatible DTO with `remainingPercent: 87`, stale freshness, and current authentication error. The assertions require `Осталось 87%`, `Данные устарели`, and the current error simultaneously, and reject a Connected dot. P0-07 SWR is fixed with strong evidence.

This does not test a fresh `UnverifiedSemantics` card. That separate honesty failure is documented in §§15–16 and §19.

# 11. HostFacade

Provider `FetchContext` receives a `ProviderHostFacade`, not global `Hosts`. Runtime facade policy is descriptor-derived:

| Product | Files | HTTP | Scoped keychain | Clock/logger/network |
|---|---:|---:|---:|---:|
| Codex / Claude local | yes | no | no | yes |
| OpenAI API | no | yes, descriptor hostname allowlist | yes, fixed service | yes |
| discovery-only | no | no | no | yes |

Production provider search found no direct filesystem, environment, glob, reqwest, or process calls. The matches are confined to tests and the tripwire test itself. `ScopedRoot` and `ScopedFile` contain private paths; providers cannot construct them. Provider-facing `FilesHost` accepts only those opaque types. Canonical root validation, traversal rejection, symlink escape rejection, duplicate elimination, and custom-root normalization are tested.

Process/PTY are not used by current providers. Their skeletons use canonical executable allowlists and structured args, do not invoke a shell, and cancellation/timeout kill only the spawned child. The owned-child cancellation test is meaningful.

# 12. Architecture boundaries

The main refresh/import path is substantially moved to `usage-runtime`: orchestration, root resolution, account routing, refresh/coalescing, and aggregate construction live there. Tauri legitimately retains IPC, composition, DTO adaptation, cache, notification delivery, tray/window behavior.

However, the boundary is not complete:

- `src-tauri/src/commands/accounts.rs:258-331` directly constructs `OpenAiCostsStrategy`, builds its fetch context, calls availability/fetch, and interprets provider results in `test_connection`. This is provider strategy selection/execution in Tauri.
- `accounts.rs:333-359` fabricates local source diagnostics without probing the source, returning `Connected`, `LocalClientOnly`, `status_class = ok`, and no warning for every local descriptor.
- `src-tauri/src/refresh_flow.rs:244-285+` still implements per-provider token/cost business aggregation, despite the runtime aggregate module.

Descriptor source order is used by the normal refresh registry, but the Tauri connection-test path is an independent semantic path. P0-08 is therefore not fully closed at the runtime/Tauri boundary even though capability isolation itself is materially improved.

# 13. Account isolation/identity

B-01 runtime isolation is strong for new imports. Physical roots are canonicalized before hashing, so `~/foo` after expansion, `/Users/me/foo`, and an internal symlink resolving to the same directory converge on the same canonical root hash. `ensure_connection` rejects rebinding the same `(provider, product, root_hash)` to another account. Default/custom root tests assert distinct connections and row ownership.

B-02 safety is fixed: local account identity remains Unknown; unattributable local rows are rejected for confirmed connections; API credential mismatch stops before network/commit; first-seen API fingerprint is Weak rather than Verified; the synthetic A/B/A router test passes. UI aliases such as `Локальная история` do not confer identity confidence. Actual Codex/Claude account detection remains unavailable and must not be described as solved.

The false local `test_connection` diagnostics noted in §12 undermine presentation honesty but do not merge storage histories.

# 14. Replacement/atomicity

Replacement is generation-aware and transactionally scoped. End-to-end runtime evidence imports generation A (total 30), replaces the file, reimports generation B (total 2), and confirms only one generation-2 row remains. Storage deletion predicates use the specific source file/generation; neighboring connection/file rows are not globally deleted. The dedicated storage replacement test verifies atomic rebuild.

P0-04 fault injection is strong. The runtime test writes through the real import batch and injects failure after transactional state has begun changing; it verifies rows, checkpoint, and generation remain unchanged. The storage-level test inserts a usage row and checkpoint, then executes invalid SQL before commit and observes complete rollback. Attempts are intentionally handled separately and tested as such.

# 15. Codex evidence

Production records and quota snapshots are marked `Coverage::UnverifiedSemantics`; window kind stays Unknown. The narrowed descriptor glob enumerates only `sessions/**/rollout-*.jsonl` and `archived_sessions/**/rollout-*.jsonl`, removing the prior `*sessions` sibling overscan. The parser's synthetic behavior, stable ID fallback, latest-observation quota scan, partial-tail handling, malformed-line tolerance, and idempotence are covered.

Real semantics are not proven. The only real-corpus probe is ignored and requires `CODEX_REAL_FIXTURE`.

More importantly, the evidence workflow is not ready:

- Production parser expects `payload.type == token_count` and counters under `payload.info.last_token_usage` (`codex.rs:13-53, 75-105`).
- `redact-codex.mjs` reads/writes `payload.last_token_usage` directly and does not preserve `payload.info` (`scripts/redact-codex.mjs:62-77`).
- Therefore normal redacted token events do not have the schema consumed by the ignored production parser test. The claimed collection route cannot validate the intended semantics.

Production UI/aggregation also fails to honor the marker: exact quota/tokens are displayed with no visible unverified warning, and aggregate totals do not retain coverage. P0-05 is not merely externally blocked; it has active code-level honesty and evidence-tooling defects.

# 16. Claude evidence

Claude parser marks usage and snapshot coverage Unverified, uses stable message/request/UUID identities, applies a monotonic final-wins reducer for out-of-order chunks, and keeps cache creation/read buckets separate. `CLAUDE_CONFIG_DIR`, partial tails, repeated chunks, late chunks, and cache storage are covered only by synthetic fixtures.

No accepted real Claude corpus proves whether message/request IDs and cumulative chunk semantics match current client output. The redactor retains hashed message/request IDs, model, timestamp, and usage counters, but it drops envelope `uuid`; events lacking both message ID and request ID therefore fall back to path/offset in the parser, weakening dedup evidence. It also converts all counters through JavaScript `Number`, which can silently alter integers above `2^53 - 1`.

As with Codex, `UnverifiedSemantics` is available in hidden diagnostics but is not visibly warned on the fresh provider card or preserved in top-level aggregates. P0-06 remains externally unproven and code-level presentation is incomplete.

# 17. Redaction safety

The scripts construct new objects, so unknown top-level fields such as `secret_new_vendor_field`, `prompt_backup`, and `user_email` are dropped. The existing tests prove ordinary prompt/content/path/tool fields are absent.

They are **not safe enough for unattended use on personal logs**:

1. Codex copies `type` and `payload.type` verbatim (`redact-codex.mjs:56-65`) and copies sanitized-but-readable `plan_type` (`:31-40`). A synthetic input with values `secret_new_vendor_value_APIKEY123`, `prompt_backup_SECRET`, and `AliceSecretProject` reproduced those strings unchanged in sanitized output. This is a direct secret/user-data disclosure path through allowlisted key names.
2. IDs are unsalted, deterministic, 48-bit truncated SHA-256 tokens (`redact-codex.mjs:14-17`, `redact-claude.mjs:14-17`). Low-entropy values are dictionary-reversible and cross-corpus linkable. The code does not validate that input IDs are high entropy.
3. Both scripts synthesize current timestamps when source timestamps are absent. That changes evidence rather than preserving an explicit missing value.
4. Token counters are coerced with JavaScript `Number`, allowing precision loss for large integers and accepting non-finite/negative values until JSON serialization behavior intervenes.
5. Codex output schema does not match the production parser (§15).
6. Invalid timestamp values throw during `toISOString`, aborting the whole collection rather than safely rejecting one line.

Existing `src/lib/redact.test.ts` has two happy-path tests. It does not inject sensitive strings into allowed scalar fields, exercise low-entropy IDs, unknown malicious fields explicitly, missing/invalid timestamps, integer overflow, or feed sanitized output back through the production parser. Evidence quality is Weak.

# 18. Test quality

| Area | Evidence | Rating |
|---|---|---|
| migration | real v2/v5 databases; mixed and preservation cases; reopen idempotence | Moderate |
| account isolation | canonical roots, distinct connections, rebind rejection, A/B/A routing | Strong |
| replacement | runtime A -> B totals and storage transactional generation test | Strong |
| atomic import | post-write SQL/runtime fault and rollback assertions | Strong |
| coalescing | overlapping public scheduled/manual calls; one physical HTTP call | Strong |
| cancellation | pre-first, queued, active, shutdown, real chunked transport | Strong for tested points; Moderate overall due pre-commit TOCTOU |
| cooldown restart | persisted storage, new coordinator, fresh state, fake clock | Strong |
| SWR | runtime LKG, DTO bridge, real mounted App component | Strong |
| HostFacade | field-level capability tests, descriptor policy, source tripwire | Strong for capability cut; Moderate for architecture as a whole |
| OpenAI exact money | arbitrary-precision real HTTP literal plus SQLite textual round-trip | Strong |
| HTTP bounds | real TCP Content-Length/chunked/exact/+1/cancel/timeout tests | Strong |
| Codex semantics | synthetic fixtures; ignored real probe; incompatible redactor output | Weak |
| Claude semantics | synthetic reducer/cache fixtures; no real corpus | Weak |

# 19. New regressions

## R1 — redaction secret exposure (reproduced, security)

Allowed scalar keys are assumed safe regardless of value. A new vendor/client schema can place user or secret text into `type`, `payload.type`, or `plan_type`, and `redact-codex.mjs` emits it. This satisfies the strict Verdict-D “secret exposure” condition.

## R2 — Codex evidence artifact is parser-incompatible (reproduced structurally)

The redactor emits token counters at a different JSON path than `parse_token_line` accepts. The opt-in P0 proof cannot validate ordinary redacted Codex token events produced by the supplied command.

## R3 — unverified metrics appear authoritative (reproduced from production render path)

`App.svelte:343-379` warns only on stale/current error. A fresh Connected provider with `UnverifiedSemantics` shows exact remaining percent and tokens with no visible warning; coverage exists only in collapsed `<details>`/tooltip. `usage-runtime/aggregation.rs:54-114` sums usage without filtering or carrying coverage, and App labels the result simply “tokens”. This satisfies the Verdict-D “fake healthy state” condition and can produce authoritative-looking totals from explicitly unverified semantics.

## R4 — local connection test fabricates health (production path)

`src-tauri/src/commands/accounts.rs:333-359` returns Connected/ok/LocalClientOnly for any local descriptor without reading a root or running its strategy. It also overwrites the source's Unverified semantics. This is a second fake-healthy path.

## R5 — disable/commit TOCTOU (code-level P0)

Cancellation/lifecycle checks and the storage transaction are not one atomic decision. Disable can occur after the checks but before `commit_refresh_bundle` obtains the lock. The tests do not control that boundary.

## R6 — permission enforcement is fail-open

Every chmod error is ignored, and sidecar assertions are conditional. No current standard macOS exposure was observed, but the stated guarantee is not enforced.

No new unrestricted HTTP implementation, float money path, broad archived glob, direct provider OS bypass, neighbor-file deletion, or coalescing deadlock was found.

# 20. Final P0 matrix

| Issue | Status | Code evidence | Test evidence | External evidence | Confidence |
|---|---|---|---|---|---|
| B-01 root/account isolation | ✅ FIXED | canonical root hash + connection ownership | default/custom and same-root rejection | not required | High |
| B-02 safety | ✅ FIXED | Unknown bucket, mismatch rejection, no automatic merge | A/B/A and confirmed-connection rejection | not required | High |
| B-02 real identity | ⛔ BLOCKED_BY_EXTERNAL_EVIDENCE | no trustworthy local account observer | synthetic safety only | absent | High |
| P0-01 replacement | ✅ FIXED | file generation and scoped transactional replacement | A=30 -> B=2; storage generation test | not required | High |
| P0-02 OpenAI pagination/partial/exact | ✅ FIXED | `page`/`next_page`, partial result, exact Decimal | real HTTP multi-page + page-failure tests | official contract represented | High |
| P0-03 coalescing | ✅ FIXED | pre-semaphore inflight + RAII | scheduled/manual one call; leader abort | not required | High |
| P0-04 atomic import | ✅ FIXED | single immediate transaction | post-write fault rollback | not required | High |
| P0-05 Codex semantics | ⚠️ REGRESSION | Unverified internally, but UI/aggregate honesty and redactor schema fail | ignored real probe; synthetic only | absent | High |
| P0-06 Claude semantics | 🟡 PARTIAL | reducer/cache logic sound; UI/aggregate marker hidden | synthetic only | absent | High |
| P0-07 SWR | ✅ FIXED | LKG + current error DTO/UI | real component test | not required | High |
| P0-08 capability/architecture | 🟡 PARTIAL | HostFacade strong; Tauri direct strategy/business paths remain | capability tests strong | not required | High |
| P0-09 restart cooldown | ✅ FIXED | persisted lazy restore | fresh coordinator + fake clock | not required | High |
| Round 2 storage permissions | 🟡 PARTIAL | chmod is fail-open | sidecars conditional | not required | High |
| Round 2 lifecycle cancellation | 🟡 PARTIAL | broad cancellation coverage; pre-commit race | no boundary-controlled race test | not required | Moderate–High |
| External collection safety | 🔴 UNRESOLVED | scalar leaks, unsalted low-entropy hashes, schema drift | happy paths only | cannot safely collect yet | High |

# 21. Verdict

## D — DATA INTEGRITY / SECURITY RISK

Gate A fails because real Codex/Claude evidence is absent.

Gate B also fails. Its necessary conditions are not met:

- Codex/Claude are not the only blockers; redaction, UI/aggregation honesty, local health diagnostics, lifecycle atomicity, and architecture boundary issues remain in code.
- production does not clearly and visibly mark fresh unverified metrics; instead it can show them as Connected exact totals.
- the advertised collection path is neither safely value-constrained nor compatible with the Codex production parser.
- a synthetic audit probe reproduced sensitive allowed-field values in sanitized output.

The result is not C because the strict rules reserve D for a reproducible secret-exposure or fake-healthy path, and both exist here.

**New provider expansion remains frozen.**

# 22. Exact next action

Do not collect or transmit personal Codex/Claude logs with the current scripts. The next remediation round must be narrowly limited to these gate blockers, followed by another independent audit:

1. make redaction output a value-level allowlist with fixed enums/ranges, safe non-linkable identifier handling, explicit missing timestamps, integer validation, and a production-parser round-trip test;
2. make Codex redaction schema exactly match `payload.info.last_token_usage` and preserve only the minimum semantics needed for the probe;
3. visibly label or exclude `UnverifiedSemantics` everywhere exact quota/token values and aggregate totals are shown;
4. stop fabricating Connected/ok local diagnostics; route connection testing through runtime-owned source logic;
5. close the cancellation/lifecycle check inside the same storage transaction/serialization point as commit;
6. make Unix permission hardening fail closed and verify WAL/SHM creation/recreation explicitly.

After those code changes and green gates, collect the external evidence package only with explicit user authorization and manual inspection. No new provider work is in scope before the gate is re-passed.

# 23. Required External Evidence Package

This section defines the eventual minimum evidence, but **the current scripts are not approved for use**. No personal log was read or processed during this audit.

## Codex minimum

At least two real, manually reviewed sessions that establish:

- outer timestamp;
- `payload.type = token_count`;
- stable high-entropy request ID or ordinal needed for dedup, transformed with a per-package random salt/token map;
- `payload.info.last_token_usage`: input, cached input, output, reasoning output, total;
- model only if it matches a strict known model-ID grammar/enumeration;
- rate-limit event fields needed to establish primary/secondary meaning: fixed allowed event kind, opaque high-entropy limit ID, used percent, window minutes, reset epoch, and a strict plan enum;
- repeated event for one request, multiple requests, out-of-order observations, file boundary, partial trailing line, and archived/current session examples;
- hand-calculated expected per-request and total counters plus expected quota observation.

The evidence must answer whether `last_token_usage` is a delta or cumulative value and whether request IDs repeat across streaming/replay events.

## Claude minimum

At least two real, manually reviewed assistant streams that establish:

- timestamp;
- stable transformed envelope UUID and/or message ID + request ID;
- strict model ID;
- input/output/cache-creation/cache-read counters;
- multiple chunks of the same response, an out-of-order/late chunk, duplicate/replayed line, and separate response;
- hand-calculated final-wins totals and cache-bucket expectations.

The evidence must establish whether counters are cumulative per logical response and which identifiers remain stable across chunks.

## Safe commands

There are **no safe approved commands at this revision**. The implemented CLI syntax is:

```text
node scripts/redact-codex.mjs <input.jsonl> <output.jsonl>
node scripts/redact-claude.mjs <input.jsonl> <output.jsonl>
```

but these commands must not be recommended against personal data until §§15 and 17 defects are remediated and re-audited. In particular, do not run the current scripts on `~/.codex` or `~/.claude` data as an audit workaround.

## Manual review requirement

Before any future sanitized fixture leaves the user's machine, open the entire output and verify it contains no request/response text, prompt, tool arguments/results, path, cwd, email, credential, token, secret, account/user/project name, hostname, environment value, or low-entropy linkable identifier. Also verify all counters and event ordering against a private hand-calculated reference. Only the sanitized artifact—not the original log—may be supplied for external semantic validation.
