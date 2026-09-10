# 1. Metadata

- Audit type: independent P0 Gate Audit after `P0 Remediation Round 3`.
- Audit date/time: 2026-09-09, Asia/Almaty.
- Auditor verdict: **D — DATA INTEGRITY / SECURITY RISK**.
- Audited branch/commit: `main` / `d99554e289c40df6a393fb8d5ad71132ff353b1d` plus the dirty working tree described below.
- Normative document: `/Users/nuras/Downloads/Usage.ai_MASTER_SPEC_v1.1.md`, 2,225 lines, SHA-256 `63976e26a1494f2cf10ea49f0cddffc945f4aa3170e2b31b70defe5862daaab0`.
- Previous audit: `TECHNICAL_AUDIT_P0_FINAL.md`, 432 lines, SHA-256 `d9493d6083501aaba7784f3fbe5506cce5eb5bb4eb19ac6d6eae30fc7545b3bc`.
- Claims document: `P0_REMEDIATION_ROUND3_REPORT.md`, 299 lines, SHA-256 `1b11c3f935c20df5612dd726a27208fd9b5f700b7ac9503193cbef68d121ecb2`.
- Scope: repository code, tests, synthetic temporary redactor inputs, and mandatory build/test gates only.
- Privacy constraint observed: no real Codex/Claude logs, user session files, or personal evidence were opened or processed.
- Mutation constraint observed: no production code, tests, fixtures, providers, Master Spec, or remediation reports were changed. This audit report is the only created file.

The remediation report was treated as a list of claims, not evidence. Every conclusion below is based on inspected implementation, tests, and execution paths.

# 2. Repository state

Commands required by the audit were run before this report was created.

| Property | Result |
| --- | --- |
| Branch | `main` |
| HEAD | `d99554e289c40df6a393fb8d5ad71132ff353b1d` |
| Upstream | `origin/main`; local branch ahead by 3 commits |
| Worktree | Dirty |
| Staged changes | None |
| Unstaged tracked changes | 44 files |
| Diff stat | 44 files, 5,559 insertions, 1,725 deletions |
| Untracked | Reports, provider fixtures, new runtime modules, migrations, scripts, frontend tests |

Relevant untracked implementation/evidence includes:

- `crates/usage-runtime/src/aggregation.rs`
- `crates/usage-runtime/src/connection_test.rs`
- `crates/usage-runtime/src/orchestration.rs`
- `crates/usage-runtime/src/root_resolver.rs`
- `crates/usage-storage/migrations/006_*` and `007_*`
- `scripts/`
- `src/App.test.ts`
- `src/lib/redact.test.ts`
- `P0_REMEDIATION_ROUND2_REPORT.md`
- `P0_REMEDIATION_ROUND3_REPORT.md`

Last five commits:

```text
d99554e C2 hosts...
b267d73 C1 storage...
750a0cd P0-02...
b4c04de ...
0428edd ...
```

Round 1 has committed work in the three commits ahead of `origin/main`. The large Round 2/Round 3 remediation is not represented by a clean commit boundary: it remains mixed in a large dirty working tree, and the Round 3 report itself is untracked. Therefore the audited artifact is not reproducible from HEAD alone and Round ownership cannot be established reliably from Git history.

# 3. Verification gates

The complete mandatory chain was run independently:

```bash
npm run check
npm test -- --run
npm run build
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

All commands exited successfully.

| Gate | Independent result |
| --- | --- |
| `npm run check` | PASS; 0 errors, 0 warnings; version consistency PASS |
| Frontend tests | PASS; 5 files, 42/42 tests |
| Frontend build | PASS; Vite built 144 modules; `node:async_hooks` message was informational |
| `cargo check --workspace` | PASS |
| Rust tests | PASS; 178 passed, 1 ignored |
| `cargo clippy ... -D warnings` | PASS; 0 warnings |
| App/package version | `1.0.0` consistently |

Rust passed-test distribution observed in the run: app 2, core 16, host 39, providers 39, runtime 54, storage 28. The sole ignored test is `codex::tests::real_codex_corpus_probe_is_opt_in_and_never_checked_in`. It is explicitly opt-in and requires an external real/redacted corpus path; no other ignored test concealed a code gate failure.

Green compilation and test gates are necessary but not sufficient for verdict B. The notification/tray path described in sections 9 and 18 is not covered by the current isolation tests.

# 4. Round 3 claims verification

| Claim | Production implementation | Test/execution evidence | Audit result |
| --- | --- | --- | --- |
| R1 value-level redaction allowlists | Both redactors reconstruct narrow output objects; enum/grammar/numeric/timestamp/opaque-ID helpers are used | 24 frontend redactor tests plus independent malicious synthetic inputs | **Substantially verified** for leak prevention; whole-file memory behavior remains |
| R2 parser compatibility | Codex output uses `payload.info.last_token_usage`; Claude output preserves parser-required usage shape | Rust roundtrip tests invoked the Node CLI and fed its actual output to the production parser in this environment | **Verified** for synthetic schema compatibility |
| R3 unverified isolation | SQL/runtime authoritative usage filters and visible Svelte warnings exist | Mixed aggregation and component tests pass | **Only partial**: tray and notification quota paths discard/ignore coverage |
| R4 honest local `test_connection` | Tauri delegates to runtime; runtime probes bounded file content and fails closed | Missing, empty, malformed, unsupported, and valid-local tests pass | **Verified**, with bounded-sampling limitations |
| R5 atomic disable/delete protection | Lifecycle check and writes occur inside one `BEGIN IMMEDIATE` transaction | Pre-commit disable/delete tests and outcome tracing pass | **Verified** |
| R6 fail-closed storage permissions | Fallible hardener, read-back verification, before/after-open hardening | Forced-failure and permission tests pass | **Verified in production logic**; WAL/SHM lifecycle test is weaker than claimed |
| Migration confidence | Backfilled unmanaged connection becomes `Unknown`; modern verified identity remains verified | Both migration directions are tested | **Verified** |
| Tauri business aggregation removal | Main account aggregation moved to runtime | Source inspection | **Partial**: tray and notification aggregation/decision logic remains in Tauri-facing flow |

The Round 3 report's global statement that all six blockers are fully resolved is false because R3 was fixed only for panel UI and stored usage aggregates, not for all user-visible side effects required by the audit.

# 5. Redaction security

Both scripts were read in full, as was `src/lib/redact.test.ts`.

## Output value policy

| Value class | Codex | Claude | Assessment |
| --- | --- | --- | --- |
| Event/type discriminators | Enum | Enum | Safe constant/enum |
| Plan/limit labels | Enum/strict grammar | N/A where unsupported | Fail closed |
| Model | Strict prefix grammar plus sensitive-word denylist | Strict grammar plus denylist | No arbitrary scalar passthrough in tested shapes |
| Request/message/UUID IDs | Package-local encounter-order aliases | Package-local encounter-order aliases | Raw IDs not emitted |
| Timestamps | Validated timestamp or omitted/rejected | Validated timestamp or omitted/rejected | No source fallback to current time |
| Counters | Normalized safe integer | Normalized safe integer | Exact only within JS safe-integer range |
| Unknown fields | Not copied | Not copied | Fail closed by reconstruction |
| Metadata | Fixed schema/version/time/count fields | Fixed schema/version/time/count fields | No input path, host, account, project, or raw ID |

Independent temporary synthetic probes placed each of the following strings across candidate scalar fields: `SECRET_API_KEY_123`, `sk-proj-secret`, `Bearer abcdef`, `alice@example.com`, `/Users/alice/private/project`, `../../private`, `my-private-project`, `password=123`, `token-secret`, and `api_key_secret`. Searching sanitized output found **zero retained sensitive substrings**.

An unknown/future-schema object containing `secret_new_vendor_field`, `prompt_backup`, `user_email`, and nested secret content was reduced to the recognized safe schema; unknown values were not copied.

For raw IDs A/B/A, both scripts produced stable within-package aliases equivalent to `req_0001`, `req_0002`, `req_0001`. Maps and counters are created per redactor invocation and are not persisted; no raw ID appears in metadata. A new invocation starts again at `req_0001`. Thus equal first-position IDs in separate packages can share the same ordinal string, but that ordinal does not encode raw equality. This satisfies the request's explicit allowance for encounter-order evidence IDs, although the remediation report's broad wording about cross-run unlinkability should be read as structural unlinkability, not guaranteed output inequality.

Timestamp probes showed: missing timestamps were not replaced with `now`; invalid strings, objects, arrays, and non-finite-like values did not pass through and caused the affected line to be rejected rather than crashing the entire run. Rejection counts were updated.

Numeric policy observed:

- accepted: `0`, `1`, `9007199254740991`, and digit string `"123"`;
- rejected: values above `Number.MAX_SAFE_INTEGER`, negative integers, fractions, nonnumeric strings, and `null`.

The CLI uses ordinary `JSON.parse`, so a JSON integer such as `9007199254740993` is rounded before validation. The rounded result is still above the allowed maximum and is rejected. The implementation therefore does **not** preserve arbitrary JSON integers exactly; it safely accepts only counters representable as exact JS safe integers. That is practical for token counters but narrower than any claim of general exact-integer preservation.

Operational limitation: both CLIs call `readFileSync(..., "utf8")`, split the whole file, and accumulate all output lines before writing. A large personal JSONL corpus is therefore loaded into memory rather than streamed. This is not a demonstrated secret leak, but it prevents treating the scripts as production-scale evidence collectors without a file-size bound or streaming implementation.

# 6. Codex roundtrip

The sanitized Codex shape contains the parser-required hierarchy:

```text
payload.type = token_count
payload.info.last_token_usage = { ...safe counters... }
```

`redacted_codex_line_roundtrips_through_production_parser` first verifies a fixed sanitized shape, then—when the script and Node are available—executes the actual Node CLI, reads the produced sanitized JSONL, and passes those bytes directly to the production `parse_chunk_bytes` path. It does not use a separate test parser and does not rebuild the JSON between redaction and parsing. During the independent gate run, the CLI branch executed and the production parser accepted one record.

Confidence is strong for the current environment and synthetic schema compatibility. Portability confidence is moderate because the CLI portion of the test can silently skip when the script is absent or the external command fails. It is not semantic evidence for real Codex logs.

# 7. Claude roundtrip

The Claude redactor retains only validated timestamp, opaque message/request/UUID identifiers, allowlisted model, and safe usage counters including cache creation/read. Its Rust roundtrip test likewise executed the Node CLI in the current environment and fed actual sanitized output directly to the production Claude parsing path.

The parser accepted the synthetic redacted record and preserved the required relationships without raw IDs. This verifies schema compatibility only. It does not prove real chunk ordering, final-usage semantics, cache accounting, late-event behavior, or dedup identity.

# 8. Unverified semantics/UI

The principal panel path is implemented correctly:

```text
UsageRecord.coverage
→ SQLite coverage column
→ verified/unverified runtime aggregate split
→ Tauri snapshot/DTO coverage
→ TypeScript ProviderSnapshot.coverage
→ App.svelte warning presentation
```

For a fresh, transport-connected provider with `Coverage::UnverifiedSemantics`, `src/App.svelte` renders:

- an amber `.dot.unverified` indicator rather than a normal connection-state dot;
- visible text `Семантика не подтверждена`;
- a visible `Неподтверждённые данные` banner;
- a marked `Токены сегодня (неподтверждённые)` label;
- an overview note that unverified sources are excluded from the authoritative result.

`src/App.test.ts` mounts the real component with a Fresh/Connected/Unverified provider and asserts the indicator, label, banner, token wording, and overview exclusion note. This is strong evidence for the panel, not merely a DTO/unit assertion.

However, coverage honesty is not end-to-end across all product surfaces: the native tray and notification paths bypass this presentation logic. Therefore the overall R3 claim is partial.

# 9. Aggregation integrity

Authoritative stored-usage aggregates are correctly filtered:

- overview totals query `coverage != 'UnverifiedSemantics'`;
- model breakdown applies the same exclusion;
- account breakdown applies the same exclusion;
- project/billing-scope breakdown applies the same exclusion;
- unverified totals and excluded-provider counts are queried separately.

The mixed test inserts 150 verified tokens and 300 unverified tokens. It asserts 150 in the authoritative overview, 300 in `unverified_overview`, and a nonzero excluded count. Although the fixture values differ from the illustrative 100/900 in the audit request, the invariant is tested correctly.

There is no `daily_trend`/`dailyTrend` implementation in the repository. Consequently no implemented daily-trend path currently leaks unverified records, but the Round 3 report's claim that `daily_trend` was explicitly fixed is unsupported: that named aggregate does not exist.

## Reproducible remaining wrong-authoritative path

The following production path remains:

1. A local provider produces `Fresh + Connected + Coverage::UnverifiedSemantics` and a quota with `remaining_percent = 20`.
2. `src-tauri/src/refresh_flow.rs:294-304` computes the minimum quota across **all fresh providers**. It does not inspect coverage.
3. `src-tauri/src/platform/tray.rs:64-75` renders that result directly as the tray title, `20%`, without an unverified qualifier.
4. Separately, `src-tauri/src/commands/notifications.rs:19-42` converts `ProviderDto` to `ProviderNoticeView` and drops coverage completely.
5. `crates/usage-runtime/src/notifications.rs:92-170` accepts any fresh view and can emit exact threshold text such as `Использовано 80% · осталось 20%`, pace warnings, or a reset claim. The runtime view type has no coverage field with which to suppress or qualify the event.

This is deterministic from the code and is reachable by a normal Fresh local snapshot. The same unverified value that the panel visibly quarantines can therefore appear as an ordinary menu-bar metric and authoritative-looking native notification. This is the inconsistent authoritative behavior explicitly prohibited by the audit's notification/tray check and by the Master Spec priority of data correctness, coverage visibility, and coverage-aware notification behavior.

Cost/budget aggregation was also inspected. Local providers currently do not emit costs, so no equivalent unverified local-cost alert was reproduced. That does not mitigate the quota/tray path above.

# 10. Local connection testing

`src-tauri/src/commands/accounts.rs` now delegates connection testing to `usage_runtime::test_connection` and maps the result to a DTO. It no longer constructs concrete provider strategies, hardcodes a local `Connected/ok`, or decides local provider semantics.

Observed runtime behavior:

| Input state | Result behavior |
| --- | --- |
| Missing Codex/Claude root | Not Connected; unavailable/error with unverified coverage |
| Inaccessible root | Permission-denied/error state |
| Existing root, no candidates/data | Unavailable/no-data warning; not normal healthy metrics |
| Malformed candidate | Parse error; not Connected |
| Unsupported JSON schema | Unsupported/error; not Connected |
| Supported event found | Connected transport/source, but `UnverifiedSemantics` plus warning |

Tests cover both providers' missing roots and the empty, malformed, unsupported, and valid-local cases. The probe reads at most 4,096 bytes from the first non-empty readable candidate, so it does not scan gigabytes of log content. It may produce a false negative when the first valid event lies later in the file or only in a later candidate. That is a bounded-sampling accuracy limitation, not fake health: the path fails closed.

Candidate path enumeration itself is not byte-bounded and could be expensive in a directory containing very many matches. No root symlink escape producing unauthorized reads was demonstrated in the inspected path.

# 11. Runtime/Tauri boundary

The account connection command boundary now conforms: Tauri is wiring/DTO adaptation and runtime owns probing semantics. Most history and overview aggregation has also moved to runtime.

The claim of complete Tauri business-aggregation removal is nevertheless too broad:

- `src-tauri/src/refresh_flow.rs::primary_remaining` selects and aggregates the primary tray quota;
- `src-tauri/src/commands/notifications.rs` builds notice inputs and separately aggregates monthly spend before invoking planner behavior.

The Master Spec assigns notification business rules to `usage-runtime` and leaves native delivery/adaptation in Tauri. More importantly, keeping coverage selection out of the notice view caused the actual integrity defect in section 9. Boundary status is **partial**, not a standalone verdict-D issue beyond that reproduced consequence.

# 12. TOCTOU

`commit_refresh_bundle_if_active` starts one immediate SQLite transaction, reads account lifecycle/enabled state inside that transaction, performs the snapshot/usage/cost writes, and commits. The lifecycle check is not performed in a separate transaction.

The disable and delete tests pause after fetch/parsing/serialization at a test-only hook immediately before commit. They then disable/delete the account, resume the commit, and assert the explicit inactive/missing outcome and absence of new snapshot, usage, and cost rows. Protection does not rely solely on a foreign-key failure.

Outcome tracing shows that inactive/missing commit results return early as cancelled/unavailable outcomes. They do not execute the success path, update last-known-good data, mark `last_success`, or schedule success handling. The storage mutex serializes a lifecycle change that starts after `BEGIN IMMEDIATE`, providing a clear linearization point rather than a stale second transaction.

No stale/inactive account commit path or TOCTOU deadlock was reproduced. R5 is fixed with strong confidence.

# 13. Permissions

The production hardener is fallible and fail closed. On Unix it:

- hardens the database parent directory to user-only access;
- hardens the DB and existing SQLite sidecars;
- reads metadata back and rejects `mode & 0o077 != 0`;
- propagates hardening errors from `Storage::open`;
- runs around the open/migration lifecycle rather than ignoring permission failures.

Search found no permission-related `let _ = ...` that swallows these errors. Forced-hardener failure tests cause `Storage::open` to return `Err`. Normal permission tests pass on the audited macOS environment.

Evidence qualification: the test named for WAL/SHM lifecycle manually creates a permissive WAL file and asserts it becomes `0600`; it does not force and unconditionally assert both a real WAL and SHM file. Other assertions are conditional on sidecar existence. Production code enumerates and hardens both sidecars and verifies read-back, so the code is fail closed, but the report overstates the strength of lifecycle test coverage. R6 is fixed with moderate-to-strong confidence, not absolute proof for every SQLite sidecar timing.

# 14. Migration identity confidence

Migration/backfill behavior now assigns `Unknown` to legacy/unattributed managed connections rather than inventing `Verified`. A migration test covers that case. Existing valid modern API identity remains `Verified`, and a separate test protects it from accidental downgrade.

This matches the Master Spec rule that path, filename, display name, and unverified local attributes are not sufficient identity proof. No regression to automatic cross-account merging was found.

# 15. Regression review

Required searches were executed and production matches analyzed:

```bash
rg "std::fs|tokio::fs|reqwest::|std::env|Command::new" crates/usage-providers
rg "as_f64|from_f64" crates
rg "Coverage::UnverifiedSemantics" crates src-tauri src
rg "LocalClientOnly|Connected" src-tauri/src/commands
rg "identity_confidence.*Verified" crates/usage-storage
rg "let _ = .*permission|set_permissions" crates/usage-storage
```

Results:

- provider filesystem/environment/process references are confined to tests, the opt-in external probe, or explicit tripwire checks; no new production HostFacade bypass was found;
- no `as_f64`/`from_f64` monetary conversion path was found;
- no residual fabricated local `Connected` branch remains in Tauri account commands;
- storage `Verified` matches inspected in this area are test/modern-identity cases, not the legacy backfill;
- permission errors are not intentionally discarded.

B-01 root/account isolation tests pass: same-root silent rebind is rejected and unknown local identity is not merged with confirmed identity. B-02 replacement-total tests pass and no additive duplication regression was found.

Existing OpenAI regression tests pass for page/`next_page`, partial pagination, exact Decimal handling, one logical fetch, and HostFacade boundaries. No changed code path produced a contrary result.

Other sanity findings:

- no redactor whole-run crash was observed for malformed individual lines;
- no production parser schema mismatch was observed for synthetic redacted outputs;
- verified data remains in authoritative totals while unverified data is separately exposed;
- no test-only pre-commit hook is active in non-test builds;
- no normal-macOS permission open regression was reproduced;
- the Svelte component test provides basic layout/content regression coverage, but no screenshot-level visual QA was performed;
- redactors are not streaming and can consume memory proportional to the entire input and output corpus.

# 16. Test-quality matrix

| Area | Rating | Reason |
| --- | --- | --- |
| Redaction leak prevention | **Strong** | 24 tests plus independent cross-field malicious and unknown-schema probes; narrow reconstruction |
| Redactor parser roundtrip | **Moderate/Strong** | Actual CLI output enters production parser here; external-command failure can skip part of test |
| Unverified UI | **Strong for panel / Weak for side effects** | Real component assertions are good; tray/notifications omit coverage and have no isolation test |
| Aggregate isolation | **Strong for stored usage / Weak for quota side effects** | SQL/runtime mixed-data test is meaningful; tray/notice aggregation bypass remains |
| `test_connection` | **Moderate/Strong** | Honest state matrix and bounded bytes; first-candidate/first-4KiB false negatives remain possible |
| TOCTOU | **Strong** | Same-transaction lifecycle check and correctly positioned disable/delete tests |
| Permission fail-closed | **Moderate** | Production read-back/failure propagation is correct; real WAL+SHM lifecycle test is not unconditional |
| Migration confidence | **Strong** | Both legacy Unknown and preserved modern Verified cases tested |
| External semantics | **Weak / Blocked** | No approved real sanitized Codex or Claude corpus |

# 17. P0 matrix

| Issue | Status | Code evidence | Test evidence | Confidence |
| --- | --- | --- | --- | --- |
| R1 redaction leak | ✅ FIXED | Narrow allowlists, safe scalars, opaque IDs, unknown-field drop | 24 tests + independent malicious probes | High for inspected schemas |
| R2 parser compatibility | ✅ FIXED | Codex/Claude sanitized shapes match production parsers | Actual CLI-to-production-parser roundtrips executed | High for synthetic shapes |
| R3 unverified honesty | 🟡 PARTIAL | Panel and usage aggregates isolate coverage; tray/notices ignore/drop it | Panel/mixed aggregate tests pass; no side-effect coverage test | High that blocker exists |
| R4 local connection test | ✅ FIXED | Runtime probe; Tauri delegation; fail-closed states | Missing/empty/malformed/unsupported/valid cases | High |
| R5 TOCTOU | ✅ FIXED | Atomic immediate transaction and explicit lifecycle outcomes | Disable/delete pre-commit tests | High |
| R6 permission hardening | ✅ FIXED | Fallible hardener and Unix mode read-back | Forced failure and mode tests; weaker sidecar lifecycle | Moderate/High |
| Migration identity confidence | ✅ FIXED | Legacy backfill uses `Unknown` | Legacy and modern identity tests | High |
| B-01 root/account isolation | ✅ FIXED | Root/account binding protections retained | Regression tests pass | High |
| B-02 replacement safety | ✅ FIXED | Replacement/dedup semantics retained | Replacement totals pass | High |
| P0-05 Codex real semantics | ⛔ BLOCKED_BY_EXTERNAL_EVIDENCE | Synthetic parser path only | Opt-in real corpus test ignored as designed | Low/Blocked |
| P0-06 Claude real semantics | ⛔ BLOCKED_BY_EXTERNAL_EVIDENCE | Synthetic parser path only | No real sanitized corpus | Low/Blocked |

# 18. Verdict

## **D — DATA INTEGRITY / SECURITY RISK**

Verdict B is not earned.

Most Round 3 changes are substantive and the mandatory gates are green. The redactors no longer reproduce the previously demonstrated arbitrary-scalar leakage in the inspected schemas; parser roundtrips, local connection honesty, TOCTOU protection, permission failure behavior, and migration confidence are materially improved.

Nevertheless, there is a reproducible production code path to **misleading authoritative metrics**:

```text
Fresh + Connected + UnverifiedSemantics quota
→ coverage ignored by primary_remaining
→ ordinary "20%" tray title

