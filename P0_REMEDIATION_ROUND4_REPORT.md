# P0 Remediation Round 4 Report

## 1. Metadata

- **Remediation Round**: Round 4 (Narrow Focus Remediation).
- **Date**: 2026-09-09.
- **Baseline Git Commit**: `main` / `d99554e289c40df6a393fb8d5ad71132ff353b1d` (dirty working tree).
- **Normative Document**: `Usage.ai_MASTER_SPEC_v1.1.md`.
- **Preceding Audit**: `TECHNICAL_AUDIT_P0_ROUND3_FINAL.md` (Verdict: `D — DATA INTEGRITY / SECURITY RISK`).
- **Target Verdict**: `B — CODE GATE PASSED / EXTERNAL EVIDENCE REQUIRED`.

---

## 2. Baseline

The preceding audit (`TECHNICAL_AUDIT_P0_ROUND3_FINAL.md`) found a reproducible production code path to **misleading authoritative metrics**, resulting in verdict `D`:
1. **Tray Metric Leak**: `src-tauri/src/refresh_flow.rs::primary_remaining` computed the minimum quota across all fresh providers without checking coverage. A local provider with `Fresh + Connected + Coverage::UnverifiedSemantics` and `remaining_percent = 20` caused the native menu-bar tray title to display `20%` as an ordinary authoritative metric without disclaimer.
2. **Notification Coverage Drop**: `src-tauri/src/commands/notifications.rs` converted `ProviderDto` to `ProviderNoticeView`, completely discarding `coverage`. In `crates/usage-runtime/src/notifications.rs`, the notification planner evaluated fresh views without coverage awareness, emitting authoritative threshold alerts (`Использовано 80% · осталось 20%`), pace warnings, and reset notifications for unverified data.
3. **Redactor Whole-File Memory Buffering**: `scripts/redact-codex.mjs` and `scripts/redact-claude.mjs` used `readFileSync` to read entire JSONL files into memory without streaming or input/output size caps.
4. **WAL/SHM Sidecar Test**: The storage hardening test tested only the `-wal` sidecar conditionally, without asserting both `-wal` and `-shm` sidecars unconditionally.

---

## 3. Scope

In accordance with strict Round 4 instructions:
- **Scope is strictly narrow**: Fixed only tray isolation, notification coverage propagation and suppression, redactor streaming with resource bounds, and sidecar test strengthening.
- **No new providers added**: No Antigravity, Z.ai, OpenCode/Zen/Go, ChatGPT consumer, claude.ai, Gemini API, or Codex App Server.
- **No architecture rewrites**: Retained existing crates, dependencies, and interfaces.
- **No scraping of personal logs**: No real user logs from `~/.codex` or `~/.claude` were collected or processed.
- **P0 Gate A not declared**: Marked P0-05 and P0-06 as `BLOCKED_BY_EXTERNAL_EVIDENCE`.

---

## 4. Coverage Trust Policy

In `crates/usage-core/src/models.rs`, authoritative and non-authoritative semantics are formalized:

```rust
impl Coverage {
    /// Returns true if this coverage represents verified, authoritative data.
    /// Only `Complete` and `Partial` are authoritative; `UnverifiedSemantics`,
    /// `LocalClientOnly`, and `Unknown` are non-authoritative.
    pub fn is_authoritative(&self) -> bool {
        matches!(self, Self::Complete | Self::Partial)
    }

    /// Whether this coverage permits ordinary display as a primary numeric tray metric.
    pub fn allows_tray_metric(&self) -> bool {
        self.is_authoritative()
    }

    /// Whether this coverage permits native authoritative quota/budget notifications.
    pub fn allows_quota_notifications(&self) -> bool {
        self.is_authoritative()
    }
}
```

---

## 5. Tray Metric Fix

In `src-tauri/src/refresh_flow.rs`, `primary_remaining` now strictly requires authoritative coverage:

```rust
pub fn primary_remaining(providers: &[ProviderDto]) -> Option<f64> {
    providers
        .iter()
        .filter(|p| p.freshness == FreshnessState::Fresh && p.coverage.allows_tray_metric())
        .flat_map(|p| p.quotas.iter())
        .map(|q| q.remaining_percent)
        .fold(None, |acc: Option<f64>, v| match acc {
            None => Some(v),
            Some(curr) => Some(curr.min(v)),
        })
}
```

**Effects**:
- A snapshot with only unverified quota (e.g. Codex 20%) returns `None`. The tray title does not display `20%`.
- In a mixed snapshot (Verified 63%, Unverified 20%), the tray selects `63%` from the verified provider and completely ignores the unverified 20%.

---

## 6. Notification Coverage Propagation & Suppression

1. **`ProviderNoticeView` in `crates/usage-runtime/src/notifications.rs`**:
   - Added `pub coverage: Coverage` field.
   - In `Planner::plan_provider`:
     ```rust
     if !view.coverage.allows_quota_notifications() {
         return out;
     }
     ```
   - Resets, pace warnings, and threshold alerts (100%, 90%, 80%, critical) are strictly suppressed for unverified semantics.

