# Usage.ai v1.3.0-rc3 — Final independent read-only release audit

Audit date: 2026-09-13 (Asia/Almaty)  
Audited tag: `v1.3.0-rc3`  
Audited commit: `90ca0dd34b13d14aead6d8539cde73c264e7a25a`  
Audit mode: exact detached clean worktree; no production, test, fixture, migration, lockfile, spec, README, CI, or artifact changes.

## 1. Executive verdict

`90ca0dd` must not be final-tagged yet.

The P0-1 legacy Claude trust regression and P0-2 diagnostics privacy regression both pass the independent checks below. All requested locked build/test gates pass, but the repository’s actual CI workflow also runs `cargo fmt --all -- --check`, and that command exits 1 on the exact RC3 tree. This is an unresolved P1 release-gate defect, so the applicable verdict is `RELEASE HOLD — REMEDIATION REQUIRED`.

There are also environment-only qualifications that remain incomplete: Claude true-subscription positive qualification, native notification smoke, launch-at-login smoke, and clean-install GUI smoke. They are recorded as environmental blockers, not converted into code findings.

No commit, push, final tag, formatter, remediation, or user-checkout cleanup was performed.

## 2. RC3/tag provenance

- Local annotated tag `v1.3.0-rc3` peels to `90ca0dd34b13d14aead6d8539cde73c264e7a25a`; its tag object is `8282f2814f5ce606474f2cae8172e323812d28c5`.
- `origin/main` is the same full commit. `git merge-base --is-ancestor v1.3.0-rc3 origin/main` passed.
- Remote refs contain RC1, RC2, RC3, and `main`; no local or remote `v1.3.0` tag was found.
- Peeled succession is monotonic: RC1 `bd0ca57`, RC2 `1a69113`, RC3 `90ca0dd`.
- RC3 itself is a docs-only commit after the production remediation commit `ce5f48f`; `git diff --name-status ce5f48f..90ca0dd` contains only `V1.3_RC3_REMEDIATION_REPORT.md`.

## 3. Master Spec provenance

`Usage.ai_MASTER_SPEC_v1.3.md` is absent from RC1, was introduced by RC2, and is byte-identical in RC2 and RC3. Independent SHA-256:

```text
38c5320684a694f24b5546f700453d23093a8340f5a2a1d4281480cb776d7d87
```

The RC3 remediation report does not record this spec hash and incorrectly labels `ce5f48f` as “candidate HEAD”. Those are report-quality inconsistencies, not changes to the tracked spec. Comparing the tracked requirements with the implementation found no weakening of the trust, read-only ownership, no-inference, diagnostics privacy, migration safety, exact-money, Intel packaging, or release-gate requirements. The RC3 change narrows legacy Claude trust and export surfaces; it does not redefine `Partial` globally.

## 4. Clean worktree

Before any audit gate, the detached RC3 worktree had an empty `git status --porcelain`. It remained clean after all gates, live tests, artifact checks, and source inspection. The audit report was not created in that release worktree.

The user’s primary checkout was not changed. Its pre-existing dirty paths remain exactly:

```text
README.md
TECHNICAL_AUDIT_V1.3_RC1_RELEASE.md
TECHNICAL_AUDIT_V1.3_RC2_RELEASE.md
```

The dirty README is not in the RC3 tag and was excluded from all exact-tree checks and artifact provenance.

## 5. Mandatory locked gates

| Gate | Independent result | Notes |
|---|---|---|
| `npm ci` | PASS | Lockfile unchanged; npm reported two moderate advisory vulnerabilities. |
| `npm run check` | PASS | Zero errors; one existing accessibility warning in `App.svelte`; version consistency `1.3.0`. |
| `npm test -- --run` | PASS | 47 passed. |
| `npm run build` | PASS | Vite build completed; only the same non-fatal warning set. |
| `cargo check --workspace --locked` | PASS | Completed on exact RC3. |
| `cargo test --workspace --locked` | PASS | 297 passed, 4 ignored. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS | Zero warnings. |
| `npm run check:pii` | PASS | Repository scan plus four production tripwire tests passed. |
| CI’s `cargo fmt --all -- --check` | FAIL | Actual CI portable job gate; formatting diffs remain in eight source files. |

