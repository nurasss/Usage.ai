# Usage.ai v1.3.0-rc6 — Final Independent Read-Only Release Audit

Audit date: 2026-09-14 (Asia/Almaty)  
Candidate tag: `v1.3.0-rc6`  
Exact audited commit: `2216492a8f09004fe6b78db5f08d87a47c3fdebc`  
Remote: `https://github.com/nurasss/Usage.ai.git`  
Audit mode: independent, read-only; no remediation performed

## 1. Executive verdict

**RELEASE HOLD — REMEDIATION REQUIRED.**

The RC6 production source, documentation correction, authoritative aggregation policy, mandatory non-live gates, SQLite/Decimal behavior, Intel artifact, DMG integrity, packaged launch smoke, and source-store non-mutation probe all passed the checks completed before the stop condition.

The audit then executed the repository's opt-in real Codex acceptance test on an available logged-in Codex environment. The test failed with:

```text
live quota fetch: Parse { schema: "codex-account-identity" }
```

The failure was isolated to a concrete P2 defect in the candidate's ignored live acceptance test: it constructs `FetchContext` with `local_root: None`, while the production coordinator scopes the real profile root and the strategy intentionally uses that root as its safe identity fallback. An independent production-equivalent live harness using the real default `.codex` scoped root succeeded with `availability=READY`, non-empty quota, identity present, and `Coverage::Complete` against installed `@openai/codex@0.144.5`.

This is not evidence of a shipped data-integrity/security failure, but it is a real candidate acceptance-test defect. Under the supplied verdict rule (“actual candidate has a concrete P1/P2 defect”) and first-defect STOP RULE, RC6 cannot be approved and no further release qualification was performed after classification.

## 2. Remote RC6 provenance

Independent remote resolution produced:

```text
origin URL:             https://github.com/nurasss/Usage.ai.git
origin/main:            2216492a8f09004fe6b78db5f08d87a47c3fdebc
v1.3.0-rc6^{commit}:    2216492a8f09004fe6b78db5f08d87a47c3fdebc
remote tag object:      e038c7937cde8070b80d6dc660d8d3b3491876ed
remote peeled commit:   2216492a8f09004fe6b78db5f08d87a47c3fdebc
```

`v1.3.0-rc6` is an annotated tag, is an ancestor of `origin/main`, and resolves exactly to `origin/main`. The commit subject is `docs(v1.3): record rc6 documentation remediation and release gate`; its parent is `b5c893036b829f291d11000517c4ed6a35cc3b41`.

No local or remote final tag `v1.3.0` existed during the audit. No tag, commit, or push was created.

## 3. Clean detached tree

All source conclusions and mandatory gates were taken from a detached temporary worktree at exact tag commit `2216492a8f09004fe6b78db5f08d87a47c3fdebc`.

Before testing:

```text
git rev-parse HEAD                 2216492a8f09004fe6b78db5f08d87a47c3fdebc
git describe --tags --exact-match v1.3.0-rc6
git status --porcelain            <empty>
```

The primary checkout contained one pre-existing untracked RC5 audit file, `TECHNICAL_AUDIT_V1.3_RC5_FINAL.md`. It was preserved and was not used as the candidate tree. The detached worktree and audit-owned temporary build/harness files were removed before this report was created.

## 4. Master Spec provenance

Normative document:

```text
Usage.ai_MASTER_SPEC_v1.3.md
SHA-256: 38c5320684a694f24b5546f700453d23093a8340f5a2a1d4281480cb776d7d87
```

It is unchanged from RC5. The audit used it as the release contract rather than treating remediation reports as authority.

The relevant V13-07 release gate explicitly labels clean install, launch-at-login, tray/window, notifications, and packaged non-mutation as “Required validation” (`Usage.ai_MASTER_SPEC_v1.3.md:1609-1629`). Its acceptance matrix also requires production-ready Codex/Claude quota, exact OpenAI API Decimal cost, isolation, safe credentials, authoritative tray/notifications, a smoke-tested Intel package, green gates, and an independent audit (`:1688-1707`).

