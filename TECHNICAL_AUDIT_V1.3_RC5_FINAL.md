# Usage.ai `v1.3.0-rc5` — final independent read-only release audit

Audit date: 2026-09-14 (Asia/Almaty)  
Candidate tag: `v1.3.0-rc5`  
Candidate commit: `28d5115624b7f1c8555da940c9959721068b2723`  
Audit mode: exact-tag detached read-only audit. No production code, tests, fixtures, migrations, lockfiles, CI, Master Spec, README, commits, pushes, tags, or release assets were changed by this audit.  
Audit execution status: **STOPPED at the first release-surface defect, as required by the supplied TЗ STOP RULE.** Results marked `NOT RUN — STOP RULE` are intentionally not converted into PASS claims.

## 1. Executive verdict

`v1.3.0-rc5` must not be final-tagged as `v1.3.0`.

Finding: **P1 release-surface documentation/security-accuracy defect** in the tracked README that is part of the exact RC5 tag. The README presents unsupported or unsubstantiated functionality as shipped functionality:

- `README.md:3` describes Gemini and Ollama as monitored key providers, although the exact RC5 registry exposes the corresponding products as `DiscoveryOnly` with no sources and the implementation has no Gemini/Ollama driver modules.
- `README.md:9` states a blanket “official quota API” model, although the candidate uses local structured history, Codex supported-client/App Server access, and Claude PTY/local strategies with different source classifications.
- `README.md:10` claims both Intel and Apple Silicon are compiled, while this candidate’s CI and independently inspected artifact cover only `x86_64-apple-darwin`; no Apple Silicon artifact was supplied or independently built before the STOP RULE.
- `README.md:22` claims provider drivers for Gemini and Ollama although `crates/usage-providers/src` contains no such driver and the descriptors are discovery-only.
- `README.md:24` calls the SQLite-backed usage storage “encrypted”; `crates/usage-storage/Cargo.toml:11` uses bundled `rusqlite`, and `crates/usage-storage/migrations/001_initial.sql:10-33` defines ordinary SQLite tables with plaintext `TEXT` fields. The implementation’s scoped Keychain handling is not evidence that the SQLite database itself is encrypted.
- `README.md:26` and `README.md:36` claim Tailwind CSS, but `package.json:19-32` has no Tailwind package or Tailwind build integration; the normative Master Spec instead states CSS variables/lightweight custom UI.

This is not a P0 runtime data-integrity finding: the inspected RC5 coverage code has an exact parser and the central Rust policy remains `Complete | Partial`. It is nevertheless a P1 release-surface defect because the claims alter the user-visible support, architecture, storage-security, and source-fidelity expectations of the final release. Under the requested verdict rules, the correct result is `RELEASE HOLD — REMEDIATION REQUIRED`.

The remediation report’s claims (`306` Rust tests, `$1.50` authoritative cost, artifact PASS, and live/native outcomes) were not treated as evidence. The independent audit had already completed the frontend gates, formatting, metadata/lockfile checks, static policy inspection, and artifact inspection before finding the README defect. Per the explicit stop rule, the remaining Rust gates and independent dataset execution were not run after the finding and no fix was attempted.

## 2. Remote tag/commit provenance

Independent remote/tag checks completed before the stop:

```text
origin = https://github.com/nurasss/Usage.ai.git
origin/main = 28d5115624b7f1c8555da940c9959721068b2723
v1.3.0-rc5^{commit} = 28d5115624b7f1c8555da940c9959721068b2723
v1.3.0-rc5 is an ancestor of origin/main = yes
```

The tag is annotated and points to the requested commit. Its commit parent is `a46786ac1b6903ab94dbeeb50406752809189758`; the RC5 commit itself is the documentation-only descendant recording the remediation report and artifact evidence. The remote peeled tag ref was observed as:

```text
83b909f8f294e1c3026d2c4f875046a9e084ebb6  refs/tags/v1.3.0-rc5
```