The requested locked gates therefore pass, but the full automatic CI contract does not.

## 6. Test-count reconciliation

Independent Rust breakdown from the full locked run:

| Crate/target | Passed | Ignored |
|---|---:|---:|
| `usage-ai-app` library | 19 | 0 |
| `usage-core` | 19 | 0 |
| `usage-host` | 50 | 0 |
| `usage-providers` | 96 | 4 |
| `usage-runtime` | 79 | 0 |
| `usage-storage` | 34 | 0 |
| **Rust total** | **297** | **4** |
| Frontend Vitest | **47** | 0 |

The RC1/RC2/RC3 totals are respectively 275, 293, and 297 passed. The net RC3 increase is real, but it is not a one-to-one list of four new functions because the change also refactors and replaces nearby coverage. The added/changed tests are in the claimed areas: legacy storage/runtime trust reclassification, DTO/tray side effects, diagnostics allowlist/export, and negative tripwire behavior. Existing restart assertions are folded into the storage migration regression.

The four ignored tests were enumerated independently:

| Test | Requirement | Audit run | Result | Release consequence |
|---|---|---|---|---|
| `claude::live_acceptance::live_auth_status_shape_parses` | Real Claude CLI status shape | Opt-in live run | PASS | None. |
| `claude::live_acceptance::live_quota_end_serves_or_degrades_cleanly` | Bounded Claude quota/degradation path | Opt-in live run | PASS | Safe degradation qualified; true subscription remains separate. |
| `codex::tests::real_codex_corpus_probe_is_opt_in_and_never_checked_in` | User-supplied real/redacted corpus | Not run; no `CODEX_REAL_FIXTURE` supplied | BLOCKED_BY_ENVIRONMENT | Optional corpus evidence only; no CI release failure. |
| `codex_appserver::live_acceptance::live_app_server_reports_verified_quota` | Real Codex verified quota | Opt-in live run | PASS | None. |

## 7. Legacy Claude P0 regression

An RC1-equivalent persisted snapshot was independently created with Anthropic/Claude Code, a synthetic 20% remaining quota, and `Coverage::Partial`. Opening it with RC3 `Storage` changed only trust classification to `UnverifiedSemantics`; the quota payload remained present.

The temporary production-equivalent run reported:

```text
rc1_to_rc3_migration=PASS
reopen_persistence=PASS
schema_version=10
integrity_check=ok
foreign_key_violations=0
legitimate_non_claude_partial=Preserved
```

The real local Usage.ai database was also checked read-only: `user_version=10`; Claude Code snapshot rows contained 201 `UnverifiedSemantics` and 0 `Partial`; the persisted `app_snapshot` Claude Code entry was `UnverifiedSemantics`. No legacy Claude `Partial` remained authoritative in the observed physical state.

## 8. Migration 010

`crates/usage-storage/migrations/010_claude_trust.sql` is an intentional schema-version marker; the typed reclassification is implemented in `crates/usage-storage/src/lib.rs` and is executed inside `Storage::migrate`.

The exact behavior is:

- Snapshot rows are selected only where the database `product_id` is `claude-code`.
- The JSON walker changes only a top-level `productId == claude-code` plus `coverage == Partial`, or entries with that same product/coverage pair inside `providers[]`.
- Only the exact `app_snapshot` cache key is inspected.
- There is no broad `UPDATE ... WHERE coverage = 'Partial'`; OpenAI API `Partial` rows remain unchanged.
- Migration uses an immediate transaction, updates `user_version` only after the work succeeds, commits atomically, and creates a pre-migration backup for an existing older database.
- Empty/current/already-reclassified databases are safe and idempotent; the temporary run and the existing storage tests cover reopen behavior.

Audit precision note: the storage JSON walker does not independently require `providerId == anthropic` or inspect a quota-source predicate. The runtime and DTO guards do require the provider/product/coverage triple, while `Snapshot` has no separate source field. Current product descriptors make `claude-code` an Anthropic product, all production Claude quota constructors emit `UnverifiedSemantics`, and the conservative downgrade creates no observed trust escalation. This is recorded as a scope limitation, not an observed P0/P1 release failure.

## 9. Defensive trust guards

All three claimed guards were found and exercised through the relevant tests:

1. `crates/usage-runtime/src/snapshot.rs::stale_view_from_lkg` parses persisted LKG data, marks it stale, then reclassifies the legacy Claude triple before it becomes a runtime view.
2. `src-tauri/src/refresh_flow.rs::outcome_to_dto` applies the DTO guard after fresh, fallback, stale, and error metadata are assembled, preventing a raw `Partial` from reaching the UI DTO.
3. `src-tauri/src/commands/snapshot.rs::get_snapshot` guards both the persisted `app_snapshot` load and the already-cached return path; the storage-cache branch stores the guarded value back into memory.

The in-memory cached branch returns a guarded copy without overwriting the cached object. In production, every writer of that cache is already guarded (`run_full_refresh` or the guarded storage-cache branch), and tray/notification/overview are not fed by an unguarded alternate path. Code search of snapshot loads, DTO mapping, LKG handling, cache handling, tray selection, and notification planning found no fourth production load path that bypasses all three defenses.

## 10. Partial semantics regression

The global policy remains `Complete | Partial` authoritative; `UnverifiedSemantics`, `Unknown`, and other non-authoritative states are excluded from ordinary authoritative totals, tray selection, and quota notifications.

The legitimate non-Claude case was independently verified: OpenAI API bounded/truncated cost data remains `Coverage::Partial` and authoritative according to the existing policy. The runtime filtered test passed, and the migration walker test preserved a non-Claude `Partial`.

The mixed regression also passed: a verified Codex value at 61% plus legacy Claude at 20% produces tray metric 61%; Claude produces no quota/pace/reset notices; normal Codex notification planning remains eligible.

## 11. Diagnostics v3 architecture

The production path is an explicit allowlist:

```text
raw DiagnosticsDto / ImportStatsDto
  -> sanitize_diagnostics
  -> SanitizedBundle
  -> copy text or JSON/file payload
```

The v3 account surface is exactly: `provider`, `product`, `accountId`, `selectedSource`, `connectionState`, `freshness`, `coverage`, `statusClass`, `connectorVersion`, `parserVersion`, `schemaFingerprint`, `capabilitiesDetected`, `cooldownUntil`, `lastRefreshAttempt`, `lastSuccessfulRefresh`, `lastDataObservedAt`, `lastSafeErrorCode`, and `recentAttempts`. Each attempt has `source`, `status`, `safeCode`, `finishedAt`, and `latencyMs`. Import output has `productId` and numeric counters only. The file envelope adds fixed `schemaVersion: 3`, generated timestamp, offline boolean, accounts, and imports.

Field classification and provenance:

- Coverage/connection/freshness/status/capability values are enum or fixed descriptor names.
- Counts and latency are numeric.
- Timestamps are RFC3339 operational timestamps.
- `accountId` is the internal UUID identifier, not a user label.
- Provider/product/source/version/fingerprint/error strings are internal descriptor, strategy, parser, schema, or fixed safe-code values; they are not process output.
- Alias, identity note, warnings, raw error messages, stdout/stderr, executable paths, user labels, workspace/project names, prompts, and import warnings have no v3 output field.

There is no raw DTO serialization followed by regex cleanup, no object spread, and no unknown-field copy. Copy and export both call the same sanitizer; the outer export function adds only fixed envelope fields.

## 12. Arbitrary-string/PII injection

The candidate production tripwire serialized the sanitized result into final export-shaped JSON and found 0/6 exact fixture injections in the resulting bytes. The six exact input values were loaded from `src-tauri/fixtures/diagnostics-pii-injection.json`; they are intentionally not reproduced in this report so the report itself remains safe for the repository scanner. The test asserted serialized output, not an intermediate raw DTO.

An additional audit-only harness passed the exact arbitrary strings `ProjectZephyr987`, `NurasSecretWorkspace`, and `private-customer-alpha` through the same production `sanitize_diagnostics` and `render_copy_text` functions. Result: `arbitrary_string_tripwire=PASS`; none survived JSON or copy output. The temporary harness was outside the repository and was deleted.

The candidate test does not construct a full `AppState` to call the outer `diagnostics_export_payload` function, but source inspection confirms that function calls the same sanitizer and adds no raw fields. The byte-level candidate tripwire plus the additional production-function harness therefore closes the allowlist proof without treating a raw intermediate DTO as safe.

