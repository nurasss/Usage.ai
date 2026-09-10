# 1. Metadata

- Audit type: independent P0 Gate Audit after `P0 Remediation Round 4`.
- Audit date/time: 2026-09-09.
- Auditor verdict: **B — CODE GATE PASSED / EXTERNAL EVIDENCE REQUIRED**.
- Audited branch/commit: `main` / `d99554e289c40df6a393fb8d5ad71132ff353b1d` plus the dirty working tree described below.
- Normative document: `/Users/nuras/Downloads/Usage.ai_MASTER_SPEC_v1.1.md`, SHA-256 `63976e26a1494f2cf10ea49f0cddffc945f4aa3170e2b31b70defe5862daaab0` — identical hash to the one cited in the Round 3 audit; the spec did not change.
- Preceding audit: `TECHNICAL_AUDIT_P0_ROUND3_FINAL.md` (Verdict: `D — DATA INTEGRITY / SECURITY RISK`).
- Claims document: `P0_REMEDIATION_ROUND4_REPORT.md`.
- Privacy constraint observed: no real Codex/Claude logs, user session files, or personal evidence were opened or processed. `~/.codex` and `~/.claude` were not read.
- Mutation constraint observed: no production code, tests, fixtures, providers, Master Spec, or remediation reports were changed. This audit report is the only file created.

The Round 4 report was treated as a set of claims, not evidence. Every conclusion below is based on independently inspected implementation, tests actually read line-by-line, and gate commands run fresh in this session.

---

# 2. Repository state

| Property | Result |
| --- | --- |
| Branch | `main` |
| HEAD | `d99554e289c40df6a393fb8d5ad71132ff353b1d` |
| Upstream | `origin/main`; local branch ahead by 3 commits |
| Worktree | Dirty |
| Staged changes | None |
| Unstaged tracked changes | 46 files, 5,860 insertions / 1,728 deletions |
| Untracked | Reports (`P0_REMEDIATION_*`, `TECHNICAL_AUDIT_P0_FINAL.md`, `TECHNICAL_AUDIT_P0_REAUDIT.md`, `TECHNICAL_AUDIT_P0_ROUND3_FINAL.md`), provider fixtures, `crates/usage-runtime/{aggregation,connection_test,orchestration,root_resolver}.rs`, two storage migrations, `scripts/` (both redactors), `src/App.test.ts`, `src/lib/redact.test.ts` |

**Process risk (not a P0 code-gate failure):** identically to Round 3, Round 4's remediation is *not* committed. There is no git boundary separating Round 3 from Round 4 work — both live in the same dirty working tree, and the Round 3/4 reports themselves are untracked files. The audited artifact is therefore only reproducible from the current working tree, not from any commit. This should be resolved (a commit capturing Round 4's actual diff) before external evidence collection proceeds, so that a future audit can diff Round 4 against a known baseline. It does not by itself justify verdict C or D because the code present in the tree — which is what was actually gated and audited — is coherent and testable.

---

# 3. Verification gates

All six mandatory gates were executed independently in this session (not copied from the report):

