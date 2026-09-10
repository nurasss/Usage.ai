# Usage.ai — Independent P0 Gate Re-audit

## 1. Audit metadata

- Audit date: 2026-09-09 (Asia/Almaty).
- Audit type: independent repeat P0 Stabilization Gate audit after remediation.
- Specification: `/Users/nuras/Downloads/Usage.ai_MASTER_SPEC_v1.1.md` (read in full).
- Previous audit: `docs/TECHNICAL_AUDIT_V1.1_IMPLEMENTATION.md` (read in full).
- Developer claims: `P0_REMEDIATION_REPORT.md` (read in full; treated as claims, not evidence).
- Audited checkout: `/Users/nuras/Desktop/usage.ai`.
- Audit constraints observed: no source, test, fixture, specification, or remediation-report changes. This file is the only audit output.

The specification's gate is strict: new private/local sources remain frozen until all P0 acceptance criteria are closed (`MASTER_SPEC`, lines 163–194 and 1641–1675). Green commands are necessary but not sufficient.

## 2. Repository state

Commands executed exactly as requested:

```text
git branch --show-current
git rev-parse HEAD
git status --short
git status
git log -5 --oneline
git diff --stat
```

- Branch: `main`.
- HEAD: `d99554e289c40df6a393fb8d5ad71132ff353b1d`.
- Upstream: ahead of `origin/main` by 3 commits.
- State: **dirty**, with no staged changes.
- Latest commits:
  - `d99554e C2 hosts: cancellation, opaque scoped files, Process/PTY, facade, descriptor sources`
  - `b267d73 C1 storage provenance: connections/files/generations, atomic import batch`
  - `750a0cd P0-02: official OpenAI page/next_page pagination, exact decimals`
  - `b4c04de v1.1: wire account router into production refresh path`
  - `0428edd v1.1 P0 stabilization: runtime/hosts/descriptors, SWR, one-fetch, narrow IPC, decomposed src-tauri`
- Tracked diff: 40 files, 3,272 insertions, 1,496 deletions. This excludes untracked files.
- Modified tracked files: `Cargo.lock`, `README.md`, core models; host cancellation/facade/files/http/lib/process; provider Cargo/Claude/Codex/descriptor/lib/OpenAI/strategy; runtime account router/history import/lib/refresh/registry; storage lib; three docs; package manifests; Tauri accounts/composition/dto/hosts/refresh flow/config; Svelte app/API/demo/types/UI tests/UI/styles.
- Untracked: `P0_REMEDIATION_REPORT.md`, the new Codex and Claude fixture directories, runtime `aggregation.rs`, `orchestration.rs`, `root_resolver.rs`, migrations 006/007, and `scripts/`.

Conclusion: three early remediation slices are committed at HEAD, but a substantial part of the audited remediation exists **only in the working tree**. The audit therefore describes this exact dirty checkout, not a reproducible commit.

Toolchain and application versions:

| Item | Observed |
|---|---:|
| `rustc` | 1.98.1 (2026-09-01) |
| `cargo` | 1.98.1 (2026-08-05) |
| Node | v25.9.0 |
| npm | 11.12.1 |
| Cargo workspace | 1.0.0 |
| `package.json` | 1.0.0 |
| `src-tauri/tauri.conf.json` | 1.0.0 |

Versions are synchronized. `node scripts/check-version.mjs` independently passed. `npm run check` invokes that script, and CI invokes `npm run check` (`package.json`; `.github/workflows/ci.yml`, portable job), so the assertion is present in CI indirectly.

## 3. Verification commands

All required commands were run against the checkout above.

| Command | Result | Independent observation |
|---|---|---|
| `npm run check` | PASS | Svelte: 0 errors, 0 warnings; version consistency 1.0.0 |
| `npm test -- --run` | PASS | 3 files, **13 passed**, 0 failed |
| `npm run build` | PASS | Vite 7.3.6, 119 modules, production bundle produced |
| `cargo check --workspace` | PASS | workspace check completed |
| `cargo test --workspace` | PASS | **138 passed, 0 failed, 1 ignored** |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS | 0 warnings/errors |

Actual Rust suite breakdown:

| Suite | Passed | Ignored |
|---|---:|---:|
| `usage-ai-lib` | 1 | 0 |
| `usage-ai-app` main | 0 | 0 |
| `usage-core` | 16 | 0 |
| `usage-host` | 27 | 0 |
| `usage-providers` | 35 | 1 |
| `usage-runtime` | 41 | 0 |
| `usage-storage` | 18 | 0 |
| doc tests | 0 | 0 |