## 13. Tripwire negative control

The four diagnostics tripwire tests all passed, including:

- safe structural fields preserved;
- production export bytes clean;
- copy/export semantic equivalence;
- intentionally unsafe generated payload rejected.

The unsafe payload was created in memory only and was not committed or left as an artifact. The safe payload passed; the negative control failed as expected, proving the tripwire can detect a regression rather than merely pass vacuously.

## 14. CI PII enforcement

`.github/workflows/ci.yml` runs `npm run check:pii` automatically in the portable job on push and pull request. The script performs the generic repository scan, then the package script runs the production `usage-ai-app` serialized-output tripwire tests. Thus a production export leak causes a unit-test failure even if repository grep is clean.

The generic scan has a narrow, explicit allowlist for the sanctioned injection fixture. It does not allow an arbitrary fixtures directory or broad text class. The exact fixture therefore does not create a false positive, while production output remains subject to the tripwire.

## 15. Multi-profile targeted regression

The full locked suite passed the existing profile isolation and source-routing coverage, including `profile_a_b_a_isolation_with_disable_and_delete`, `profile_a_fetch_cannot_observe_profile_b`, `default_and_custom_roots_never_share_rows`, `app_server_serves_default_scopes_only`, `claude_quota_ends_serve_default_scopes_only`, and the storage default/custom connection isolation tests.

The same run covered default profiles, custom/empty roots, Codex A/B routing, Claude A/B routing, OAuth default-only routing, disable/delete cancellation, and per-profile notification state. Migration 010 changes only the targeted legacy Claude trust field and did not change account or connection identity routing.

## 16. Locked reproducibility

- `cargo metadata --locked --no-deps --format-version 1` passed.
- `Cargo.lock` and `package-lock.json` stayed byte-unchanged after dependency installation and all locked gates.
- The exact detached worktree stayed clean.
- Intel CI explicitly runs a locked release binary build before Tauri bundling, invokes the bundle with `CARGO_NET_OFFLINE=true`, and checks the lockfiles afterward.
- The CI workflow does not publish the first arbitrary file returned by a broad DMG search; its `find` commands are verification-only.

The reproducibility/lock evidence passes. The separate CI format gate does not, as recorded in sections 5 and 28.

## 17. Intel artifact provenance

The durable RC3 DMG is present at:

```text
target/x86_64-apple-darwin/release/bundle/dmg/Usage.ai_1.3.0_x64.dmg
size: 6808274 bytes
mtime: 2026-09-12T14:15:09+0500
sha256: 7764792634f4717c09d528042388d0f89ba51bd7742cd9c6bf287aea7fe28111
```

An isolated exact-tag build from the detached `90ca0dd` tree produced a fresh `.app`; its standalone executable and bundled executable both hashed to:

```text
276104c612a5a9faa45e3b575307225521f6cf2d5abebc7575062736f44ff142
```

The durable DMG’s embedded executable has the same hash. The exact-tag fresh app therefore independently links the durable artifact to the audited source, even though the docs-only candidate commit timestamp is later than the artifact timestamp. The exact fresh `.app` and temporary build target were removed after verification.

The RC3 remediation report’s recorded candidate HEAD and DMG mtime are inaccurate, but the artifact hash and independent exact-tag rebuild provide stronger provenance than that report alone.

## 18. Binary/DMG validation

The durable embedded executable is a non-fat Mach-O `x86_64`; `lipo -info` reports architecture `x86_64`. Bundle metadata is:

```text
CFBundleIdentifier          com.nurasss.usageai
CFBundleShortVersionString  1.3.0
CFBundleVersion              1.3.0
LSMinimumSystemVersion      13.0
```

`hdiutil verify` returned `checksum ... is VALID`. A read-only mount contained `Usage.ai.app` and the expected Applications link; the embedded executable was x86_64 and carried the hash above. The image was cleanly unmounted.