2. **`src-tauri/src/commands/notifications.rs`**:
   - Propagates `coverage: p.coverage` from `ProviderDto` into `ProviderNoticeView`.
   - In `deliver_budget_notices`: added `&& r.coverage.is_authoritative()` filter so budget alerts never fire on unverified records.

---

## 7. Side-Effect Regression Tests

### `src-tauri/src/refresh_flow.rs`
- **`tray_metric_selection_cases_a_b_c_d`**:
  - Case A (Verified 20%): returns `Some(20.0)`.
  - Case B (Unverified 20%): returns `None`.
  - Case C (Verified 63%, Unverified 20%): returns `Some(63.0)`.
  - Case D (Only Unverified 15%): returns `None`.
- **`full_side_effect_regression_unverified_semantics_produces_no_authoritative_side_effects`**:
  - Evaluates a `Fresh + Connected + UnverifiedSemantics + 20% remaining` snapshot across both tray and notification evaluator.
  - Asserts `primary_remaining(...) == None`.
  - Asserts notification planner generates `0` notices.

### `crates/usage-runtime/src/notifications.rs`
- **`unverified_low_quota_and_critical_thresholds_are_suppressed`**: Unverified quota with remaining 20% and 5% triggers 0 threshold notifications.
- **`unverified_pace_and_reset_are_suppressed`**: Unverified quota with pace spike and window jump triggers 0 notifications.
- **`mixed_providers_verified_fires_while_unverified_suppressed`**: Verified provider fires 80% threshold notice, while unverified provider in same run is suppressed.

---

## 8. Streaming Redactor Implementation

Both `scripts/redact-codex.mjs` and `scripts/redact-claude.mjs` were converted from `readFileSync` whole-file buffers to line-by-line Node.js streams:
- `createReadStream(inputFile, { encoding: 'utf8' })` piped through `readline.createInterface({ crlfDelay: Infinity })`.
- Output written via `createWriteStream(tempOutputFile)` with backpressure handling (`once(outputStream, 'drain')`).
- **Atomic File Write**: Outputs to `${outputFile}.tmp.${process.pid}.${Date.now()}` and atomically renames via `renameSync` only after `finish` event.
- **Fail-Closed Cleanup**: On any stream or parsing error, the temp file is deleted via `unlinkSync`, preventing partial or corrupted outputs.
- Zero calls to `readFileSync` remain in `scripts/`.

---

## 9. Redactor Resource Limits

Strict resource bounds are enforced:
- `MAX_INPUT_BYTES = 50 * 1024 * 1024` (50 MiB): Inspected via `statSync(inputFile)` before opening stream; rejects oversized files fail-closed.
- `MAX_OUTPUT_BYTES = 50 * 1024 * 1024` (50 MiB): Tracked on write; aborts and cleans up temp file if exceeded.
- `MAX_DISTINCT_IDS = 50,000`: Context `mapId` tracks distinct seen identifiers and throws if limit exceeded, preventing memory exhaustion.

Covered by comprehensive integration tests in `src/lib/redact.test.ts`:
- Input size cap rejection test (fails before processing).
- Output size cap abort and cleanup test (verifies 0 leftover temp files).
- Max distinct IDs limit enforcement test.
- End-to-end streaming output and metadata file generation test.

---

## 10. WAL/SHM Sidecar Hardening Test

In `crates/usage-storage/src/lib.rs`, `wal_and_shm_sidecars_are_hardened_and_fail_closed_if_insecure` was strengthened:
- Unconditionally creates **both** `usage.db-wal` and `usage.db-shm` with permissive `0o666` permissions.
- Runs `SystemPermissionHardener::harden_permissions`.
- Unconditionally asserts that **both** files are hardened to `0o600` (`mode & 0o777 == 0o600`).

---

## 11. Tests Added