## 5. RC5→RC6 documentation-only diff

Independent diff from `v1.3.0-rc5` to `v1.3.0-rc6` contains only:

```text
M README.md
M V1.3_RC5_COVERAGE_REMEDIATION_REPORT.md
A V1.3_RC6_DOCUMENTATION_REMEDIATION_REPORT.md
```

No Rust, TypeScript, Svelte, SQL migration, CI workflow, Tauri configuration, `Cargo.lock`, `package-lock.json`, or Master Spec file changed.

The narrower diff from build-input parent `b5c893036b829f291d11000517c4ed6a35cc3b41` to the RC6 tag adds only `V1.3_RC6_DOCUMENTATION_REMEDIATION_REPORT.md`. Therefore RC6 satisfies the supplied documentation-only artifact-equivalence rule.

## 6. README accuracy

The prior RC5 documentation defects are corrected in RC6:

- production scope is limited to Codex, Claude Code, and configured OpenAI API;
- Gemini/Gemini API and Ollama are explicitly discovery/backlog only and have no production drivers;
- authoritative provider-facing totals are limited to canonical `Complete` and `Partial` coverage;
- storage is described as ordinary local SQLite, with Usage.ai secrets in Keychain where applicable; no encrypted-SQLite claim remains;
- frontend is described as Svelte 5/TypeScript/Vite with lightweight custom CSS; no Tailwind claim remains;
- release platform is Intel macOS x86_64 only; no Apple Silicon/universal claim remains;
- native notification, launch-at-login, clean-install GUI, and Claude true-subscription positive are explicitly identified as separate environment qualifications.

Source cross-checks agree: Gemini is `DiscoveryOnly` with no strategies, Ollama has no production strategy, `package.json` contains no Tailwind dependency, and `tauri.conf.json` targets `.app`/DMG with minimum macOS 13.0.

Status: **PASS**.

## 7. Mandatory gates

Completed independently at exact RC6:

| Gate | Result | Evidence |
|---|---:|---|
| `cargo fmt --all -- --check` | PASS | no diff/output |
| `npm ci` | PASS | 137 packages installed; lockfile unchanged |
| `npm run check` | PASS | Svelte check: 0 errors, 0 warnings; version 1.3.0 consistent |
| `npm test -- --run` | PASS | 5 files, 47 tests, 0 failures |
| `npm run build` | PASS | Vite 7.3.6, 144 modules; production dist created |
| `npm run check:pii` | PASS | Node tripwire clean; Rust subgate 4/4 passed |
| `cargo check --workspace --locked` | PASS | exact locked workspace |
| `cargo test --workspace --locked` | PASS | 306 passed, 0 failed, 4 ignored |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS | 0 warnings/errors |
| `cargo metadata --locked --no-deps --format-version 1` | PASS | lock resolution accepted |
| `git diff --check v1.3.0-rc5..v1.3.0-rc6` | PASS | no whitespace errors |

The first Rust test and first clippy attempts were interrupted only by `No space left on device` while writing temporary build outputs. Audit-owned targets were cleaned and both commands were repeated successfully with `CARGO_INCREMENTAL=0`; clippy additionally used one build job. These resource controls do not alter source, dependencies, lockfiles, warnings, or test semantics.

`npm audit --omit=dev` reported zero production vulnerabilities. Full `npm audit` reported two moderate entries in the dev-only Vitest/`@vitest/mocker` chain (GHSA-82fw-gwwq-j7x9). No automatic fix or dependency change was performed. The affected package is not part of the shipped Tauri runtime; this is recorded as dependency-hygiene information, not the stop finding.

## 8. Test-count reconciliation

Actual Rust package counts:

| Package | Passed | Failed | Ignored |
|---|---:|---:|---:|
| `usage-ai-app` | 20 | 0 | 0 |
| `usage-core` | 19 | 0 | 0 |
| `usage-host` | 52 | 0 | 0 |
| `usage-providers` | 99 | 0 | 4 |
| `usage-runtime` | 80 | 0 | 0 |
| `usage-storage` | 36 | 0 | 0 |
| **Total** | **306** | **0** | **4** |

All doc-test targets passed with zero tests. Frontend count is exactly 47.

The four ignored tests are:

| Test | Purpose | Why ignored | Audit run | Release consequence |
|---|---|---|---|---|
| `codex::tests::real_codex_corpus_probe_is_opt_in_and_never_checked_in` | external real/redacted Codex corpus | requires explicit corpus path | not run | no corpus was supplied |
| `codex_appserver::live_acceptance::live_app_server_reports_verified_quota` | real logged-in Codex quota | requires local login and opt-in | **run; failed** | P2 stop finding described in §26 |
| `claude::live_acceptance::live_auth_status_shape_parses` | real Claude auth shape | requires local Claude install and opt-in | not run after stop | `NOT_RUN_STOP_RULE` |
| `claude::live_acceptance::live_quota_end_serves_or_degrades_cleanly` | real Claude quota/degradation | requires local Claude install and opt-in | not run after stop | `NOT_RUN_STOP_RULE` |

No new test was hidden by ignore: the static ignored list and runtime count both equal four.

## 9. Authoritative usage

An external temporary Rust harness linked the exact RC6 `usage-core`, `usage-storage`, `usage-host`, `usage-providers`, and `usage-runtime` path crates. Its registry dependency set was constrained to versions present in the exact RC6 `Cargo.lock` (`REGISTRY_ONLY_IN_HARNESS=[]`). It invoked production `Storage` and runtime aggregation APIs rather than duplicating the SQL policy.

Dataset for account A/product Codex:

```text
Complete                 100
Partial                   50
Unknown                 1000
LocalClientOnly         2000
UnverifiedSemantics     4000
```

A separate account B contained 75 authoritative tokens.

Observed results:

```text
account A raw tokens              7150
authoritative account A tokens     150
global authoritative overview      225
global unverified overview         7000
account isolation                  PASS
```

Status: **PASS**. Only exact `Complete` and `Partial` rows reach provider-facing totals.

## 10. Authoritative costs

Production API dataset for account A/product Codex:

```text
Complete               USD 1.10
Partial                USD 0.40
Unknown                USD 10.00
LocalClientOnly        USD 20.00
UnverifiedSemantics    USD 40.00
```

Account B contained separate authoritative `USD 3.25`.

Observed results:

```text
account A raw cost              USD 71.50
account A authoritative cost    USD 1.50
account B authoritative cost    USD 3.25
account/product isolation       PASS
```

Both `account_usage_and_cost_today` and `build_aggregates` returned the authoritative values and excluded non-authoritative costs.

Status: **PASS**.

## 11. Decimal money

The harness inserted and summed `rust_decimal::Decimal` values through production storage/runtime APIs. Exact comparisons succeeded for `71.50`, `1.50`, `3.25`, and `24.00`. Static inspection found no binary-float conversion in cost aggregation or budget spend paths; `f64` use is limited to percentage/layout-style domains.

Status: **PASS**.

## 12. Budget trust

`src-tauri/src/commands/notifications.rs` filters budget spend by account, product, currency, `CostKind::Reported`, and `coverage.is_authoritative()`. The mandatory suite independently executed `budget_spend_path_excludes_non_authoritative_rows`, which passed. The central coverage method accepts only `Complete | Partial`.

Status: **PASS** for the implemented budget trust path.

## 13. Raw/local analytics

Raw storage reads intentionally remain broad. In the external dataset, raw usage remained 7150 while authoritative usage was 150; raw cost remained USD 71.50 while authoritative cost was USD 1.50.

A separate persisted-invalid-coverage database retained raw rows and raw coverage text in CSV export while granting them no authority:

```text
raw invalid-row cost       USD 24.00
authoritative cost         USD 0.00
raw export values retained PASS
```

No destructive filtering or history deletion was observed.

Status: **PASS**.

## 14. SQL/domain parity

The domain policy is centralized at `Coverage::is_authoritative()` and accepts only `Complete` and `Partial`. Verified SQL helpers use exact `IN ('Complete','Partial')`; unverified helpers use the complementary exact set behavior. Cost records cross the storage boundary through an exact parser with explicit current enum spellings and default unknown/future strings to `Coverage::Unknown`.

External persisted strings `legacy-invalid-Complete`, `Partial-suffix`, and `garbage` all decoded as `Unknown` and contributed zero authoritative cost. No substring, prefix, suffix, case-folding, or whitespace-normalizing fail-open parser was found.

Status: **PASS**.

## 15. RC3 trust regression

The full mandatory suite passed the legacy Claude reclassification, tray suppression, notification suppression, mixed verified/unverified behavior, and full side-effect regression tests. A legacy Claude `Partial` snapshot is reclassified before DTO/tray selection; synthetic/unverified Claude values cannot become ordinary authoritative tray values or quota notifications.

Status: **PASS**.

## 16. Diagnostics privacy

`npm run check:pii` passed both the repository tripwire and production serialized-output Rust tests. The suite covered allowlisted copy text, negative controls, export/copy equivalence, safe-field preservation, and production export bytes. Static host/process policy inspection found bounded process output, scoped files, allowlisted executables/hosts, credential scrubbing, and no raw provider payload logging path introduced by RC6.

Status: **PASS**.

## 17. Multi-profile

The full suite passed A/B/A profile isolation, default/custom-root isolation, Codex and Claude cross-profile non-observation, first-seen/mismatch routing, disable/delete cancellation, source-connection isolation, and distinct account/product cost aggregation. The external dataset independently kept account A and B usage/cost separate.

Status: **PASS**.

## 18. SWR/LKG

The full suite passed stale-while-revalidate preservation, last-known-good serving after failure, corrupt snapshot fail-safe handling, persisted cooldown restart, rate-limit cooldown, offline handling, coalescing, cancellation, and disable/delete pre-commit checks.

Status: **PASS**.

## 19. Storage integrity

The external file-backed SQLite database was opened/migrated through production `Storage::open`, populated through production APIs, modified only in the audit-owned temporary database to simulate legacy-invalid persisted coverage, reopened through production storage, and checked before and after:

```text
PRAGMA integrity_check     ok
PRAGMA foreign_key_check  0 rows
```

The database and harness were deleted after the audit. No user database was opened or modified.

Status: **PASS**.

## 20. CI/reproducibility

`.github/workflows/ci.yml` wires the portable checks (`npm ci`, Svelte/version check, PII, frontend tests/build, fmt check, locked Rust check, all-targets clippy with `-D warnings`, locked tests) and an Intel `macos-13` job for x86_64 check/test/build/Tauri bundle verification. The release path checks the binary architecture and verifies bundle presence. No broad publish/upload step that could select an arbitrary stale DMG was found.

`Cargo.lock` and `package-lock.json` remained byte-unchanged after all completed gates. `cargo metadata --locked` passed.

Status: **PASS**.

## 21. Build-SHA vs tag-SHA provenance

Candidate tag SHA:

```text
2216492a8f09004fe6b78db5f08d87a47c3fdebc
```

Recorded artifact build-input SHA:

```text
b5c893036b829f291d11000517c4ed6a35cc3b41
```

Independent Git comparison from that build input to the tag contains only the RC6 documentation remediation report; no production source, lockfile, migration, build configuration, or CI file differs. This meets the audit's explicit documentation-only equivalence rule.

A second exact-tag release rebuild was not practical on this host: mandatory Rust targets twice demonstrated a roughly 3 GiB temporary-disk ceiling, while the preserved Intel release target itself occupies approximately 1.6 GiB. The existing release target was not overwritten or cleaned.