There is a stale target-contamination signal: the standalone file at `target/x86_64-apple-darwin/release/usage-ai-app` has hash `14fb85e4e8859692b3a46f396a9d5549c27345ab9fceb10e57ebe69fd649d63f` and an older mtime, unlike the bundled release executable. This disproves the remediation report’s equality claim for that stale path. The CI/release path builds a fresh standalone binary and bundles it, and no publish step selects that stale file or uses a broad DMG glob; the durable release DMG itself is valid and linked to the exact fresh bundle hash.

## 19. Packaged runtime smoke

The freshly built exact-tag app was launched with `open -n`, remained alive at the 5-second and 30-second checks, and exited through the application quit AppleScript with return code 0. No immediate crash occurred. The durable packaged app received the same 30-second/graceful-quit smoke treatment earlier in the audit.

Native visual state was not claimed from this process-level smoke; tray/panel visual checks are recorded separately as environment-blocked.

## 20. Source-owned mutation proof

Before and after the live Codex/Claude probes, a metadata-only signature over the relevant Codex and Claude auth/config/session paths was identical:

```text
before: 704d52996006c32be404a7b833af09ce82c9cf760bab387254658e9b4f2b2413
after:  704d52996006c32be404a7b833af09ce82c9cf760bab387254658e9b4f2b2413
```

The packaged smoke had an independent unchanged metadata signature as well. No file contents or secrets were printed. The live Codex RPC uses `refreshToken:false`; Claude’s production driver removes ambient provider token variables from child processes. No official client credential, session, or configuration writeback was observed.

## 21. Codex live qualification

Final exact opt-in run:

```text
USAGE_AI_LIVE_CODEX=1
live_app_server_reports_verified_quota: PASS (17.26 s)
```

The acceptance asserts a real app-server handshake, non-empty quota, plan presence, observed identity, and safe rendered surface. Source inspection confirms the bounded read-only RPC sequence (initialize, account read with refresh disabled, rate-limits read) and no inference request. The final run passed; an earlier attempt observed transient `NotConfigured`, but the repeated exact acceptance passed once the local CLI was ready.

## 22. Claude live qualification

The installed Claude CLI was found and both opt-in live tests passed: auth status shape and bounded quota-end behavior. The safe-degradation branch completed quickly and did not hang or synthesize quota.

The host’s status output confirmed a logged-in OAuth-shaped state but did not provide a true subscription-availability qualification field. The production driver also scrubs ambient token variables, so a shell token or OAuth-shaped status cannot be promoted to a true-subscription positive result.

```text
CLAUDE_LIVE_SAFE_DEGRADE: PASS
CLAUDE_TRUE_SUBSCRIPTION_LIVE: BLOCKED_BY_ENVIRONMENT
```

No Claude quota happy path was marked PASS without a qualifying true-subscription signal.

## 23. Native notification

`BLOCKED_BY_ENVIRONMENT`: the available audit session did not provide a reliable native notification visual/event assertion or permission-safe notification fixture. The authoritative-only planner logic and unverified suppression tests pass, but no native toast was claimed.

## 24. Launch at login

`BLOCKED_BY_ENVIRONMENT`: a full enable → native registration → relaunch/reboot → disable restoration cycle was not available in the audit session. No setting was changed for this check.

## 25. Clean-install GUI

`BLOCKED_BY_ENVIRONMENT`: the valid DMG was verified and mounted read-only, but a quarantined drag-install plus visual tray/panel/settings smoke was not available. The exact-tag Tauri bundle run reached the Finder-prettification AppleScript and was stopped because the interactive Finder step did not complete in this environment; the audit-owned mount/process was cleaned up. Gatekeeper was not disabled.

## 26. Security regression sweep

| Check | Result |
|---|---|
| Unverified Claude becomes authoritative | NO |
| Legacy Claude `Partial` remains authoritative | NO |
| Arbitrary diagnostics text exported | NO |
| Exact injected PII exported | NO |
| Cross-profile credential fallback | NO |
| Credential leak/writeback | NO |
| Inference generated by monitoring | NO |
| Money `f64` regression | NO |
| Data-destructive migration | NO |

The first six security rows are all negative, so no P0 reject condition was observed. The independent release-gate format failure is a separate P1 process/release issue, not a security escalation.

## 27. Master Spec release matrix