The remediation report's overall `138 / 0 / 1` and frontend `13` claims are correct. Its per-crate breakdown is not: it claims `usage-host: 31`, `usage-ai-lib: 7`, and `usage-core: 6`, while the independent run produced 27, 1, and 16 respectively. The incorrect entries happen to cancel numerically.

The ignored test is `codex::tests::real_codex_corpus_probe_is_opt_in_and_never_checked_in` (`crates/usage-providers/src/codex.rs`, lines 535–550). It requires `CODEX_REAL_FIXTURE` and real/redacted external data. It is directly related to the mandatory Codex semantics evidence; therefore P0 Codex semantics cannot be called done while this is the only real-corpus probe and it is not run.

## 4. Remediation report claim verification

| Claim | Code evidence | Test evidence | Independently proven? |
|---|---|---|---|
| B-01 root isolation | `Storage::ensure_connection`, storage lines 326–370; runtime orchestration lines 155–249 | root isolation and same-root tests | **No, as a whole.** New roots are isolated, but migration 006 corrupts trustworthy cost/snapshot attribution; see §5. |
| B-02 honest identity | account router lines 21–88; orchestration lines 25–43, 283–302; UI formatter lines 32–43 | router A→B→A, import rejection, UI label unit tests | **Mostly.** Architecture safely treats local identity as unknown; no real local account switch is observable. |
| P0-01 replacement | `commit_import_batch`, storage lines 468–535 | runtime replacement test lines 524–617; storage replacement test lines 2126–2190 | **Yes**, with moderate rather than full UI-overview evidence. |
| P0-02 OpenAI partial + exact money | OpenAI lines 44–49 and 337–425 | page-two tests lines 683–744; decimal test 604–616 | **Partial only.** Pagination is fixed; the “no float roundtrip” claim is false. |
| P0-03 coalescing/RAII | refresh lines 199–265 | production coalescing 1005–1059; leader abort 1400–1445 | **Yes.** |
| P0-04 atomic import | storage lines 468–535 | runtime injected fault 359–454; storage rollback 2193–2225 | **Yes.** |
| P0-05 Codex | descriptor lines 98–115; Codex parser/coverage | synthetic fixture tests plus ignored real-corpus probe | **No.** External evidence is absent; wildcard adds an over-scan risk. |
| P0-06 Claude | Claude lines 14–232; migration 007; storage upsert | synthetic streaming/cache tests | **No.** External provenance is absent. |
| P0-07 SWR/UI | refresh lines 481–565; Tauri adapter 187–233; Svelte card 347–356 | runtime SWR; DTO unit test; handcrafted DOM test | **Functionally supported, but UI test claim overstated.** The DOM test does not render `App.svelte`. |
| P0-08 HostFacade/cancellation | host facade; strategy `FetchContext`, lines 120–147 | facade policy, tripwire, process child test | **Partial only.** Production HTTP limit/cancellation defects remain; see §§14 and 16. |
| P0-09 cooldown restart | refresh lines 616–650 | restart test lines 1219–1263 | **Yes.** |

## 5. B-01 — root/account isolation

### Default/custom roots and same-root rebind

The production path is now coherent:

1. managed account/custom path is read in `usage-runtime/src/orchestration.rs`, lines 49–132;
2. roots are resolved in `root_resolver.rs`, lines 9–58;
3. each canonical root is hashed and passed to `ensure_connection` in orchestration lines 210–248;
4. connection/file/generation IDs are attached in `history_import.rs` and committed through `commit_import_batch`;
5. storage aggregation filters by account/product, and Tauri adapts runtime aggregates.

`Storage::ensure_connection` derives the connection UUID from provider/product/root and rejects a different owner (lines 339–368). `same_root_cannot_silently_rebind_between_accounts` (lines 2084–2124) models root X→A then root X→B and verifies rejection. `default_and_custom_roots_never_share_rows` (`history_import.rs`, lines 456–522) exercises two accounts, two roots, actual file parsing, provenance, and storage. These two subclaims are strong.

### Legacy contamination — critical failure

`migrate_legacy_rows` (`usage-storage/src/lib.rs`, lines 1380–1457) is unsafe:

- only the `usage_records` product selection is restricted to `connection_id IS NULL` (lines 1385–1389);
- **all** `cost_records` and **all** `snapshots` for every selected product are then reassigned to a generated legacy account (lines 1449–1456), regardless of their existing account identity;
- `snapshots.payload_json` is not rewritten, so its embedded `Snapshot.account_id` can disagree with the row's new `account_id`;
- the migration affects databases at version 1–5 (`Storage::migrate`, lines 153–186), including pre-remediation OpenAI API costs and LKG snapshots that were already tied to an explicit API-key account.

Reproducible path: create a schema-v5 database with account A, an `openai-api` cost row and snapshot for A, then open it with current `Storage::open`. Migration 006 creates the legacy OpenAI API account and changes both rows to that legacy UUID. This produces wrong account cost attribution and moves A's LKG snapshot away from A. It is exactly the data-integrity category described by verdict D.

`migrates_real_v1_database_without_data_loss` (lines 1842–1882) is insufficient: it creates only a v1 Codex `usage_records` row. It asserts neither existing `cost_records`, existing snapshots, multiple accounts, payload identity, nor a realistic v5 database. Thus it cannot catch the destructive reassignment.

**B-01 status: ⚠️ REGRESSION / NEW RISK.** New import isolation is improved, but the required legacy-contamination part is not safe.

## 6. B-02 — local identity

The implementation now honestly separates unattributed local history from confirmed identities:

- default local accounts are stable product buckets labelled `Локальная история`, with no external identity (`orchestration.rs`, lines 25–43 and 283–291);
- managed local accounts use `IdentityConfidence::Unknown` (`commands/accounts.rs`, lines 146–160);
- unknown local data may enter only an unknown connection; unattributed data is rejected for Weak/Verified connections (`account_router.rs`, lines 51–88; `history_import.rs`, `confirmed_connection_rejects_unattributable_rows`, lines 619–667);
- the UI replaces generic `Аккаунт`/`Аккаунт N` for unknown identity (`src/lib/ui.ts`, lines 32–43).

The synthetic A→B→A test (`account_router.rs`, lines 159–194) correctly proves the router rule. It does **not** prove that Codex or Claude exposes a real identity. Production explicitly returns no observed local identity (`refresh.rs`, lines 567–595). Therefore account switches inside those clients cannot currently be detected; history is deliberately a per-root unknown-identity bucket.

**B-02 status: ✅ FIXED for architecture safety and honest presentation.** Real local identity observation remains an explicit limitation, not a capability.

## 7. P0-01 — replacement provenance

`history_import` reads an opaque file identity, recognizes inode/device change or truncation, assigns the next generation, and sends replacement state plus rows/checkpoint to `commit_import_batch`. That function deletes older-generation rows, updates file identity/generation, upserts replacement records, and updates the offset inside one immediate SQLite transaction (`usage-storage/src/lib.rs`, lines 468–535).

`replacement_rebuilds_totals_without_inflation` (`history_import.rs`, lines 524–617) executes real file import: generation 1 totals 30, the file is replaced, generation 2 totals 2, final storage total is 2 rather than 32, and file generation becomes 2. `replacement_rebuilds_old_generation_rows_atomically` provides the storage-level counterpart. Truncation and inode-change behavior is additionally covered at storage lines 1682–1716.

The runtime test checks the same storage aggregate used by the overview pipeline, although it does not instantiate the final `AppSnapshot`. Evidence is still sufficient for the core invariant.

**P0-01 status: ✅ FIXED. Confidence: high.**

## 8. P0-02 — OpenAI Costs

### Current official contract