The tag list contains `v1.3.0-rc1` through `v1.3.0-rc5`. Neither local nor remote `v1.3.0` exists. Provenance therefore passes; the release is held for the independent P1 finding, not for a tag/SHA mismatch.

## 3. Master Spec provenance

`Usage.ai_MASTER_SPEC_v1.3.md` is tracked in the exact candidate. Independent SHA-256:

```text
38c5320684a694f24b5546f700453d23093a8340f5a2a1d4281480cb776d7d87
```

The same hash was independently obtained from RC3 and RC4. `git diff v1.3.0-rc4..v1.3.0-rc5 -- Usage.ai_MASTER_SPEC_v1.3.md` is empty. No weakening of the normative release gate was found. Relevant normative evidence includes:

- `Usage.ai_MASTER_SPEC_v1.3.md:164-189`: Gemini, Ollama, browser-cookie sources, and other listed integrations remain out of v1.3 and discovery-only.
- `Usage.ai_MASTER_SPEC_v1.3.md:204-209`: the frontend stack is Svelte, TypeScript, Vite, and CSS variables/lightweight custom UI; Tailwind is not part of the normative stack.
- `Usage.ai_MASTER_SPEC_v1.3.md:221-227`: the primary release target is Intel `x86_64-apple-darwin`; Apple Silicon must not be architecturally blocked but is not the main v1.3 release gate.
- `Usage.ai_MASTER_SPEC_v1.3.md:1613-1643`: Intel clean install, upgrade, smoke, notifications, launch-at-login, package, mandatory gates, and independent audit remain normative release requirements.

The README defect is therefore a contradiction/overclaim in the candidate release surface, not a spec change.

## 4. Clean detached audit tree

The audit used a temporary detached worktree at `/tmp/usageai-rc5-audit.9sz4gV` created from the exact `v1.3.0-rc5` tag. Before tests:

```text
HEAD = 28d5115624b7f1c8555da940c9959721068b2723
exact tag = v1.3.0-rc5
git status --porcelain = empty
```

The primary checkout was clean before the audit. The supplied RC4 audit stash was inspected but not applied:

```text
stash = e8fc09019b49bb358e8879b400223d50323a5eb2
stash parents include candidate index/worktree snapshots
stash is ancestor of v1.3.0-rc5 = no
```

The temporary detached worktree and audit-only temporary logs were removed after the stop. No harness was left in the repository. The existing user-supplied `/tmp/usageai-rc5-final-target` artifact directory was retained and was not deleted or modified.

The only repository file created by this audit is this report.

## 5. Mandatory gates

The following results are independent results from the clean detached RC5 worktree, obtained before the README finding:

| Gate | Independent result | Evidence/qualification |
|---|---|---|
| `cargo fmt --all -- --check` | PASS | No output; formatter check did not modify files. |
| `npm ci` | PASS | Added 137 packages, audited 138; npm emitted its existing 2-moderate-vulnerability advisory. |
| `npm run check` | PASS | `svelte-check found 0 errors and 0 warnings`; version check reported `1.3.0`. |
| `npm test -- --run` | PASS | 5 test files, 47/47 frontend tests passed. |
| `npm run build` | PASS | Vite build completed, 144 modules; the existing non-fatal `node:async_hooks` externalization warning was emitted. |
| `npm run check:pii` Node tripwire | PASS | `PII tripwire: clean.` |
| `npm run check:pii` Rust subgate | NOT RUN — STOP RULE | `cargo test -q -p usage-ai-app --lib pii_tripwire` was still compiling in the detached worktree when the mandatory stop was applied; it was interrupted, not classified as a test failure. |
| `cargo check --workspace --locked` | NOT RUN — STOP RULE | Not executed after the first defect. |
| `cargo test --workspace --locked` | NOT RUN — STOP RULE | Not executed after the first defect. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | NOT RUN — STOP RULE | Not executed after the first defect. |