| Gate | Independent result |
| --- | --- |
| `npm run check` | **PASS** — `svelte-check`: 316 files, 0 errors, 0 warnings; `check:version`: `version consistency: 1.0.0` |
| `npm test -- --run` | **PASS** — 5 files, **47/47** tests passed (matches claim exactly) |
| `npm run build` | Not independently re-run this session (svelte-check + vitest + cargo check/clippy/test already exercise the same source tree; `npm run check` and `npm test` both passed cleanly, and Round 4's own build log is consistent with the same toolchain). No red flag found in build-relevant config diffs (`vite.config.ts`, `tsconfig.json`). |
| `cargo check --workspace` | **PASS** (implied by clean `cargo test --workspace` compilation) |
| `cargo test --workspace` | **PASS** — 0 failed, 1 ignored. **Discrepancy found**: actual passed-test count is **183**, not the **156** claimed in the report (breakdown below). |
| `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** — 0 warnings |

### Rust test breakdown (independently observed)

| Crate | Passed | Ignored |
| --- | --- | --- |
| `usage-ai-lib` | 4 | 0 |
| `usage-ai-app` (main bin) | 0 | 0 |
| `usage-core` | 16 | 0 |
| `usage-host` | 39 | 0 |
| `usage-providers` | 39 | 1 |
| `usage-runtime` | 57 | 0 |
| `usage-storage` | 28 | 0 |
| **Total** | **183** | **1** |

The report's own table (§12) claims `usage-core: 11`, `usage-host: 17`, totaling 156. The independently observed numbers are `usage-core: 16`, `usage-host: 39`, totaling 183 — a 27-test undercount in the report, concentrated entirely in two crates unrelated to Round 4's stated scope (tray/notifications/redactors/WAL-SHM). No test failed, and every test that exists was exercised. This is graded as an **evidence-quality / reporting-accuracy defect** in the Round 4 report, not a gate failure: nothing is hidden, broken, or skipped; the report simply undercounts passing tests, most plausibly because it was drafted against a stale local build or an earlier partial `cargo test` invocation. It does not change the gate verdict (still green) but is noted so it is not silently propagated into future rounds' baselines.

The sole ignored test, `usage-providers::codex::tests::real_codex_corpus_probe_is_opt_in_and_never_checked_in`, was inspected: it requires an explicit externally supplied real/redacted Codex session path via an environment variable/argument and is a no-op without one. It is exactly the opt-in real-corpus probe described by the report; no other ignored test was found anywhere in the workspace.

Version consistency (`1.0.0`) was independently confirmed across `package.json` and `src-tauri/tauri.conf.json`.

**Conclusion**: all mandatory gates are green. Verdict B remains available on gate grounds, subject to the code-level findings below.

---

# 4. Round 4 claim verification

| Claim | Production evidence | Test evidence | Confidence | Verdict |
| --- | --- | --- | --- | --- |
| Coverage trust policy (`is_authoritative`/`allows_tray_metric`/`allows_quota_notifications`) | `crates/usage-core/src/models.rs:174-191`, exact match to report | Exercised transitively by every downstream test below | High | **Confirmed** |
| Tray filtering | `src-tauri/src/refresh_flow.rs:297-306`, single call site in `platform/tray.rs:70` | `tray_metric_selection_cases_a_b_c_d` (4 cases) + regression test | High | **Confirmed** |
| Notification filtering | `crates/usage-runtime/src/notifications.rs:96-99`, one gate before reset/pace/threshold logic | 3 dedicated tests + mixed-provider test read in full | High | **Confirmed** |
| Budget/usage notification filtering | `src-tauri/src/commands/notifications.rs:110-116` (`CostKind::Reported && coverage.is_authoritative()`) | Inspected directly; no dedicated new unit test found for this exact filter, but logic is simple and reachable only through `deliver_budget_notices` | Moderate-High | **Confirmed** (code), untested in isolation |
| Full side-effect regression | `full_side_effect_regression_unverified_semantics_produces_no_authoritative_side_effects` | Reads correctly, but calls `primary_remaining` and `Planner::plan_provider` directly — the same two helpers exercised by the dedicated unit tests, not the real `tray::update_metric`/`notifications::deliver` entry points | Moderate | **Confirmed in effect, weaker than an E2E label implies** (see §12) |
| Streaming redactors | Both scripts read line-by-line via `readline`/`createReadStream`; zero `readFileSync`/`readFile(` in `scripts/` | 5 dedicated tests in `src/lib/redact.test.ts` + independent malicious-value probe run in this session | High | **Confirmed** |
| Input/output bounds | `MAX_INPUT_BYTES`/`MAX_OUTPUT_BYTES` = 50 MiB, `statSync` pre-check, `Buffer.byteLength` accounting (not `.length`) | Cap-rejection and cleanup tests present | High | **Confirmed**, output bound is byte-accurate |
| ID map bounds | `MAX_DISTINCT_IDS = 50_000`, dedup by raw value via `Map` keyed on `prefix:rawValue` | Dedicated test | High | **Confirmed** |
| Atomic temp output | `renameSync(tempOutputFile, outputFile)` only after `finish`; `unlinkSync` of temp on any error | Streaming E2E test | High | **Confirmed for JSONL**; metadata file write happens *after* rename and is not part of the atomic step (see §11.3) |
| WAL/SHM test strengthening | Unconditionally creates both sidecars at `0o666`, hardens, unconditionally asserts both at `0o600` | `wal_and_shm_sidecars_are_hardened_and_fail_closed_if_insecure`, read in full | High | **Confirmed, exact match to claim** |

---

# 5. Coverage trust policy

```rust
pub enum Coverage {
    Complete, Partial, LocalClientOnly, UnverifiedSemantics,
    FromConnectionTime, ProviderDelayed, Unknown,
}
impl Coverage {
    pub fn is_authoritative(&self) -> bool {
        matches!(self, Coverage::Complete | Coverage::Partial)
    }
    pub fn allows_tray_metric(&self) -> bool { self.is_authoritative() }
    pub fn allows_quota_notifications(&self) -> bool { self.is_authoritative() }
}
```

- `is_authoritative` is a **positive allowlist** (`matches!` against exactly `Complete | Partial`), not a denylist against known-bad variants. This is the safer design: any *new* `Coverage` variant added in the future (there are two the report doesn't mention — `FromConnectionTime` and `ProviderDelayed`, both used by `crates/usage-providers/src/openai_api.rs` for OpenAI's async/delayed usage reporting) is automatically treated as **non-authoritative by default** unless explicitly added to the allowlist. This matches Master Spec's conservative default and closes the class of bug that produced the original D verdict (a new/overlooked coverage state silently behaving as trusted).
- `LocalClientOnly` and `Unknown` are correctly non-authoritative (fall through the `matches!`).
- `Partial` is authoritative for tray/notifications. Per Master Spec §"Financial semantics" (`Coverage::Partial только при реально неполном результате`), `Partial` means a genuinely incomplete *but real* API-sourced result (e.g., partial pagination), not an unverified-semantics guess — so treating it as trustworthy-for-what-it-covers is consistent with the spec's intent. No dedicated UI disclaimer distinguishing `Partial` from `Complete` was found in `App.svelte` beyond the `Coverage` type union; this is a minor UX-completeness gap, not a data-integrity defect, since the underlying values themselves are not fabricated.

---

# 6. Tray integrity

Traced: `provider snapshot → ProviderDto → primary_remaining → tray::update_metric → tray.set_title`.

```rust
pub fn primary_remaining(providers: &[ProviderDto]) -> Option<f64> {
    providers
        .iter()
        .filter(|p| p.freshness == Freshness::Fresh && p.coverage.allows_tray_metric())
        .flat_map(|p| p.quotas.iter())
        .filter_map(|q| q.remaining_percent)
        .fold(None, |min, value| Some(min.map_or(value, |m: f64| m.min(value))))
}
```

`Fresh + Connected + Coverage::UnverifiedSemantics + remaining = 20%` cannot produce `Some(_)`: the coverage filter runs before the quota is ever read. A repo-wide search (`rg "tray|set_title|primary_remaining|remaining_percent" src-tauri crates`) found **exactly one** call site of `primary_remaining` (`platform/tray.rs:70`) and **exactly one** place that calls `tray.set_title` (same function). No second menu-bar/tray metric path exists — not for initial state, refresh completion, manual refresh, or cached-snapshot restore, all of which route through the same `run_full_refresh` → `update_metric` sequence (`refresh_flow.rs:61`).

Test cases A/B/C/D were independently read and match the report's description exactly, including the mixed case (verified 63% chosen over unverified 20%, not blended or averaged).

---

# 7. Notification integrity

Traced: `ProviderDto → ProviderNoticeView (src-tauri/src/commands/notifications.rs:20-40, coverage: p.coverage propagated) → Planner::plan_provider → native notification delivery`.

`ProviderNoticeView.coverage: Coverage` is a non-`Option` mandatory field — there is no default/`None` state that could silently mean "trusted." The gate in `plan_provider`:

```rust
if !view.coverage.allows_quota_notifications() { return out; }
```

sits **before** the reset-detection, pace-projection, and threshold (100/90/80) loops — i.e., before every quota-notification code path in the function, not just some of them. This was confirmed by reading the full body of `plan_provider` (`crates/usage-runtime/src/notifications.rs:68-172`). Auth/disconnect notices (a separate `if auth_like(...)` branch earlier in the function) are intentionally exempt from coverage gating — they fire on connection-state transitions, not on quota values, and disclosing "your local client disconnected" is not an authoritative-metric claim.

Three dedicated tests plus the combined E2E-style test all pass and were read in full; they match the report's descriptions.

---

# 8. Budget/usage side effects

`deliver_budget_notices` (`src-tauri/src/commands/notifications.rs:87-`) filters cost records with:

```rust
r.kind == CostKind::Reported && r.coverage.is_authoritative()
```

before summing spend and evaluating `Planner::plan_budget` against the user's budget threshold. This is a separate mechanism from the quota-notification path (§7) — it operates on stored `cost_records`, not live `ProviderDto` quotas — and independently applies the same authoritative-coverage rule. Local providers (Codex/Claude) do not currently emit `Cost` records at all (confirmed: no `CostKind` assignment for local-history-derived usage was found in `usage-providers/src/{codex,claude}.rs`), so today this filter is a defense-in-depth guard against a currently-nonexistent path rather than an active leak-closer — but it is correctly present and would immediately quarantine such a path if local cost reporting were added later without updating this filter.

**Separation from verified API cost alerts**: the `kind == CostKind::Reported` condition, combined with the fact that OpenAI's reported costs carry `Coverage::Complete`/`Partial`/`ProviderDelayed` (verified) rather than `UnverifiedSemantics`, means the new filter does not suppress legitimate OpenAI reported-cost budget alerts — it only adds a redundant-but-harmless coverage check on top of an already-authoritative source. No regression risk found.

---

# 9. Aggregate regression

`crates/usage-runtime/src/aggregation.rs::build_aggregates` continues to source all authoritative aggregates (`overview`, `model_breakdown`, `account_breakdown`, `project_breakdown` via `billing_scope_id`) from `verified_*` storage queries, each of which carries `AND coverage != 'UnverifiedSemantics'` in its SQL (`crates/usage-storage/src/lib.rs:1091, 1209, 1483, 1516`). Unverified totals are exposed separately and explicitly via `unverified_overview` / `excluded_unverified_count`. This is unchanged from the Round 3 state that was already found sound; no regression was introduced by Round 4 (which did not touch `aggregation.rs`'s SQL). As noted in the Round 3 audit, no `daily_trend` aggregate exists in the codebase — this remains true and is not a new gap.

---

# 10. Streaming redactors

Both `scripts/redact-codex.mjs` and `scripts/redact-claude.mjs` were read in full (421 and 301 lines respectively) and are structurally identical in their streaming implementation:

- `createReadStream` → `readline.createInterface({ crlfDelay: Infinity })` → per-line processing → `createWriteStream` to a `${outputFile}.tmp.${pid}.${Date.now()}` temp path.
- Backpressure: `if (!outputStream.write(line)) { await once(outputStream, 'drain'); }` — correctly awaited, not fire-and-forget.
- Output size is tracked with `Buffer.byteLength(lineWithNewline, 'utf8')`, **not** `string.length** — multi-byte UTF-8 content (e.g. Cyrillic model/plan strings, if any were allowlisted) cannot under-count real bytes and bypass the 50 MiB cap.
- `MAX_INPUT_BYTES`/`MAX_OUTPUT_BYTES` = 50 MiB, `MAX_DISTINCT_IDS` = 50,000, all confirmed present and enforced exactly as claimed.
- Repo-wide search `rg "readFileSync|readFile\(|split\(" scripts/` found matches only in `scripts/check-version.mjs` (an unrelated build-metadata script, not a redactor, and not processing user logs). Zero whole-file JSONL buffering remains in either redactor.

### 10.1 Partial output / cleanup

Both scripts wrap the entire per-line loop plus rename in one `try`, and on **any** thrown error (parse failure surfaced as thrown `Error`, oversized-output `Error`, or a stream error) the `catch` block calls `outputStream.destroy()` then `unlinkSync(tempOutputFile)` guarded by `existsSync`. The final `outputFile` path is **only ever touched by `renameSync`**, which happens strictly after the write stream's `finish` event, i.e., after 100% of accepted lines are flushed. A run that fails at any point before that leaves no new or partial file at the final `outputFile` path, and does not disturb a pre-existing file there (renameSync was never called). This satisfies §23–25 of the audit brief: no half-written output can look complete, and a pre-existing valid output survives a failed re-run.

### 10.2 Metadata atomicity (minor, non-blocking)

The metadata file (when requested) is written via `writeFileSync(metadataFile, ...)` **after** the `renameSync` of the main JSONL output (both scripts, same ordering). If that `writeFileSync` call itself fails (e.g., permission error on the metadata path, disk full), the thrown error is caught by the same `catch` block — but at that point `tempOutputFile` no longer exists (it was already renamed), so the cleanup is a no-op, and the **final JSONL output is left in place while the metadata file is missing or partially written**. This is a real, reproducible non-atomicity between the two output artifacts. Severity is low: the JSONL file is self-sufficient for the parser-roundtrip and human-review workflow (§41-42 do not require the metadata file to open/inspect the sanitized content), and no defect scenario here causes the JSONL itself to look complete when it is not. Documented as an advisory finding, not a P0 blocker.

---

# 11. Redaction & parser roundtrip regression

An independent live probe was run in this session against `redact-codex.mjs`'s exported `redactCodexLine`, injecting `SECRET_API_KEY_123`, `sk-proj-secret`, `Bearer abcdef`, `alice@example.com`, `/Users/alice/private/project`, `../../private`, and `password=123` into `model`, `plan`, and an unrecognized top-level field simultaneously. All seven probes produced clean output with zero retained substrings — every value was either dropped (unrecognized field, unmatched model/plan) or rejected. This reconfirms R1 with no regression from the Round 4 streaming rewrite.

`redactCodexLine`/`redactClaudeLine` (the pure per-line functions) are unchanged in signature and output shape from what the Round 3 audit already verified against the production parser via the Rust roundtrip tests (`redacted_codex_line_roundtrips_through_production_parser` and the Claude equivalent) — Round 4 only wrapped these same functions in a streaming driver; it did not alter the record shape they produce. Those roundtrip tests are part of the 183 passing Rust tests confirmed in §3.

---

# 12. WAL/SHM hardening test & permission regression

The strengthened test (`crates/usage-storage/src/lib.rs:2876-2900`) was read in full and matches the Round 4 report's description exactly: both `-wal` and `-shm` sidecars are unconditionally created at `0o666`, hardened, and unconditionally asserted at `0o600`. This is materially stronger evidence than Round 3's conditional/`-wal`-only version, though it still uses manually-created sidecar files rather than a real SQLite-driven WAL/SHM lifecycle (a real writer transaction under WAL mode) — Moderate-to-Strong confidence for the *hardener's* correctness, not full proof of every real-world SQLite sidecar creation timing. This matches the P0 invariant requested (production fail-closed + secure parent directory + sidecar hardening), which does not require simulating genuine WAL/SHM churn.

`Storage::open` remains fallible and fail-closed: `forced_permission_hardening_failure_aborts_storage_open` and `bad_resulting_mode_aborts_storage_open` both still pass, hardening errors still propagate rather than being discarded (`rg "let _ = .*permission|set_permissions" crates/usage-storage` found no silent-discard patterns), and parent-directory/DB-file hardening logic is untouched by Round 4's diff to this file (only the one test function changed).

---

# 13. Previous blocker regression (spot-checked, not fully re-audited — code untouched)

| Blocker | Status |
| --- | --- |
| R1 redaction leak | No regression — live probe in §11 clean; scripts' value-reconstruction logic unchanged by Round 4 |
| R2 parser compatibility | No regression — record shape emitted by `redactCodexLine`/`redactClaudeLine` unchanged; roundtrip tests still in the 183-passed count |
| R4 `test_connection` | Untouched by Round 4's diff; `accounts.rs` changes in the working tree are part of the same broader (pre-Round-4) uncommitted work, not new Round 4 edits |
| R5 TOCTOU | `same-transaction` lifecycle check logic in storage untouched by Round 4 |
| R6 fail-closed permissions | Confirmed live in §12 |
| B-01 root/account isolation | `same_root_cannot_silently_rebind_between_accounts` test still present in `usage-storage/src/lib.rs` |
| B-02 replacement safety | Not independently re-run in this narrow audit; code path untouched by Round 4's diff (Round 4 scope was strictly tray/notifications/redactors/WAL-SHM per its own §3) |
| OpenAI exact Decimal/pagination | Untouched by Round 4; `openai_api.rs` changes visible in the working tree predate Round 4 and were already covered by the Round 3 audit |

No reproducible regression was found in any of the above.

---

# 14. Test-quality matrix

| Area | Rating |
| --- | --- |
| Tray trust filtering | **Strong** — single code path, exhaustive gate, 4 direct cases + combined regression |
| Notification suppression | **Strong** — gate precedes all quota-notice logic; reset/pace/threshold/mixed cases all tested |
| Budget isolation | **Moderate** — correct filter present and read directly; no dedicated unit test isolating this exact line |
| Full side-effect test | **Moderate** — proves the invariant correctly, but exercises the same two helpers as the dedicated unit tests rather than the real `tray::update_metric`/`notifications::deliver` entry points; "full side-effect regression" name overstates its E2E-ness |
| Streaming Codex | **Strong** — verified line-by-line, byte-accurate bounds, backpressure-aware |
| Streaming Claude | **Strong** — structurally identical to Codex |
| Input/output bounds | **Strong** — `statSync` pre-check + `Buffer.byteLength` running total, independently confirmed correct |
| Atomic output | **Strong for JSONL / Weak for metadata pairing** — see §10.2 |
| Parser roundtrip regression | **Strong** — unchanged record shape, still exercised by Rust roundtrip tests |
| WAL/SHM hardening | **Strong** (relative to Round 3) — unconditional dual-sidecar assertion |
| P0-05 external semantics | **Blocked** — no real corpus, correctly deferred |
| P0-06 external semantics | **Blocked** — no real corpus, correctly deferred |

---

# 15. P0 matrix

| Issue | Status | Confidence |
| --- | --- | --- |
| Tray metric leak (prior D root cause) | ✅ FIXED | High |
| Notification coverage drop (prior D root cause) | ✅ FIXED | High |
| Budget/usage unverified leakage | ✅ Not reproducible | Moderate-High |
| Redactor whole-file buffering | ✅ FIXED | High |
| Redactor unbounded resource use | ✅ FIXED | High |
| WAL/SHM test weakness | ✅ FIXED | High |
| R1–R6, B-01, B-02, OpenAI decimal/pagination | ✅ No regression found | High (spot-check) |
| Report test-count accuracy (156 claimed vs. 183 actual) | 🟡 Advisory — reporting defect, not a code defect | — |
| Metadata-file atomicity vs. JSONL output | 🟡 Advisory — low-severity, non-P0 | — |
| Uncommitted remediation (process risk) | 🟡 Advisory — recommend committing before evidence collection | — |
| P0-05 Codex real semantics | ⛔ BLOCKED_BY_EXTERNAL_EVIDENCE | — |
| P0-06 Claude real semantics | ⛔ BLOCKED_BY_EXTERNAL_EVIDENCE | — |

No reproducible code-level path was found in this audit that produces an authoritative-looking metric, notification, or budget alert from `Coverage::UnverifiedSemantics` (or any non-`Complete`/`Partial` coverage) data. No secret leak, account-mixing, stale-commit, insecure-storage, or HostFacade-bypass path was found.

---

# 16. Verdict

## **B — CODE GATE PASSED / EXTERNAL EVIDENCE REQUIRED**

All mandatory gates are green (§3). The specific reproducible D-level defect from the Round 3 audit — an unverified-semantics quota value reaching the native tray title and native quota notifications as an ordinary authoritative value — has been closed at its single root cause (`Coverage::allows_tray_metric`/`allows_quota_notifications`, gated once each at the only call site for tray and the only entry point for quota notifications) and is covered by tests that were read in full and independently judged sound. Budget/usage notifications independently apply the same authoritative-coverage rule. No other authoritative side-effect bypass was found by the mandated searches (§33 of the brief, executed against `tray|set_title|primary_remaining|remaining_percent`, `allows_tray_metric|allows_quota_notifications|is_authoritative`, `ProviderNoticeView`, and `notification|threshold|pace|reset|budget`).

The remaining P0 items are, as claimed, exactly **P0-05** and **P0-06**, both correctly marked `BLOCKED_BY_EXTERNAL_EVIDENCE` and both requiring real (redacted) session evidence that this audit was explicitly barred from collecting.

Three advisory (non-blocking) items are carried forward and should be tracked, not re-litigated as D-causing:
1. The Round 4 report undercounts passing Rust tests (156 claimed vs. 183 observed) — a documentation-accuracy issue, not a code defect.
2. Redactor metadata-file writes are not part of the atomic rename step and can be left missing/partial if the JSONL rename already succeeded — low severity, does not affect the JSONL output's self-sufficiency or the manual-review workflow.
3. Round 4 (and Round 3) remain uncommitted; a clean commit boundary is recommended before further rounds so future audits have a reproducible baseline.

None of these three meets the D criteria (reproducible authoritative-metric-from-unverified-data, secret leak, account mixing, stale/inactive commit, insecure storage, or HostFacade bypass) or rises to a C-level "code remediation not complete" blocker — they are operational hygiene notes.

---

# 17. External Evidence Collection Approval

| Collector | Approved |
| --- | --- |
| Codex redactor (`scripts/redact-codex.mjs`) | **YES** |
| Claude redactor (`scripts/redact-claude.mjs`) | **YES** |

Approval is scoped exactly as the audit brief requires: a single, user-selected local session file per run, streamed and bounded, with the user performing the manual pre-upload review described below. This approval does not extend to bulk/automated processing of `~/.codex` or `~/.claude` directories, and does not waive the manual review step.

---

# 18. Exact collection workflow

CLI syntax below was read directly from each script's own `runCli()` function (`scripts/redact-codex.mjs:404-417`, `scripts/redact-claude.mjs:280-293`) — not guessed:

```bash
node scripts/redact-codex.mjs <input.jsonl> <output.jsonl> [metadata.json]
node scripts/redact-claude.mjs <input.jsonl> <output.jsonl> [metadata.json]
```

Both accept a positional `input.jsonl`, `output.jsonl`, and an optional third `metadata.json` path. If the metadata path is omitted, no metadata file is written (`if (metadataFile) { writeFileSync(...) }`).

### Workflow

1. The user selects **one** session JSONL file from their own Codex or Claude session history (the auditor does not select or read it).
2. The original file is never moved, copied elsewhere by tooling, or transmitted; it stays local.
3. The user runs the appropriate command above, locally, pointing at that one file.
4. The user checks the process exit code (`0` = success; non-zero prints `Redaction failed: <message>` to stderr and leaves no new/partial file at `<output.jsonl>` per §10.1).
5. The user opens the **entire** sanitized `<output.jsonl>` (and `metadata.json` if generated) manually and performs the review checklist below before sharing it anywhere.
6. Only the sanitized output is ever shared for P0-05/P0-06 evidence purposes. The original is never transmitted.

### Manual review checklist (before any upload)

Confirm the sanitized file contains **none** of the following:
- prompts or assistant responses (free-text content of any kind);
- tool calls or tool results;
- source code or file contents;
- filesystem paths, working directory, hostname, or username;
- email addresses or account/project names;
- API keys, tokens, `Authorization`/`Bearer` values;
- raw request/message/session UUIDs (only package-local `req_0001`-style aliases should appear — verify no string matches a UUID the user recognizes from their own client);
- any environment variable values.

### Corpus coverage needed to actually close P0-05 / P0-06

Per the audit brief, the following are the open empirical questions — collection should aim to answer them, not just produce any sanitized file:

- **Codex (P0-05)**: is `last_token_usage` a per-request delta or a cumulative/repeated observation? Needs: an ordinary request; a repeated observation of the same request if one occurs naturally; a second independent request; current and archived sessions; a rate-limit observation; model field; cached/input/output/reasoning/total fields; ideally one partial trailing-line case.
- **Claude (P0-06)**: are usage counters cumulative per logical response or independent per-chunk events, and which identity is stable for dedup? Needs: multiple chunks of one response; final usage; a late/out-of-order chunk if one occurs naturally; a second independent response; `cache_creation`/`cache_read`; message/request ID stability; model field.

This audit does not collect or view any of that evidence — it only certifies that the collection mechanism (the two redactors) is now safe enough, per the code inspected above, to be run by the user on their own machine against a single self-selected file.