| Area | File | Test Name | Target Behavior |
| --- | --- | --- | --- |
| Tray | `src-tauri/src/refresh_flow.rs` | `tray_metric_selection_cases_a_b_c_d` | Excludes unverified quotas from tray metric |
| Side-Effects | `src-tauri/src/refresh_flow.rs` | `full_side_effect_regression_unverified_semantics_produces_no_authoritative_side_effects` | E2E isolation of tray & notifications for unverified data |
| Notifications | `crates/usage-runtime/src/notifications.rs` | `unverified_low_quota_and_critical_thresholds_are_suppressed` | Suppresses 80%/critical threshold notices |
| Notifications | `crates/usage-runtime/src/notifications.rs` | `unverified_pace_and_reset_are_suppressed` | Suppresses pace and reset notices |
| Notifications | `crates/usage-runtime/src/notifications.rs` | `mixed_providers_verified_fires_while_unverified_suppressed` | Verified provider notifies while unverified is muted |
| Redactor | `src/lib/redact.test.ts` | `streams codex file line-by-line and atomically writes output and metadata` | Streaming Codex CLI + atomic rename |
| Redactor | `src/lib/redact.test.ts` | `streams claude file line-by-line and atomically writes output and metadata` | Streaming Claude CLI + atomic rename |
| Redactor | `src/lib/redact.test.ts` | `rejects input file exceeding maxInputBytes before processing (Codex and Claude)` | 50 MiB input bound fail-closed |
| Redactor | `src/lib/redact.test.ts` | `fails closed and removes temp file if output exceeds maxOutputBytes` | 50 MiB output bound + temp cleanup |
| Redactor | `src/lib/redact.test.ts` | `enforces maxDistinctIds bound in context` | 50,000 distinct IDs cap |
| Storage | `crates/usage-storage/src/lib.rs` | `wal_and_shm_sidecars_are_hardened_and_fail_closed_if_insecure` | Both WAL and SHM sidecars hardened to 0600 |

---

## 12. Verification Gates Results

All six mandatory verification gates were run and passed cleanly:

| Gate | Status | Details |
| --- | --- | --- |
| `npm run check` | **PASS** | 0 errors, 0 warnings; version consistency `1.0.0` PASS |
| `npm test -- --run` | **PASS** | 5 test files, 47 passed (0 failed) |
| `npm run build` | **PASS** | Vite production bundle generated (144 modules) |
| `cargo check --workspace` | **PASS** | All crates compiled without errors |
| `cargo test --workspace` | **PASS** | 156 passed, 0 failed, 1 ignored (opt-in external probe) |
| `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** | 0 warnings across all targets |

### Test Breakdown by Crate / Suite
- `src-tauri` (`usage-ai-app`): 4 passed
- `crates/usage-core`: 11 passed
- `crates/usage-host`: 17 passed
- `crates/usage-providers`: 39 passed, 1 ignored (`real_codex_corpus_probe_is_opt_in_and_never_checked_in`)
- `crates/usage-runtime`: 57 passed
- `crates/usage-storage`: 28 passed
- **Total Rust Tests**: **156 passed**, 1 ignored
- **Total Frontend Tests**: **47 passed** across 5 files (`ui.test.ts`, `redact.test.ts`, `format.test.ts`, `snapshot.test.ts`, `App.test.ts`)

---

## 13. Regression Checks

Regression scans confirm no unintended bypasses:
1. `git grep -n "primary_remaining" crates src-tauri`: Verified that `tray.rs` calls `primary_remaining`, which filters via `p.coverage.allows_tray_metric()`.
2. `git grep -E "readFileSync|readFile\(" scripts/`: Returned **0 matches**. All redactor scripts use streaming.
3. `git grep -n "UnverifiedSemantics" crates src-tauri src`: Verified consistent quarantine across storage SQL queries, UI badges, tray metric, and notification planner.

---

## 14. Remaining Limitations & Blockers

- **P0-05 (Codex Real Semantics)**: `BLOCKED_BY_EXTERNAL_EVIDENCE`.
- **P0-06 (Claude Real Semantics)**: `BLOCKED_BY_EXTERNAL_EVIDENCE`.

With Round 4 complete:
- The streaming redactors with value allowlists and resource bounds are fully secured and ready to collect real evidence safely.
- No real user logs or sessions are included in the repository.
- Until real evidence is collected and audited, Codex and Claude local history remains strictly quarantined as `Coverage::UnverifiedSemantics` with zero authoritative side effects.

---

## 15. Self-Audit Table

| Audit Finding | Previous Status | Round 4 Remediation | Verification Evidence |
| --- | --- | --- | --- |
| Menu-bar tray shows 20% for unverified quota | Leaked ordinary percentage | Filtered out in `primary_remaining` | Unit & E2E tests assert `None` |
| Native notification emits threshold/pace/reset for unverified quota | Dropped coverage in NoticeView | `coverage` field added; notices suppressed | 3 notification tests assert 0 alerts |
| Stored aggregates vs side-effects discrepancy | Storage isolated, side-effects leaked | Both storage and side-effects isolated | Comprehensive side-effect regression tests pass |
| Redactor memory buffering (`readFileSync`) | Whole file buffered | Converted to streaming pipeline | Zero `readFileSync`; streaming tests pass |
| Redactor unbounded input/output | No bounds | 50 MiB input/output caps, 50k ID cap | Limit rejection and cleanup tests pass |
| WAL/SHM sidecar test strength | WAL only tested | Unconditionally tests both WAL and SHM | `wal_and_shm_sidecars_are_hardened` passes |

---

## 16. Readiness for Independent Audit

Ready for independent P0 Gate Re-Audit: **YES**

The code gate blocker has been cleared. The system is ready for independent evaluation targeting Verdict **B — CODE GATE PASSED / EXTERNAL EVIDENCE REQUIRED**.