Status: **PASS under the supplied artifact-equivalence rule**, with the build-input attribution originating from the RC6 provenance record and the source-equivalence claim independently verified.

## 22. Intel app/DMG

Validated artifact paths:

```text
App: /tmp/usageai-rc6-final-target/x86_64-apple-darwin/release/bundle/macos/Usage.ai.app
Bin: /tmp/usageai-rc6-final-target/x86_64-apple-darwin/release/bundle/macos/Usage.ai.app/Contents/MacOS/usage-ai-app
DMG: /tmp/usageai-rc6-final-target/x86_64-apple-darwin/release/bundle/dmg/Usage.ai_1.3.0_x64.dmg
```

Independent measurements:

```text
Executable size:     19,978,528 bytes
Executable mtime:    2026-09-14T02:05:53+0500
Executable mode:     0755
Executable SHA-256:  b9104b36fbe365d73ac377f1dc8ff26b6a739342606fae6e345b334e5f01ef6a
Executable file:     Mach-O 64-bit executable x86_64

DMG size:            6,872,837 bytes
DMG mtime:           2026-09-14T02:06:20+0500
DMG mode:            0644
DMG SHA-256:         4a4601ae8eaeb34421e536520258ad6f8bc6b0da4f7871466c9fa9cc1253abb9
DMG format:          UDIF read-only compressed (UDZO/zlib)
hdiutil verify:      VALID
```

Read-only mount contained `Usage.ai.app` and the `Applications` link. The embedded executable is x86_64 and has the same SHA-256 as the outer app bundle executable. Embedded plist values are:

```text
CFBundleIdentifier          com.nurasss.usageai
CFBundleShortVersionString  1.3.0
CFBundleVersion             1.3.0
```

Status: **PASS**.

## 23. Signing scope

`tauri.conf.json` has `signingIdentity: null`. Independent `codesign` inspection reports `code object is not signed at all`. The host's `spctl` returned `accepted` only with `source=no usable signature` and `override=security disabled`; therefore that result is not accepted as Gatekeeper evidence. The audit did not change or disable any security setting.

The Master Spec defines the current distribution as a local Intel `.app`/DMG and does not make Developer ID signing/notarization an explicit V13-07 acceptance item. Unsigned status is therefore recorded, not converted into a P0/P1 finding. A quarantined clean-install qualification remains required separately.

## 24. Packaged smoke

The exact RC6 packaged executable was launched via its `.app` bundle as a new process. It remained alive at 5 seconds and 30 seconds and exited after `SIGTERM` without requiring `SIGKILL`.

```text
ALIVE_5S=yes
ALIVE_30S=yes
TERM_EXIT=yes
```

No pre-existing Usage.ai process was terminated. No audit-owned packaged process or DMG mount remained after cleanup.

Status: **PASS** for non-visual packaged launch/crash smoke.

## 25. Source-store mutation

A metadata-only before/after signature covered Codex auth/config, historical Codex sessions/archived sessions, Claude config, and Claude project history. File contents and secrets were never printed. The active current-day Codex session subtree was excluded because this audit host itself appends to it.

The initial broad signature observed concurrent-host noise. A controlled repeat around a separately launched packaged process produced:

```text
CHANGED_COUNT=0
```

No Usage.ai-caused mutation of the accepted source set was observed.

Status: **PASS**, with the concurrent-host caveat explicitly handled.

## 26. Codex live

Installed executable:

```text
/Users/nuras/.npm-global/bin/codex
@openai/codex@0.144.5
```

The opt-in ignored test was run with `USAGE_AI_LIVE_CODEX=1`. It performs only `initialize`, `account/read` with `refreshToken=false`, and `account/rateLimits/read`; it sends no inference request.

Actual result:

```text
codex_appserver::live_acceptance::live_app_server_reports_verified_quota ... FAILED
live quota fetch: Parse { schema: "codex-account-identity" }
0 passed; 1 failed; 102 filtered out
```

### Finding RC6-P2-01 — live acceptance context diverges from production

Severity: **P2** (release-evidence/test defect; no demonstrated shipped runtime failure).

Evidence chain:

1. `crates/usage-providers/src/codex_appserver.rs:1657-1671` creates the live test `FetchContext` with file access enabled but hard-codes `local_root: None`.
2. `codex_appserver.rs:715-728` intentionally falls back from App Server account identity to the scoped profile's `auth.json`; that fallback is impossible when the test supplies `None`.
3. `crates/usage-runtime/src/refresh.rs:393-405` shows the production coordinator calling `scoped_product_root` and passing the resulting root into `FetchContext`.
4. On the installed Codex version, quota RPCs were available but the App Server response did not provide the account identity shape expected by the test-only context, producing the exact `codex-account-identity` error.
5. An independent temporary harness using the production default `.codex` scoped root, exact RC6 crates, exact locked dependency versions, the same safe RPC strategy, and no inference request returned:

```text
CODEX_PRODUCTION_CONTEXT availability=READY quota_nonempty=yes identity_present=yes coverage=Complete
```

Thus the shipped default production path is positively qualified on this host, but the candidate's named opt-in acceptance test does not reproduce it and fails its stated acceptance purpose. This is a concrete candidate defect, so the STOP RULE applies.

Production Codex live path: **PASS**.  
Repository opt-in live acceptance test: **FAIL / P2**.

## 27. Claude live

`claude` was present at `/Users/nuras/.npm-global/bin/claude`, but both Claude opt-in tests were not executed after RC6-P2-01 activated the mandatory STOP RULE.

```text
CLAUDE_SAFE_DEGRADATION:       NOT_RUN_STOP_RULE
CLAUDE_TRUE_SUBSCRIPTION_LIVE: NOT_RUN_STOP_RULE
```

No positive Claude subscription claim is made.

## 28. Native notification

Static implementation and mandatory unit/integration tests for authoritative-only notification planning, profile labels, mute, quiet hours, deduplication, thresholds, pace, reset, and budget trust passed. An actual native visual/event notification smoke was not performed before the stop condition.

```text
NATIVE_NOTIFICATION: NOT_RUN_STOP_RULE
```

This remains an explicit Master Spec V13-07 required validation item.

## 29. Launch at login

The Tauri autostart plugin is wired and settings code enables/disables it through the native manager. No enable/register/relaunch/restore smoke was performed before the stop condition.

```text
LAUNCH_AT_LOGIN: NOT_RUN_STOP_RULE
```

This remains an explicit Master Spec V13-07 required validation item.

## 30. Clean-install GUI

The DMG checksum, read-only mount, package layout, executable architecture, plist, and packaged process launch passed. A quarantined drag-install into an isolated test location followed by visual tray/panel/provider-card/settings/quit checks was not performed before the stop condition. Gatekeeper was not disabled by this audit.

```text
CLEAN_INSTALL_GUI: NOT_RUN_STOP_RULE
TRAY_WINDOW_VISUAL: NOT_RUN_STOP_RULE
```

These remain explicit Master Spec V13-07 required validation items.

## 31. Master Spec release matrix

