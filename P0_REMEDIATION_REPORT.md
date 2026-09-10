# P0 Remediation Report: Usage.ai

## Metadata

* **Baseline Revision**: `main / d99554e289c40df6a393fb8d5ad71132ff353b1d`
* **Final Working Tree Status**: Dirty (remediation branch containing targeted fixes across runtime, providers, storage, host, and UI shell)
* **Initial Verdict**: `D — DATA INTEGRITY RISK`
* **Files Changed**:
  * `Cargo.lock`
  * `package.json`
  * `package-lock.json`
  * `src-tauri/tauri.conf.json`
  * `README.md`
  * `docs/INTEGRATION_MATRIX.md`
  * `docs/TECHNICAL_AUDIT_V1.1_IMPLEMENTATION.md`
  * `docs/integrations/OpenAI-Codex-Integration-Spec.md`
  * `crates/usage-core/src/models.rs`
  * `crates/usage-host/src/lib.rs`
  * `crates/usage-host/src/cancel.rs`
  * `crates/usage-host/src/facade.rs`
  * `crates/usage-host/src/files.rs`
  * `crates/usage-host/src/http.rs`
  * `crates/usage-host/src/process.rs`
  * `crates/usage-providers/Cargo.toml`
  * `crates/usage-providers/src/lib.rs`
  * `crates/usage-providers/src/descriptor.rs`
  * `crates/usage-providers/src/strategy.rs`
  * `crates/usage-providers/src/codex.rs`
  * `crates/usage-providers/src/claude.rs`
  * `crates/usage-providers/src/openai_api.rs`
  * `crates/usage-runtime/src/lib.rs`
  * `crates/usage-runtime/src/account_router.rs`
  * `crates/usage-runtime/src/aggregation.rs` (new)
  * `crates/usage-runtime/src/history_import.rs`
  * `crates/usage-runtime/src/orchestration.rs` (new)
  * `crates/usage-runtime/src/refresh.rs`
  * `crates/usage-runtime/src/registry.rs`
  * `crates/usage-runtime/src/root_resolver.rs` (new)
  * `crates/usage-storage/src/lib.rs`
  * `crates/usage-storage/migrations/006_legacy_identity.sql` (new)
  * `crates/usage-storage/migrations/007_claude_cache_buckets.sql` (new)
  * `src-tauri/src/commands/accounts.rs`
  * `src-tauri/src/composition.rs`
  * `src-tauri/src/dto.rs`
  * `src-tauri/src/platform/hosts.rs`
  * `src-tauri/src/refresh_flow.rs`
  * `src/App.svelte`
  * `src/styles.css`
  * `src/lib/api.ts`
  * `src/lib/demo.ts`
  * `src/lib/types.ts`
  * `src/lib/ui.ts`
  * `src/lib/ui.test.ts`
  * `scripts/check-version.mjs` (new)
  * `crates/usage-providers/fixtures/codex/`
  * `crates/usage-providers/fixtures/claude/`

---

## Verification