Fresh + Connected + UnverifiedSemantics quota
→ coverage discarded when ProviderNoticeView is built
→ normal threshold/pace/reset notification
```

This meets the audit's explicit D criterion. It is not merely missing wording in a secondary diagnostics screen: it is a native menu-bar value and native notification generated from data that the panel itself correctly labels semantically unverified. R3 is therefore incomplete, P0-05/P0-06 are not the only remaining blockers, and transition to external evidence collection is not yet safe.

No independent reproducible path was found in this audit for secret leakage from the revised recognized schemas, account mixing, stale/inactive commit, insecure storage access, fake local connection health, or HostFacade/capability bypass. The D verdict is driven by the concrete tray/notification integrity path above.

# 19. Exact next action

Perform one narrowly scoped code remediation before collecting external evidence:

1. Carry `Coverage` into `ProviderNoticeView` (or filter before constructing it).
2. For `Coverage::UnverifiedSemantics`, either suppress quota threshold/pace/reset notifications or make every such native notification explicitly non-authoritative. Suppression is the safer P0 behavior until semantics are externally proven.
3. Make `primary_remaining` exclude unverified providers, or make the tray metric visibly and unambiguously unverified. Exclusion is the safer P0 behavior.
4. Add regression coverage using a `Fresh + Connected + UnverifiedSemantics + 20% remaining` snapshot and prove that it cannot produce an ordinary tray percentage or authoritative quota notice.
5. Re-run all mandatory gates and independently re-audit only this end-to-end side-effect path.

Before authorizing large personal corpus processing, also change the redactor CLI to stream JSONL or enforce and document a conservative input-size limit. This is operational hardening, not the cause of the D verdict.

Do not start P0-05/P0-06 evidence collection until the tray/notification blocker is closed and the code gate is reassessed.

# 20. External Evidence Collection Approval

| Collector | Approved now? | Reason |
| --- | --- | --- |
| Codex redactor | **NO** | Overall code gate is D; unverified results can still drive authoritative-looking side effects. The CLI also loads the complete input and output into memory. |
| Claude redactor | **NO** | Same gate and whole-file memory constraints. |

Because the verdict is not B, no command is authorized for execution on personal logs and no evidence workflow is issued. The inspected CLI syntaxes are recorded only to avoid ambiguity, **not as approval to run them**:

```text
node scripts/redact-codex.mjs <input.jsonl> <output.jsonl> [metadata.json]
node scripts/redact-claude.mjs <input.jsonl> <output.jsonl> [metadata.json]
```

The auditor did not run either command on real user data. Once a later audit grants approval, the minimum manual pre-transfer review must confirm absence of prompt/assistant/tool-call text, file contents, paths, cwd, email, hostname, username, API keys, bearer/authorization tokens, account/project names, and raw identifiers. Until then, originals and sanitized candidates should remain local and private.