The remediation report’s full-gate claims are not substituted for the three unexecuted Rust gates.

## 6. Test-count reconciliation

The independent frontend result is `47/47`.

The RC5 diff contains two new regression tests relative to RC4:

1. `crates/usage-storage/src/lib.rs:3104` — `persisted_coverage_parser_is_exact_and_fail_closed`.
2. `src-tauri/src/commands/notifications.rs:224` — `budget_spend_path_excludes_non_authoritative_rows`.

The remediation report claims `306 passed / 4 ignored` Rust tests, but the independent full Rust suite was not run because of the STOP RULE. Therefore the historical `297 -> 304 -> claimed 306` reconciliation is structurally plausible and the two additions are in the stated coverage/budget scope, but the claimed `306/4` count is **not independently accepted**.

## 7. Authoritative usage totals

Static production-path review found the expected central policy:

- `crates/usage-core/src/models.rs:176-182`: `Coverage::is_authoritative()` returns true only for `Coverage::Complete` and `Coverage::Partial`.
- `crates/usage-storage/src/lib.rs:1242-1255`: global verified totals use SQL `coverage IN ('Complete','Partial')`.
- `crates/usage-storage/src/lib.rs:1311-1329`: account/product provider totals use the same exact predicate.
- `crates/usage-runtime/src/aggregation.rs:73-151`: overview, model, account, project, and separate unverified overview use the trust-aware storage methods.
- `crates/usage-runtime/src/aggregation.rs:171-195`: provider-facing daily usage/cost applies the account/product and authoritative filters.

The required independent production-equivalent temporary dataset (`100 + 50 + 1000 + 2000 + 4000`) was **not executed after the stop**. Expected result `150` is not claimed as an independent runtime result in this report; the remediation report’s result is treated only as an unverified claim.

## 8. Authoritative cost totals

Static review found the RC5 cost boundary:

- `crates/usage-storage/src/lib.rs:1332-1362` loads persisted cost rows through `cost_records_between`.
- `crates/usage-storage/src/lib.rs:1358` maps persisted coverage through the exact parser.
- `crates/usage-runtime/src/aggregation.rs:224-245` filters cost rows by authoritative coverage, account, and product before grouping reported/estimated amounts.
- The daily provider path at `crates/usage-runtime/src/aggregation.rs:179-211` applies the same trust-aware filter.

The required independent cost dataset (`$1.00 + $0.50 + $10.00 + $20.00 + $40.00`) was **not executed after the stop**. Expected `$1.50` is not independently asserted here.

## 9. Exact Decimal

The inspected cost path retains `CostRecord.amount_decimal` as `rust_decimal::Decimal` (`crates/usage-core/src/models.rs:230-245`). Storage binds and parses the stored decimal text (`crates/usage-storage/src/lib.rs:1341-1351`), and runtime sums `Decimal` values (`crates/usage-runtime/src/aggregation.rs:180-194` and `224-241`) before converting only the final DTO amount to a string.

The static source sweep found no `f64`/`f32` conversion in these money aggregation paths. Quota percentages legitimately use `f64` in separate quota/tray paths; that is not a monetary conversion. The exact `$1.50` runtime assertion was not independently executed after the stop.

## 10. Raw/local analytics

The raw helper `Storage::token_totals_between` at `crates/usage-storage/src/lib.rs:1228-1240` has no coverage predicate and is used by local history import (`crates/usage-runtime/src/history_import.rs:240-250`). The separate verified and unverified usage helpers at lines `1242-1284` split trust-aware presentation from raw/history retention.

Static review therefore found no RC5 deletion or rewrite of non-authoritative historical rows. The required same-dataset runtime/export execution was not completed after the STOP RULE.

## 11. SQL/domain policy consistency

Static SQL/domain parity passes for the inspected paths: every verified usage aggregation query uses exactly `coverage IN ('Complete','Partial')`, matching `Coverage::is_authoritative()`.