Checked on 2026-09-09 against the [OpenAI Usage API reference](https://platform.openai.com/docs/api-reference/usage/audio_transcriptions_object) and the generated [official openai-python costs parameters](https://github.com/openai/openai-python/blob/main/src/openai/types/admin/organization/usage_costs_params.py). The request cursor is `page`; it corresponds to response `next_page`; response also carries `has_more`. Current code follows this contract (`openai_api.rs`, lines 50–75 and 257–276), stops on `has_more == false`, and bounds pagination.

Page 1 success followed by page 2 network or schema failure returns page-1 records, `Coverage::Partial`, warning, and `source_error` (`openai_api.rs`, lines 337–425). Tests at lines 683–744 genuinely model both failure modes. Snapshot and costs are built from one pass; one-fetch tests count physical calls. Stable IDs include currency, project, line item, and API key.

### Money precision claim is false

`decimal_from_number` calls `serde_json::Number::to_string()` and then parses `Decimal` (`openai_api.rs`, lines 35–49). The workspace enables only serde_json default/std features, not `arbitrary_precision`. A non-integer JSON number has therefore already passed through serde_json's binary floating representation before `to_string`; avoiding the spelling `as_f64()` does not avoid the equivalent roundtrip.

The test named `decimal_amounts_stay_exact_without_float_roundtrip` uses only `19.99` and `0.06` (lines 604–616). Their shortest binary-float render happens to reproduce the same short text, so the test cannot disprove the old bug. A precision-sensitive literal such as `0.1234567890123456789` normalizes to `0.12345678901234568` through an IEEE-754 JSON-number path. Storage later preserves the already-rounded `Decimal` as text (`usage-storage/src/lib.rs`, lines 625–650).

### Production HTTP defect affecting Costs

`ReqwestHttpHost` derives `Default`, leaving `max_bytes_default = 0`; production constructs it with `ReqwestHttpHost::default()` (`src-tauri/src/platform/hosts.rs`, lines 12–22). The effective limit expression is `req.max_bytes.min(self.max_bytes_default).max(1024)` (`usage-host/src/http.rs`, line 64), so every production response is capped at **1 KiB**, not the requested 2 MiB. Provider tests use `ScriptHttp` and bypass this code. A normal multi-row Costs page can therefore be rejected while all current OpenAI tests stay green.

**P0-02 status: ⚠️ REGRESSION / NEW RISK.** Pagination/partial behavior is fixed, but exact money and the production transport path are not.

## 9. P0-03 — real coalescing

The production order is correct: the per-scope watch sender is found/registered before semaphore admission (`refresh.rs`, lines 199–260). Followers subscribe and await the same result; different scope keys obtain independent executions. `InflightGuard::drop` removes the sender and publishes either the result or channel closure.

`production_triggers_coalesce_into_one_execution` (lines 1005–1059) uses concurrent public `refresh_many` calls with scheduled/manual triggers and proves one `ScriptHttp` execution. `different_scopes_run_in_parallel` proves two account scopes execute twice. `leader_drop_cleans_up_inflight_and_allows_subsequent_refresh` (lines 1400–1445) aborts the leader, bounds the waiter's completion, and verifies a subsequent refresh succeeds.

The abort test is timing-based (50 ms), but its assertions target the correct failure mode. No permanent waiter/inflight leak was found.

**P0-03 status: ✅ FIXED. Confidence: high.**

## 10. P0-04 — atomic rows + checkpoint

`commit_import_batch` uses one `TransactionBehavior::Immediate` transaction for old-generation deletion, generation update, every record upsert, `file_imports` checkpoint update, and commit (`usage-storage/src/lib.rs`, lines 468–535).

The runtime fault test (`history_import.rs`, lines 359–454) first imports a file, appends a second record, injects `ImportBeforeCheckpoint` after row writes but before checkpoint, and verifies total, byte offset, and generation are unchanged. This is a failure inside the transaction, not pre-transaction validation. The direct storage rollback test at lines 2193–2225 is supportive but less production-specific.

**P0-04 status: ✅ FIXED. Confidence: high.**

## 11. P0-05 — Codex semantics

Code-level improvements are real: `CODEX_HOME` and default `.codex` are descriptor fields; custom `sessions`/`archived_sessions` paths normalize to product home; request-ID/path-offset identities are stable; malformed and trailing partial lines are handled; generations protect replacement; repeated import is idempotent; quota window kind remains `Unknown`; records use `Coverage::UnverifiedSemantics`.

However, all checked-in fixtures under `fixtures/codex/` are synthetic/provenance-unknown. There is no README or provenance note. The only real-corpus test is ignored and only asserts that some rows parse and remain unverified; even if manually enabled, it does not compare hand-verified delta/cumulative totals. This does not meet Master Spec lines 1657–1659.

There is also a new scope issue: descriptor glob `*sessions/**/rollout-*.jsonl` (`descriptor.rs`, line 110) matches `sessions` and `archived_sessions`, but also any other child ending in `sessions`. Within `CODEX_HOME`, a coincidentally named directory with `rollout-*.jsonl` can be scanned. The host prevents escape outside the scoped root, but the pattern is broader than the two claimed directories and has no negative test.

**P0-05 status: 🟡 PARTIAL.** Implementation safety improved, real-source semantics evidence is blocked, and the wildcard should be narrowed before treating file scope as exact.

## 12. P0-06 — Claude semantics

The parser reads `message.id`, `requestId`, UUID fallback, model, input/output, and separate cache creation/read fields (`claude.rs`, lines 14–113). The reducer and storage upsert apply monotonic per-field maxima; migrations/domain/storage keep creation and read buckets distinct (`claude.rs`, lines 123–155; migration 007; storage SQL line 12). Synthetic tests cover a late smaller chunk, duplicate identity, and cache separation.

Evidence limitations:

- `fixtures/claude/` has no provenance note and cannot be classified as real-redacted;
- `message.id + requestId` semantics are explicitly assumed pending real evidence (`claude.rs`, lines 39–43);
- checked-in tests validate values authored for the implementation, not a captured/reviewed Claude Code corpus;
- unlike Codex, Claude records are labelled `Coverage::LocalClientOnly`, not `UnverifiedSemantics` (`claude.rs`, lines 191–211), so the UI/storage does not surface the remaining semantic uncertainty.

**P0-06 status: ⛔ BLOCKED_BY_EXTERNAL_EVIDENCE**, with an additional honesty gap in coverage labelling. It cannot be upgraded to fixed from synthetic fixtures alone.

## 13. P0-07 — SWR

The core path is implemented:

- a successful bundle transaction stores the snapshot (`refresh.rs`, lines 481–530; storage lines 653–724);
- failure loads the latest stored snapshot and rewrites freshness to stale (`refresh.rs`, lines 533–565; `snapshot.rs`, lines 13–21);
- Tauri overwrites stale `Connected` with the current auth/network/source state, adds `current_error`, and carries last success/attempt (`refresh_flow.rs`, lines 187–233);
- Svelte renders old metrics and a stale/current-error banner with timestamps (`App.svelte`, lines 347–356);
- retention keeps the newest snapshot per account/product (`storage/lib.rs`, lines 801–850).

The backend/runtime tests are useful. The claimed “full card DOM” test is not a component-render test: `src/lib/ui.test.ts`, lines 42–112 manually constructs an HTML string that resembles the component. It could remain green if `App.svelte` stopped rendering the banner. Direct code inspection confirms the current component does render it, but test evidence is only moderate.

**P0-07 status: ✅ FIXED. Confidence: moderate.** The behavior exists; the remediation report overstates UI test strength.

## 14. P0-08 — HostFacade security

Provider strategy inputs no longer receive unrestricted `Hosts`: `FetchContext` contains `ProviderHostFacade`, opaque `ScopedRoot`, secret bytes for this call, and cancellation (`strategy.rs`, lines 120–147). Facade policy gives Codex/Claude files but no HTTP/keychain and OpenAI HTTP/scoped keychain but no files (`facade.rs`; `runtime/registry.rs`, lines 35–57). Provider-facing file methods accept only `ScopedRoot`/`ScopedFile`; constructors' raw fields are private. Searches found no production `std::fs`, `std::env`, `glob`, `reqwest`, or process calls inside providers; matches are test-only or comments. Keychain service is runtime-owned. Process execution canonicalizes an allowlisted executable and never exposes `kill(pid)`.

Remaining enforcement defects:

1. Production HTTP is unintentionally limited to 1 KiB because `max_bytes_default` defaults to zero (§8).
2. `response.bytes().await` is outside both the cancellation `select!` and the request timeout (`usage-host/src/http.rs`, lines 72–125). Cancellation after headers does not interrupt a stalled body read until the outer strategy timeout drops the whole fetch. The test `cancelled_request_finishes_without_transport` checks only a token cancelled before sending.
3. When `Content-Length` is absent, `response.bytes()` buffers the complete body before checking its length, so `max_bytes` is not an allocation/streaming bound.
4. `cancel_account` cancels only an already-existing token (`refresh.rs`, lines 124–151). If disable/delete wins before a queued refresh first creates the map entry, the later refresh obtains a fresh uncancelled token. Existing cancellation test sleeps until work is already active and does not cover this ordering.
5. `ProcessHost` waits for child exit before reading stdout/stderr (`process.rs`, lines 93–130). A verbose child can block on a full pipe until timeout; truncation after `read_to_end` is not an I/O memory bound.

The owned-child cancellation test is valid for the process it spawns and cannot kill a foreign PID: only the local `Child` handle is killed. That subclaim is strong.

**P0-08 status: 🟡 PARTIAL.** Capability shape is substantially improved, but HTTP and lifecycle enforcement are not complete.

## 15. P0-09 — cooldown restart

`restore_scope_cooldown` consults persistent storage when the new scope has no live future cooldown and matches account plus source (`refresh.rs`, lines 616–650). The restart test (lines 1219–1263) creates coordinator A, persists a 429 cooldown, drops A, creates B over the same storage, verifies zero additional calls before expiry, advances an injected clock, then observes one new call. It does not reuse A's state map and uses no real sleep for expiry.

**P0-09 status: ✅ FIXED. Confidence: high.**

## 16. Host security

Positive findings: exact HTTPS host allowlist, redirects disabled, bearer secret not logged, descriptor-owned capability selection, scoped keychain namespace, opaque files, symlink escape rejection, path traversal rejection, child-handle-only termination, and a source-code bypass tripwire.

Negative findings: production HTTP 1 KiB limit, unbounded body buffering, cancellation not racing body reads, and the queued-refresh cancellation ordering described in §14. The code tripwire checks provider source text only; it does not exercise the real `ReqwestHttpHost`, macOS Keychain, or full application lifecycle. The Codex wildcard is scoped to `.codex`/custom root but broader than the stated directories.

No direct secret exposure was found. Host security nevertheless does not meet the report's unqualified FIXED claim.

## 17. Storage integrity

Atomic import batches, file generations, root ownership, stable cost storage as decimal text, and per-scope LKG retention are sound improvements.

Critical exception: migration 006's blanket reassignment of costs/snapshots is a data-integrity regression (§5). In addition, Unix permission evidence is incomplete:

- `harden_db_permissions` attempts 0600 for DB/WAL/SHM after migration but ignores all errors (`storage/lib.rs`, lines 123–142);
- the test `database_file_is_user_only` checks only the main DB (`lines 2226–2235`), not `-wal` or `-shm`;
- sidecars can be recreated later, and the implementation relies on an unverified assumption that the parent app-data directory is user-only.

Thus WAL/SHM user-only semantics are not proven by the current test suite.

## 18. Runtime/concurrency

Coalescing and RAII waiter release are implemented correctly and specifically tested. Different account scopes remain parallel. Persistent rate-limit cooldown restore is correct. Import holds the storage mutex only around discrete storage operations rather than during file reads.

The remaining concurrency/lifecycle risk is the absent-token cancellation race: disable/delete does not install a cancelled tombstone when no token exists. HTTP cancellation also stops at the headers boundary. No evidence of an inflight-map deadlock after leader abortion was found.

## 19. UI/SWR/identity

Current UI shows old metrics together with stale/current error state and timestamps, and local unknown identity is labelled honestly. `Connected` from the old snapshot is overwritten by the latest source error in DTO adaptation. Navigation keys include provider/product/account.

Test caveat: the DOM test is handcrafted rather than an `App.svelte` render. Also the provider card calls `formatAccountLabel(provider.alias)` without identity confidence (`App.svelte`, line 347), which treats every generic alias as unknown; this is conservative for local identity but could rename a deliberately generic confirmed/API alias. It is not the core P0 failure.

## 20. Test-quality review

| Guarantee / key test | Rating | Reason |
|---|---|---|
| Root isolation runtime test | Strong | Real temp roots → parser → provenance → SQLite attribution |
| Same-root rebind | Strong | Models exact A then B ownership conflict |
| Legacy migration | Weak | Only v1 Codex usage; omits costs, snapshots, v5, multi-account, embedded payload |
| Replacement | Strong | Real file replacement, final total and generation asserted |
| Atomic import fault | Strong | Failure injected after row writes inside transaction; state checked afterward |
| OpenAI pagination/partial | Strong for strategy | Counts calls and preserves payload/error; uses mock transport |
| OpenAI decimal precision | Weak / false-positive | Only short decimals that survive shortest-float rendering |
| Coalescing | Strong | Concurrent public `refresh_many`; physical calls counted |
| Leader cancellation | Moderate/strong | Correct behavior, but sleep-based scheduling |
| Restart cooldown | Strong | New coordinator, shared persistent store, fake clock |
| SWR runtime | Strong | Actual coordinator/storage fallback |
| SWR DTO | Moderate | Direct adapter test, not command/component integration |
| SWR DOM | Weak | Tests a manually authored HTML string, not `App.svelte` |
| HostFacade policy | Moderate | Shape assertions; does not exercise real production HTTP/keychain |
| Provider OS bypass tripwire | Moderate | Useful lexical guard, limited to provider sources |
| Owned child cancellation | Strong for ownership | Cancels a spawned child handle and checks tracking |
| DB permissions | Weak | Main file only; WAL/SHM untested |
| Codex/Claude semantics | Weak for real semantics | Synthetic/provenance-unknown fixtures; required corpus absent |

## 21. New regressions

1. **Critical — wrong account attribution during migration:** migration 006 moves every cost and snapshot for a product to a legacy account, including previously trustworthy API-account data.
2. **High — production OpenAI responses effectively capped at 1 KiB:** default transport configuration contradicts the 2 MiB request policy and is bypassed by mocks.
3. **High — exact money claim is test-shaped:** JSON numbers are converted through the default binary representation before `Decimal`.
4. **Medium — HTTP limits/cancellation are incomplete:** body is buffered before size check and outside cancellation/timeout.
5. **Medium — disable/delete race:** cancellation does not persist if called before token creation.
6. **Medium — Codex over-scan:** `*sessions` is broader than `sessions` plus `archived_sessions`.
7. **Medium — WAL/SHM permissions unproven:** ignored chmod failures and no sidecar assertions.
8. **Low/medium — process output handling:** pipes are drained only after wait and capped only after full buffering.

No new provider integration was found, and no direct credential logging or filesystem escape was found.

## 22. Master Spec P0 matrix

The Master Spec numbering differs from the remediation issue labels. This table follows Master Spec §36 exactly.

| Master criterion | Status | Evidence |
|---|---|---|
| P0-1 OpenAI one-fetch | ✅ FIXED | One paginated strategy pass feeds snapshot and costs; call-count tests |
| P0-2 SWR | ✅ FIXED | LKG values + stale + current error metadata through DTO/UI |
| P0-3 Demo integrity | ✅ FIXED | Packaged Tauri path propagates invoke failure; demo only when no Tauri runtime (`api.ts`, lines 26–36) |
| P0-4 Codex semantics | ⛔ BLOCKED_BY_EXTERNAL_EVIDENCE | Master explicitly requires redacted real fixtures; only ignored probe exists |
| P0-5 Claude semantics | ⛔ BLOCKED_BY_EXTERNAL_EVIDENCE | Synthetic fixtures have no provenance; exact real schema/cumulative semantics not proven |
| P0-6 Runtime decomposition | ✅ FIXED | Tauri refresh flow reduced 776→475 lines; orchestration/root resolution/aggregation moved into runtime and used in production |
| P0-7 CI | ✅ FIXED | strict workspace all-target Clippy and app crate in CI; current run green |
| P0-8 Runtime integration tests | 🟡 PARTIAL | Good runtime/storage and separate DTO tests exist, but no single fake-host refresh→parse→transaction→DTO test; production HTTP defects are mock-hidden |

Gate A cannot be passed while Master P0-4 and P0-5 are blocked: the exact wording is “Done when” real redacted Codex fixtures prove semantics and Claude duplicate/cache semantics are proven (`MASTER_SPEC`, lines 1657–1663), and the gate requires all criteria before expansion (lines 184–194 and 1641–1644). More importantly, this checkout also has code-level data integrity and Host defects, so verdict B is not available.

Remediation issue matrix:

| Issue | Previous status | Current status | Code/test summary | Confidence |
|---|---|---|---|---|
| B-01 | Blocker | ⚠️ REGRESSION / NEW RISK | New root isolation good; migration 006 corrupts cost/snapshot owner | High |
| B-02 | Blocker | ✅ FIXED (with honest limitation) | Unknown bucket, rejection, label; no real switch observation | High |
| P0-01 | P0 | ✅ FIXED | Generational transactional rebuild | High |
| P0-02 | P0 | ⚠️ REGRESSION / NEW RISK | Pagination fixed; money and real HTTP path fail strict audit | High |
| P0-03 | P0 | ✅ FIXED | Pre-semaphore registration + RAII + public concurrency tests | High |
| P0-04 | P0 | ✅ FIXED | One transaction and post-write fault injection | High |
| P0-05 | P0 | 🟡 PARTIAL | Safe/unverified parser, missing real corpus, broad wildcard | High |
| P0-06 | P0 | ⛔ BLOCKED_BY_EXTERNAL_EVIDENCE | Synthetic reducer/cache behavior only | High |
| P0-07 | P0 | ✅ FIXED | Full behavior visible; component test weak | Moderate |
| P0-08 | P0 | 🟡 PARTIAL | Facade shape good; transport/lifecycle enforcement incomplete | High |
| P0-09 | P0 | ✅ FIXED | Fresh coordinator + persisted cooldown + fake clock | High |

## 23. Final verdict

# D — DATA INTEGRITY / SECURITY RISK

Gate A is **not passed**. Provider expansion is unsafe.

The decisive D-level finding is the reproducible migration path that silently reassigns existing OpenAI API costs and snapshots to a generated legacy account (`migrate_legacy_rows`, lines 1380–1457). This can produce wrong per-account totals and remove the original account's LKG view. In addition, production HTTP configuration caps responses at 1 KiB while tests bypass the real host, and monetary JSON parsing does not meet the required no-binary-roundtrip invariant.

Verdict B is not applicable because missing real fixtures are not the only blockers; code-level integrity and enforcement defects remain. Green build/test/Clippy results do not cover these paths.

**Provider expansion remains frozen until the code-level findings are remediated and evidence validation is complete, unless Master Spec is explicitly amended.**

## 24. Exact next actions

Minimum next actions, in order:

1. Correct migration 006 so only genuinely ambiguous local rows move to unknown identity; preserve trustworthy API costs/snapshots and embedded snapshot account identity. Add realistic v5 multi-account migration coverage for usage, costs, snapshots, and LKG lookup.
2. Correct production HTTP default/effective size limits; stream with a hard byte cap and race both send and body consumption against timeout/cancellation. Exercise `ReqwestHttpHost`, not only `ScriptHttp`.
3. Parse OpenAI monetary values from preserved lexical decimal text (or an equivalent exact deserializer), with precision-sensitive tests through SQLite readback.
4. Make disable/delete install a cancelled account token even before work starts; add the cancel-before-first-refresh/queued-work ordering test.
5. Replace the Codex wildcard with exact allowed roots/patterns and add negative over-scan coverage.
6. Test DB, WAL, and SHM modes and propagate hardening failures; verify the parent data-directory policy.
7. Render the real Svelte component in the SWR DOM test and add one fake-host refresh→transaction→DTO integration path.
8. Only after code gates are clean, validate the external evidence below and rerun this audit from a committed, clean revision.

## 25. Required external evidence

### Codex

Collect a minimal real/redacted macOS JSONL corpus containing only usage/schema fields needed to prove behavior:

- envelope timestamp, event type, and source-provided request/ordinal identity if present;
- `payload.type = token_count`;
- `last_token_usage`: input, cached input, output, reasoning output, total;
- cumulative `total_token_usage` only where present and needed to distinguish delta versus cumulative;
- minimal rate-limit structure: limit ID, primary/secondary percentages, window minutes, reset timestamp, plan type;
- optional model field only if it is a non-personal model identifier.

Include active and archived sessions, duplicates, a growing file, partial trailing line, truncation/replacement, and a hand-reviewed expected-total manifest. Record client/app version, capture date, whether the file is real-redacted, redaction procedure, and reviewer—without identifying the user.

### Claude

Collect minimal real/redacted multi-turn/streaming records containing only:

- timestamp;
- envelope UUID, `requestId`, `message.id` needed for identity analysis;
- model identifier;
- usage input/output, cache-read, and cache-creation counters;
- enough ordered and out-of-order repeated chunks to establish whether counters are cumulative, delta, or final snapshots;
- a hand-reviewed expected-total/cache manifest and Claude Code version.

### Safe redaction procedure

Perform redaction locally before the artifact enters the repository. Parse JSON and build a strict allowlist object; do not regex-copy entire lines. Replace necessary identifiers with deterministic opaque values so dedup relations survive. Drop prompts, assistant content, tool inputs/outputs, code, paths, working directories, emails, organization/project names, API/session/auth tokens, credentials, headers, environment values, and unrelated metadata. Run the existing forbidden-field/secret scan plus manual review. Add a provenance README stating source class (`real-redacted`), client version, allowed fields, removed field classes, expected totals, redactor version, and reviewer. Never check in the raw source corpus.