| Command | Result | Tests Count / Details |
|---|---|---|
| `npm run check` | **PASS** | `svelte-check`: 0 errors, 0 warnings; version consistency: 1.0.0 |
| `npm test -- --run` | **PASS** | 3 test files, 13 tests passed |
| `npm run build` | **PASS** | Vite v7.3.6 production bundle built successfully |
| `cargo check --workspace` | **PASS** | Clean check across all crates and targets |
| `cargo test --workspace` | **PASS** | 138 tests passed (usage-host: 31, usage-ai-lib: 7, usage-core: 6, usage-providers: 35 passed, 1 ignored; usage-runtime: 41 passed; usage-storage: 18 passed) |
| `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** | 0 warnings, strict deny enforced |

---

## Issue Matrix

| Issue | Previous State | Current State | Evidence | Tests | Status |
|---|---|---|---|---|---|
| **B-01** (Root/account isolation) | Default and custom roots shared records; same root could be rebound; legacy rows lived in default account | Separate connections with explicit hashes; same root cannot be rebound between accounts; legacy rows isolated into "Legacy / Identity unknown" account with `IdentityConfidence::Unknown` | `crates/usage-storage/src/lib.rs`, `crates/usage-runtime/src/history_import.rs` | `same_root_cannot_silently_rebind_between_accounts`, `migrates_real_v1_database_without_data_loss`, `default_and_custom_roots_never_share_rows` | **FIXED** |
| **B-02** (Honest local identity) | Local Codex/Claude accounts fabricated identity or displayed misleading "Аккаунт 1" | Stored `IdentityConfidence::Unknown`, label "Локальная история" / "Аккаунт не определён"; unattributed rows rejected by confirmed accounts; unconfigured UI cards display "Локальная история" | `crates/usage-runtime/src/account_router.rs`, `src/lib/ui.ts`, `src/App.svelte` | `synthetic_aba_sequence_rejects_and_recovers_without_silent_merge`, `confirmed_connection_rejects_unattributable_rows`, `honest account labels` UI vitest | **FIXED** |
| **P0-01** (Replacement runtime proof) | File rewrite caused cumulative total inflation; test failed compilation with `E0515` | Byte-offset and file generation tracking deletes prior generation rows atomically inside the batch transaction; totals reflect only generation 2 | `crates/usage-runtime/src/history_import.rs`, `crates/usage-storage/src/lib.rs` | `replacement_rebuilds_totals_without_inflation`, `replacement_rebuilds_old_generation_rows_atomically` | **FIXED** |
| **P0-02** (Partial OpenAI pagination) | Page 2 failure discarded Page 1; float `f64` rounding risk | Page 1 records committed; `Coverage::Partial` returned; current error preserved; exact `Decimal` parsing from lexical string | `crates/usage-providers/src/openai_api.rs` | `page_two_network_error_preserves_partial_result`, `malformed_page_two_preserves_partial_result`, `decimal_amounts_stay_exact_without_float_roundtrip` | **FIXED** |
| **P0-03** (Production coalescing & Inflight RAII) | Inflight registration raced semaphore; dropping a leader future leaked the registration, deadlocking subsequent waiters | Scope-level coalescing registers inflight watch channel before semaphore wait; RAII `InflightGuard` cleans up inflight channel on drop or failure | `crates/usage-runtime/src/refresh.rs` | `production_triggers_coalesce_into_one_execution`, `different_scopes_run_in_parallel`, `leader_drop_cleans_up_inflight_and_allows_subsequent_refresh` | **FIXED** |
| **P0-04** (Atomic import batch) | Mid-import failures could commit partial rows without checkpoint | Storage and runtime batch transactions wrap usage rows, source files, and checkpoints atomically; fault injection rolls back all | `crates/usage-runtime/src/history_import.rs`, `crates/usage-storage/src/lib.rs` | `runtime_import_fault_rolls_back_rows_checkpoint_and_generation`, `failed_import_batch_leaves_db_unchanged`, `batch_import_is_atomic_and_attempts_persist` | **FIXED** |
| **P0-05** (Codex fixture evidence & Sibling discovery) | Synthetic fixtures; unproven semantics; single root hardcoded; sibling `archived_sessions` directory not glob-scanned | `CODEX_HOME` unified across discovery and history import; `*sessions/**` glob discovers both `sessions` and `archived_sessions`; coverage explicitly marked `Coverage::UnverifiedSemantics` | `crates/usage-providers/src/codex.rs`, `crates/usage-providers/src/descriptor.rs`, `crates/usage-runtime/src/root_resolver.rs` | `custom_path_accepts_product_home_and_sessions_dir`, `custom_child_directory_normalizes_from_descriptor_glob`, `archived_sessions_sibling_is_discovered_and_imported`, `window_kind_stays_unknown_without_proven_semantics` | **BLOCKED_BY_EXTERNAL_EVIDENCE** |
| **P0-06** (Claude semantics) | Out-of-order streaming chunks could regress usage; cache create/read conflated | Streaming reducer enforces monotonicity; cache creation/read segregated without double counting; `CLAUDE_CONFIG_DIR` supported | `crates/usage-providers/src/claude.rs`, `crates/usage-storage/src/lib.rs` | `late_older_chunk_does_not_regress_final_usage`, `cache_creation_adds_to_cached_not_total`, `late_claude_chunk_preserves_monotonic_totals_and_cache_buckets` | **BLOCKED_BY_EXTERNAL_EVIDENCE** |
| **P0-07** (SWR current error visibility & card DOM) | SWR served last-known-good (LKG) snapshot but lost current failure, showing misleading healthy state; DOM state unverified | Snapshot serves LKG with `freshness: Stale`; DTO explicitly sets `Freshness::Stale(0)` when stale; UI renders stale banner with error code and timestamps; full card DOM verified in Vitest | `src-tauri/src/refresh_flow.rs`, `crates/usage-runtime/src/refresh.rs`, `src/lib/ui.ts`, `src/lib/ui.test.ts`, `src/App.svelte` | `e2e_codex_refresh_then_failure_serves_last_known_good`, `failure_serves_stale_good_data`, `renders stale provider card DOM with LKG metrics, warning banner, and honest label` UI vitest | **FIXED** |
| **P0-08** (HostFacade boundary & Process cancellation) | Providers held direct or unrestricted access to host resources; killed process children unasserted | Runtime constructs `HostFacade` derived from product descriptor; `in_flight_cancellation_kills_owned_child` proves child termination; raw OS access tripwire test | `crates/usage-host/src/facade.rs`, `crates/usage-host/src/process.rs`, `crates/usage-providers/src/lib.rs`, `crates/usage-runtime/src/registry.rs` | `local_product_gets_files_only`, `api_product_gets_http_and_scoped_keychain_only`, `facade_policy_matches_descriptor_model`, `production_code_has_no_direct_os_access`, `in_flight_cancellation_kills_owned_child` | **FIXED** |
| **P0-09** (Restart cooldown survival) | Cooldown state was volatile, re-querying rate-limited providers upon restart | Persisted cooldowns restored lazily upon scope refresh across coordinator lifecycles; verified via fake clock without real sleep | `crates/usage-runtime/src/refresh.rs` | `persisted_cooldown_survives_coordinator_restart_without_real_sleep`, `rate_limit_cooldown_blocks_even_manual_refresh` | **FIXED** |

---

## Remaining Limitations & External Evidence Requirements

1. **Real Account Identity on Local Sources**:
   * Local session transcripts (Codex and Claude JSONL) do not expose stable account credentials or authenticated user email.
   * As required by B-02, these are honestly presented as `IdentityConfidence::Unknown` under "Локальная история". They cannot be automatically associated with verified API accounts.

2. **Codex Evidence Gap (P0-05)**:
   * Status: `BLOCKED_BY_EXTERNAL_EVIDENCE`.
   * The codebase explicitly tags all local Codex metrics as `Coverage::UnverifiedSemantics`.
   * **Required Fields for Evidence Confirmation**:
     To upgrade Codex from `BLOCKED_BY_EXTERNAL_EVIDENCE` to `FIXED`, an anonymized real transcript corpus must be captured on macOS containing exclusively:
     - `timestamp`: ISO-8601 string
     - `type`: `"event_msg"`
     - `payload.type`: `"token_count"`
     - `payload.info.last_token_usage`: `{input_tokens, cached_input_tokens, output_tokens, reasoning_output_tokens, total_tokens}`
     - `payload.info.total_token_usage`: cumulative session tokens
     - `payload.rate_limits`: `{limit_id, primary: {used_percent, window_minutes, resets_at}, secondary, plan_type}`
     All user prompts, model responses, code blocks, paths, emails, and credentials must be stripped.

3. **Claude Evidence Gap (P0-06)**:
   * Status: `BLOCKED_BY_EXTERNAL_EVIDENCE`.
   * Reducer heuristics handle out-of-order and duplicate chunks monotonically with separate cache buckets.
   * To confirm exact source semantics without heuristics, verified multi-turn anonymized message chunks from official Claude Code releases are required.

4. **Private Providers**:
   * Unimplemented discovery products (Antigravity, Z.ai, OpenCode, ChatGPT Consumer, claude.ai, Gemini API, Codex App Server) remain blocked and visible as `UnsupportedSource` / `AccountModel::DiscoveryOnly`. No private unofficial endpoints have been introduced.

---

## Ready for Audit

Ready for independent P0 Gate Audit: **YES**

*(All workspace verification gates are GREEN; the implementation resolves all architectural and data-integrity blockers within the scope of verifiable evidence, honestly marks unresolved external evidence as BLOCKED_BY_EXTERNAL_EVIDENCE, and does not unilaterally declare P0 Gate passed).*