Persisted cost coverage does not use a fuzzy parser in RC5. `parse_persisted_coverage_exact` at `crates/usage-storage/src/lib.rs:17-30` maps only exact enum strings and maps every other string to `Coverage::Unknown`. The schema declares coverage `NOT NULL` for usage and cost rows (`crates/usage-storage/migrations/001_initial.sql:10-21`), so NULL insertion is rejected by the production schema; SQL `NOT IN` also does not turn a NULL into an authoritative row.

No independent NULL/invalid-row execution was performed after the stop.

## 12. Budget regression

Static review of the real budget delivery path found:

- `src-tauri/src/commands/notifications.rs:125-161` obtains spend before calling `Planner::plan_budget` and before delivery deduplication.
- `src-tauri/src/commands/notifications.rs:170-191` scopes by account, product, currency, and `CostKind::Reported`, then requires `record.coverage.is_authoritative()`.
- The new regression at `src-tauri/src/commands/notifications.rs:224-292` keeps the normal alert behavior for a genuinely exceeded canonical amount.

The required independent `$1` authoritative + `$100` unknown with `$10` budget scenario, and the `$11` normal-alert scenario, were not executed after the stop. The remediation report’s budget result is not substituted.

## 13. RC3 trust regression

The inspected source still contains the prior Claude trust protections, including legacy Claude snapshot reclassification in `crates/usage-runtime/src/snapshot.rs:20-26` and the DTO guard in `src-tauri/src/dto.rs:387-393`. The exact policy preserves legitimate non-Claude `Partial` while downgrading legacy Claude `Partial`.

The requested migration 010, restart persistence, tray, notification, and legacy Claude regression suite was not rerun after the first defect. No new source regression was statically identified before the stop, but no independent PASS is claimed.

## 14. Diagnostics/PII regression

The independent Node diagnostics tripwire passed with `PII tripwire: clean.` The Rust PII subgate was still compiling and was stopped under the required STOP RULE. The exact six-string/arbitrary-project/export equivalence suite, negative tripwire, and full CI PII execution are therefore not independently completed in this audit.

No private contents or secrets were printed. The README finding is documentation accuracy, not a diagnostics data leak.

## 15. Multi-profile regression

The exact RC5 source retains descriptor/account/product scoping in the inspected aggregation and notification paths. The requested Codex A/B, Claude A/B, default/custom root, empty-root, and OAuth-default-only tests were not run after the stop. No independent PASS is claimed.

## 16. SWR/LKG regression

The audit did not rerun the targeted LKG, stale-plus-current-error, account-scoped cache, cooldown, or scheduler tests after the stop. No code change in the RC5 diff touched those surfaces, but this audit does not promote the remediation report’s claims to independent evidence.

## 17. Formatting/reproducibility

Independent pre-stop results:

- `cargo fmt --all -- --check`: PASS.
- `cargo metadata --locked --no-deps`: PASS; 6 workspace packages and 6 workspace members were reported.
- `git diff -- Cargo.lock package-lock.json`: empty in the detached candidate before the audit report was created.
- The candidate’s RC4-to-RC5 source diff is limited to the remediation report, `crates/usage-storage/src/lib.rs`, and `src-tauri/src/commands/notifications.rs`; the Master Spec and lockfiles are unchanged.

An informational `git diff --check v1.3.0-rc4..v1.3.0-rc5` reported trailing whitespace in the committed remediation report at its date/repository lines and one coverage-table line. It was not changed because the audit is read-only and had already reached the mandatory stop.

## 18. CI wiring

`.github/workflows/ci.yml` was inspected in the exact tag. The portable job explicitly wires:

- `npm ci`, `npm run check`, `npm run check:pii`, `npm test`, and `npm run build`;
- `cargo fmt --all -- --check`;
- `cargo check --workspace --locked`;
- `cargo clippy --workspace --all-targets --locked -- -D warnings`;
- `cargo test --workspace --locked`.