| Area | Independent status | Basis |
|---|---|---|
| Codex quota production path | PASS | real safe production-context App Server probe |
| Codex opt-in acceptance test | FAIL / P2 | `local_root=None` diverges from production |
| Claude quota | NOT_RUN_STOP_RULE | live tests stopped after first defect |
| Codex history | PASS | full suite, evidence fixtures, raw preservation |
| Claude history | PASS | full suite, generation/dedup/cache regressions |
| OpenAI API cost | PASS | exact Decimal, pagination, independent aggregation dataset |
| Multi-profile | PASS | full suite plus independent A/B dataset |
| Authoritative usage totals | PASS | production storage/runtime harness |
| Authoritative cost totals | PASS | production storage/runtime harness |
| Raw/local analytics | PASS | raw totals and export preserved |
| Budget trust | PASS | exact authoritative filters and regression test |
| Credential ownership | PASS | scoped/read-only design; no observed writeback |
| Zero inference | PASS | source design and safe Codex RPC method allowlist |
| Source fidelity | PASS | coverage and source classifications preserved |
| Legacy Claude upgrade | PASS | migration/reclassification regressions |
| Diagnostics privacy | PASS | PII tripwire and production export tests |
| Decimal money | PASS | exact Decimal harness and code path |
| SWR/LKG | PASS | full suite |
| Adaptive scheduler | PASS | cooldown/offline/coalescing/cancellation suite |
| Tray policy | PASS | authoritative-only tests; visual smoke not reached |
| Notification policy | PASS | authoritative-only tests; native smoke not reached |
| fmt | PASS | exact candidate |
| Locked reproducibility | PASS | check/test/clippy/metadata; lockfiles unchanged |
| CI | PASS | portable and Intel gates wired |
| README accuracy | PASS | prior P1 claims corrected and source-checked |
| Intel artifact | PASS | x86_64 app and valid DMG |
| Artifact provenance | PASS (docs-only equivalence) | build input→tag has no production diff |
| Packaged smoke | PASS | alive 5s/30s, TERM exit |
| Native notification | NOT_RUN_STOP_RULE | required qualification remains |
| Launch at login | NOT_RUN_STOP_RULE | required qualification remains |
| Clean-install GUI/tray/window | NOT_RUN_STOP_RULE | required qualification remains |
| Independent audit | HOLD | concrete P2 triggered STOP RULE |

## 32. Security final sweep

```text
Authoritative usage polluted?             NO
Authoritative cost polluted?              NO
Unknown cost can trigger budget?          NO
Raw/local history destroyed?              NO
Legacy Claude Partial authoritative?      NO
Diagnostics private-string leak?          NO
Cross-profile contamination?              NO
Credential mutation/writeback?            NO
Monitoring inference generated?           NO
Money float regression?                   NO
README overclaims shipped/security scope? NO
```

No unsafe `YES` was found. RC6-P2-01 concerns acceptance-test fidelity, not a demonstrated production security/data-integrity failure.

## 33. Remaining blockers

One concrete candidate defect blocks approval under the supplied rules:

```text
RC6-P2-01: opt-in Codex live acceptance test is not production-equivalent and fails on the available real Codex client.
```

Because the stop condition occurred before environment qualification, the following Master-Spec-required checks also remain unqualified for any subsequent final audit:

```text
native notification visual/event smoke
launch-at-login enable/register/relaunch/restore smoke
clean-install GUI plus tray/window smoke
```

Claude safe degradation and true-subscription positive were not run after the stop; the Master Spec requires a production-ready Claude primary/fallback strategy but does not separately name a true-subscription live positive as a V13-07 standalone checklist item. No positive claim is made here.

## 34. Final verdict

RC6 has no reproduced P0/P1 production data-integrity, security, trust, aggregation, Decimal, artifact, or packaged-runtime defect. Nevertheless, the repository contains a concrete P2 acceptance-test defect and therefore fails the user's explicit approval rule. The correct verdict is:

**RELEASE HOLD — REMEDIATION REQUIRED.**

This is not an environmental-only hold: the P2 exists in the candidate test code. The remaining native environment checks are additionally incomplete because the mandated STOP RULE prevented continuation.

## 35. Exact next action

Do not create or push `v1.3.0` from RC6. Close finding `RC6-P2-01` in a subsequent release candidate, then run a new independent final read-only audit from the exact remote candidate, including the still-required native notification, launch-at-login, and clean-install tray/window qualifications. No production remediation is implied by this audit, and no remediation was performed.

FINAL TAG v1.3.0: HOLD — REMEDIATION REQUIRED