| Spec area | Status | Evidence |
|---|---|---|
| Codex quota | PASS | Exact opt-in verified-quota acceptance passed. |
| Claude quota | BLOCKED_BY_ENVIRONMENT | Safe degradation passed; true subscription positive unavailable. |
| Codex history | PASS | Locked provider/runtime/history tests and isolation coverage. |
| Claude history | PASS | Locked reducer, profile, and provenance coverage. |
| Multi-profile | PASS | A/B/A and default/custom isolation tests passed. |
| Multi-profile quota | PASS | Provider scope routing and default-only tests passed. |
| Credential ownership | PASS | Read-only metadata smoke and scrubbed child environment. |
| Zero inference | PASS | Source review and live read-only handshakes. |
| Trust/fidelity | PASS | Coverage/source/freshness guards and side-effect tests. |
| Legacy upgrade trust | PASS | RC1-equivalent migration, reopen, physical state, and guards. |
| Diagnostics privacy | PASS | v3 allowlist, 0/6 fixture leaks, arbitrary-string harness. |
| Decimal money | PASS | Locked storage/provider aggregation and Decimal regression suite. |
| SWR/LKG | PASS | Stale LKG/current-error and persistence tests. |
| Adaptive scheduler | PASS | Coalescing, cooldown, offline, cancellation, and cadence tests. |
| Tray | PASS | Authoritative-only selection and mixed 61%/20% regression. |
| Notifications | PASS | Authoritative-only planner, profile mute, and mixed-provider logic. |
| Intel artifact | PASS | x86_64 app, metadata, valid durable DMG, matching exact rebuild. |
| Packaged smoke | PASS | Exact app alive at 30 seconds and graceful quit. |
| Locked reproducibility | PASS | Locked metadata/gates and unchanged lockfiles. |
| Native notification | BLOCKED_BY_ENVIRONMENT | Native visual/event check unavailable. |
| Launch-at-login | BLOCKED_BY_ENVIRONMENT | Native relaunch cycle unavailable. |
| Clean-install GUI | BLOCKED_BY_ENVIRONMENT | Quarantined install and visual smoke unavailable. |
| CI/gates | FAIL | Actual CI `cargo fmt --all -- --check` fails. |
| Independent audit | FAIL | P1 CI release-gate defect remains. |

## 28. Remaining blockers

P1 release blocker:

```text
cargo fmt --all -- --check  -> exit 1
```

The exact RC3 tree has rustfmt diffs in:

```text
crates/usage-host/src/process.rs
crates/usage-host/src/pty.rs
crates/usage-providers/src/claude.rs
crates/usage-providers/src/codex_appserver.rs
crates/usage-storage/src/lib.rs
src-tauri/src/commands/diagnostics.rs
src-tauri/src/dto.rs
src-tauri/src/refresh_flow.rs
```

This is an automatic CI failure, not a cosmetic local-only observation, and it must be resolved before a final release tag under the tracked Definition of Done.

Environmental qualifications still outstanding are Claude true-subscription positive, native notification, launch-at-login, and clean-install GUI. They do not justify a new code remediation round by themselves, but they prevent an unconditional approval even after the format gate is closed.

Non-blocking evidence cautions are the stale HEAD/mtime/equality statements in the RC3 remediation report and the unscoped provider/source predicate in the storage JSON walker. Independent exact-tree, physical-state, and artifact checks above prevent either caution from being mistaken for a passed claim.

## 29. Final verdict

The exact commit is technically strong on the two audited P0 regressions and has a valid Intel artifact/runtime path, but it is not release-ready because the repository’s own automatic format gate fails. Since that is a P1 release-gate defect, the final verdict is not an environmental-only hold.

No final `v1.3.0` tag may be created from `90ca0dd` at this time.

## 30. Exact next action

Do not tag or push. The next action is limited to the release owner resolving the reported `cargo fmt --all -- --check` failure in the eight exact paths listed in section 28, then rerunning the actual CI workflow and this read-only audit against the resulting exact commit. No production behavior change is prescribed by this audit.

After that gate is green, perform only the minimal environment checklist: qualify a real Claude true-subscription happy path, native notification, launch-at-login relaunch, and quarantined DMG clean-install GUI smoke. Until both the P1 gate and those required qualifications are closed, do not create `v1.3.0`.

FINAL TAG v1.3.0: HOLD — REMEDIATION REQUIRED