The `intel-macos` job explicitly wires the Intel target, locked check/test/build, Tauri bundle build, lockfile cleanliness, and an x86_64 binary check. No upload/publish step was found in this workflow. Wiring inspection PASS; execution of the unrun Rust jobs is not claimed.

## 19. Intel artifact provenance

The locally supplied remediation artifact was inspected at:

```text
app: /tmp/usageai-rc5-final-target/x86_64-apple-darwin/release/bundle/macos/Usage.ai.app
DMG: /tmp/usageai-rc5-final-target/x86_64-apple-darwin/release/bundle/dmg/Usage.ai_1.3.0_x64.dmg
```

Independent observed app executable (the bundle’s actual `CFBundleExecutable`):

```text
path: /tmp/usageai-rc5-final-target/x86_64-apple-darwin/release/bundle/macos/Usage.ai.app/Contents/MacOS/usage-ai-app
size: 19978528 bytes
mtime: 2026-09-13T22:50:32+0500
SHA-256: b9104b36fbe365d73ac377f1dc8ff26b6a739342606fae6e345b334e5f01ef6a
file: Mach-O 64-bit executable x86_64
lipo: Non-fat x86_64
```

The remediation report refers to `Contents/MacOS/Usage.ai`, which does not exist; `Info.plist` correctly names `usage-ai-app`. The actual executable’s size/hash match the report’s recorded values. This path mismatch is an evidence/report accuracy warning, separate from the P1 README finding.

The artifact was not independently rebuilt from a fresh exact-tag release target before the STOP RULE. The tag’s only parent-to-tag source addition is documentation, and the remediation report records a build from the parent implementation commit, but that is not a substitute for a fresh independent exact-tag build/equivalence proof. No GitHub release asset was present in the inspected remote refs.

## 20. DMG validation

Independent checksum/stat inspection observed:

```text
size: 6865018 bytes
mtime: 2026-09-13T23:16:37+0500
SHA-256: 2ada4eb6eb17ce6fbccc43a3ab92357d8affd773856d1681129bb383df419346
```

The macOS `file` command identified the artifact as zlib-compressed data. The embedded bundle was not independently mounted read-only, checked for the expected app, and unmounted after the first defect. The remediation report’s successful-mount claim is not independently accepted.

## 21. Packaged runtime

The remediation report claims an `open -n` smoke with a live app and graceful quit, but this audit did not independently launch the packaged app after the first defect. Status: **NOT RUN — STOP RULE**. No packaged-runtime PASS is claimed.

## 22. Source-owned mutation

No source-owned mutation probe was run after the stop. The remediation report records a metadata-only before/after signature, but that claim is not independently re-executed here. No secrets or auth contents were read or printed by this audit.

## 23. Codex live

The prior report states that this host has no active local Codex login and therefore cannot perform verified live quota acceptance. This audit did not rerun the opt-in live probe after the stop. Status: **BLOCKED_BY_ENVIRONMENT (prior environment evidence; not independently rerun after STOP RULE)**.

## 24. Claude live

The prior report states that a qualifying Claude true-subscription login is not available on this host, while safe-degradation behavior had been tested separately. This audit did not rerun either live probe after the stop. Status: **BLOCKED_BY_ENVIRONMENT (prior environment evidence; not independently rerun after STOP RULE)**.

## 25. Native notification

The normative native notification smoke was not independently performed in this audit. The prior report records it as unavailable on the host. Status: **BLOCKED_BY_ENVIRONMENT (prior environment evidence; not independently rerun after STOP RULE)**.

## 26. Launch-at-login

The normative native launch-at-login smoke was not independently performed in this audit. The prior report records the native qualification as a separate environment check. Status: **BLOCKED_BY_ENVIRONMENT (prior environment evidence; not independently rerun after STOP RULE)**.

## 27. Clean-install GUI

The clean-install → launch → tray → panel → cards → settings → quit sequence was not independently performed after the first defect. The prior report records the full GUI qualification as environment-dependent. Status: **BLOCKED_BY_ENVIRONMENT (prior environment evidence; not independently rerun after STOP RULE)**.

## 28. Storage integrity

No temporary upgraded DB was created or modified by this audit after the stop, and no independent `PRAGMA integrity_check`/`foreign_key_check` run was completed. The exact schema and migration sources were inspected statically; no release conclusion is promoted to PASS.

## 29. Security sweep

| Question | Independent audit result |
|---|---|
| Non-authoritative usage in authoritative total? | STATIC CODE REVIEW: no path found; required independent execution NOT RUN — STOP RULE. |
| Non-authoritative cost in authoritative total? | STATIC CODE REVIEW: no path found; required independent execution NOT RUN — STOP RULE. |
| Non-authoritative cost triggers budget? | STATIC CODE REVIEW: explicit authority filter present; budget execution NOT RUN — STOP RULE. |
| Raw/local analytics destroyed? | STATIC CODE REVIEW: raw and trust-aware helpers remain separate; execution NOT RUN — STOP RULE. |
| Legacy Claude Partial authoritative? | STATIC CODE REVIEW: legacy reclassification guards remain; regression suite NOT RUN — STOP RULE. |
| Diagnostics private string leak? | Node PII tripwire PASS; Rust subgate incomplete under STOP RULE. |
| Cross-profile contamination? | No static RC5 coverage-path defect found; multi-profile suite NOT RUN — STOP RULE. |
| Credential writeback/leak? | No credential contents were read; live/source-mutation probes NOT RUN — STOP RULE. |
| Monitoring inference? | No new inference path was found in the inspected RC5 diff; full suite NOT RUN — STOP RULE. |
| Money float regression? | Static cost path remains Decimal; runtime assertion NOT RUN — STOP RULE. |
| Release documentation accurately describes support/security? | **NO — P1 finding in tracked `README.md:3,9-10,22,24,26,36`.** |

## 30. Master Spec release matrix

`NOT RUN — STOP RULE` is used below only as an audit execution status. It is not a fourth acceptance status and must not be interpreted as PASS. The allowed normative statuses are otherwise used as requested.

| Master Spec requirement | Status | Evidence/qualification |
|---|---|---|
| Codex quota | NOT RUN — STOP RULE | Live/complete Rust qualification not independently completed. |
| Claude quota | NOT RUN — STOP RULE | Live/complete Rust qualification not independently completed. |
| Codex history | NOT RUN — STOP RULE | Targeted regression suite stopped. |
| Claude history | NOT RUN — STOP RULE | Targeted regression suite stopped. |
| Multi-profile isolation | NOT RUN — STOP RULE | Targeted A/B suite stopped. |
| Multi-profile quota | NOT RUN — STOP RULE | Live/targeted qualification stopped. |
| Authoritative usage totals | NOT RUN — STOP RULE | Required independent temporary dataset not executed. |
| Authoritative cost totals | NOT RUN — STOP RULE | Required independent Decimal dataset not executed. |
| Raw/local analytics preservation | NOT RUN — STOP RULE | Static path looks separated; execution stopped. |
| Credential ownership | NOT RUN — STOP RULE | Source mutation/live probes stopped. |
| Zero inference | NOT RUN — STOP RULE | No new static path found; full qualification stopped. |
| Trust/fidelity | NOT RUN — STOP RULE | Full trust regressions stopped. |
| Legacy upgrade trust | NOT RUN — STOP RULE | Migration regression stopped. |
| Diagnostics privacy | NOT RUN — STOP RULE | Node PASS; Rust PII subgate incomplete. |
| Exact Decimal money | PASS (static) | Decimal storage and aggregation path inspected; runtime gate incomplete. |
| SWR/LKG | NOT RUN — STOP RULE | Targeted suite stopped. |
| Adaptive scheduler | NOT RUN — STOP RULE | Targeted suite stopped. |
| Tray | NOT RUN — STOP RULE | Static prior guards visible; runtime suite stopped. |
| Notifications | NOT RUN — STOP RULE | Static prior guards visible; runtime suite stopped. |
| Budget | NOT RUN — STOP RULE | New production-path regression present but not independently executed. |
| `cargo fmt` | PASS | Exact detached worktree formatter check passed. |
| Locked reproducibility | PASS (pre-stop) | Metadata passed; lockfile diff empty before report creation. |
| CI | PASS (wiring) | Required portable/Intel commands present in `.github/workflows/ci.yml`; unrun jobs are not claimed. |
| Intel artifact | PASS (inspection) | Actual app executable is x86_64; metadata is 1.3.0. |
| Artifact provenance | NOT RUN — STOP RULE | No fresh exact-tag rebuild/equivalence proof completed. |
| Packaged smoke | NOT RUN — STOP RULE | No independent launch after finding. |
| Native notification | BLOCKED_BY_ENVIRONMENT | Prior host limitation only; not rerun. |
| Launch-at-login | BLOCKED_BY_ENVIRONMENT | Prior host limitation only; not rerun. |
| Clean-install GUI | BLOCKED_BY_ENVIRONMENT | Prior host limitation only; not rerun. |
| Independent audit | FAIL | P1 tracked README release-surface defect. |

The README is not a Master Spec feature row that can be hidden behind an otherwise green code matrix; it is part of the exact release candidate and directly affects source-fidelity/security expectations.

## 31. Remaining blockers

1. **P1 — tracked README release claims.** Exact evidence is `README.md:3,9-10,22,24,26,36`, contradicted by `Usage.ai_MASTER_SPEC_v1.3.md:164-189,204-209,221-227`, `crates/usage-providers/src/descriptor.rs:263-281`, `crates/usage-providers/src/discovery.rs:5-28`, `package.json:19-32`, and the plaintext SQLite schema in `crates/usage-storage/migrations/001_initial.sql:10-33`.
2. **Audit completion consequence, not a new defect:** the mandatory Rust/independent dataset/package/live checks after the finding are unexecuted because the supplied TЗ explicitly requires stopping on any defect. Their unexecuted status cannot be used to approve the candidate.
3. **Evidence warning:** the remediation report names a nonexistent executable path (`Contents/MacOS/Usage.ai`) although the actual bundle executable is `Contents/MacOS/usage-ai-app`. The actual binary is present and x86_64, but the recorded path should not be reused as provenance evidence without correction.

No production remediation was performed. No P0 runtime pollution, credential mutation, or PII leak was declared from the incomplete post-finding checks.

## 32. Final verdict

**RELEASE HOLD — REMEDIATION REQUIRED**

The candidate’s remote provenance and Master Spec provenance are sound, and the inspected RC5 coverage implementation statically addresses the previous substring fail-open class. Those positives do not override the P1 documentation/security-accuracy defect in a tracked file of the exact release candidate. `v1.3.0` must not be created or pushed from this candidate.

## 33. Exact next action

Do not create the final tag. In a separate remediation change, correct or substantiate the claims at `/Users/nuras/Desktop/usage.ai/README.md:3,9-10,22,24,26,36` (including explicit DiscoveryOnly wording for Gemini/Ollama, the actual Intel-only release scope, the actual non-Tailwind UI stack, and the actual SQLite/Keychain security boundary). Correct the artifact evidence path in `V1.3_RC5_COVERAGE_REMEDIATION_REPORT.md` as part of release-documentation hygiene if that report is retained. Then produce a new candidate and rerun the complete independent audit, including the stopped Rust gates, the required temporary authoritative usage/cost/budget dataset, read-only DMG mount, packaged smoke, and any available live/native qualifications. No tag/push command was executed or recommended for the held candidate.

FINAL TAG v1.3.0: HOLD — REMEDIATION REQUIRED
