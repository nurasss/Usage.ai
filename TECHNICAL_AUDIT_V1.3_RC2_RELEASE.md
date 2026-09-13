# Usage.ai v1.3.0-rc2 — final independent read-only release audit

Audit date: 2026-09-11/12 (Asia/Almaty)  
Candidate tag: `v1.3.0-rc2`  
Tag object: `00d09c81058277cb3cff36d0660dace3a9c6e9c4`  
Candidate commit: `1a691130963e4aa78840e514ead5ffed35280cb0`

## 1. Executive verdict

**RELEASE REJECTED.** Exact RC2 commit must not be tagged `v1.3.0`.

Two independently reproduced P0 findings remain:

1. **Legacy RC1 Claude trust is not migrated or invalidated.** A persisted Claude synthetic snapshot with `Coverage::Partial` is deserialized by RC2 unchanged and remains `is_authoritative() == true`. LKG rewrites freshness only; the application cache returns a fresh legacy DTO unchanged. Current LKG freshness checks happen to suppress quota tray/notification side effects, but the unverified value remains authoritative in the current domain/UI and can enter the ordinary provider surface. This violates Master Spec §2.6 and the explicit RC1→RC2 audit gate.
2. **Diagnostics export retains injected identity/free-form strings.** The production sanitizer leaves `MyPrivateProject` and `raw account identity note` byte-for-byte in the sanitized payload. The requested six-value CI injection also exits 0. This is a P0 privacy/export failure under the audit severity rules.

Additional release blockers/gaps: Claude has no production-enabled authoritative quota source; the release-build CI path is not lock-enforced; no actual RC2 Intel artifact is present; Codex live-positive failed on this host; mandatory native/package qualification could not be performed on RC2.

## 2. RC2 provenance

`git rev-parse v1.3.0-rc2` resolves the annotated tag object; peeling it resolves exactly to `1a691130963e4aa78840e514ead5ffed35280cb0`. `git merge-base --is-ancestor v1.3.0-rc2^{} main` returned 0.

Remote evidence:

```text
refs/heads/main       1a691130963e4aa78840e514ead5ffed35280cb0
refs/tags/v1.3.0-rc2 00d09c81058277cb3cff36d0660dace3a9c6e9c4
refs/tags/v1.3.0     absent
```

The RC2 commit is `main`, `origin/main`, and the peeled RC2 tag target. The seven post-RC1 implementation commits plus the final report commit are visible in history. The RC2 tree contains no README diff from RC1 and none of the prohibited marketing claims. The dirty main checkout was not used as evidence.

## 3. Master Spec provenance

`Usage.ai_MASTER_SPEC_v1.3.md` and `Cargo.lock` are tracked in RC2. SHA-256:

```text
38c5320684a694f24b5546f700453d23093a8340f5a2a1d4281480cb776d7d87
```

The tracked file is byte-identical to `/Users/nuras/Downloads/Usage.ai_MASTER_SPEC_v1.3(1).md`. It was newly added after RC1, rather than edited relative to a tracked RC1 copy; comparison with the supplied original establishes that normative text was not weakened for RC2.

Relevant normative clauses are §2.6 (unverified data never authoritative by omission), §10.3.4 (Claude acceptance), §21.1 (safe migration), §22.3 (no raw secret in diagnostics), V13-03, V13-07, the release matrix, and Definition of Done.

## 4. Clean audit worktree

All source inspection and gates ran from detached worktree `/private/tmp/usage-ai-rc2-audit-worktree` at exact `1a691130...`. Its source status was clean before and after gates; `Cargo.lock` had no diff. `node_modules`, `dist`, and the isolated Cargo target were ignored/transient.

The user's main checkout was preserved unchanged:

```text
 M README.md
?? TECHNICAL_AUDIT_V1.3_RC1_RELEASE.md
```

This RC2 report is the only repository file created by this audit.

## 5. Mandatory locked gates

| Gate | Independent result |
|---|---|
| `npm ci` | PASS; 137 packages installed; npm reported 2 moderate dependency advisories |
| `npm run check` | PASS; 0 errors, 1 existing Svelte a11y warning at `src/App.svelte:327` |
| `npm test -- --run` | PASS; 5 files, 47 tests |
| `npm run build` | PASS; Vite production build, same warning |
| `cargo check --workspace --locked` | PASS |
| `cargo test --workspace --locked` | PASS; 293 passed, 4 ignored, 0 failed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS; 0 warnings/errors |
| `npm run check:pii` on RC2 tree | PASS |

The warning and npm advisories are not the basis of the verdict.

## 6. Test-count reconciliation

Actual Rust breakdown:

| Crate/target | Passed | Ignored |
|---|---:|---:|
| `usage-ai-app` library | 19 | 0 |
| `usage-core` | 19 | 0 |
| `usage-host` | 50 | 0 |
| `usage-providers` | 96 | 4 |
| `usage-runtime` | 77 | 0 |
| `usage-storage` | 32 | 0 |
| Total | **293** | **4** |

RC1 has 279 Rust test attributes and RC2 has 297. The diff contains exactly 18 additions (8 `#[test]`, 10 `#[tokio::test]`) and no test-attribute deletions. Thus 275→293 passed is a real +18 regression-test increase, not a changed counting command.

Ignored tests:

| Test | Why ignored | Run in audit? | Result | Covered elsewhere? |
|---|---|---|---|---|
| `codex::tests::real_codex_corpus_probe_is_opt_in_and_never_checked_in` | User-supplied real/redacted corpus required | No; no explicit corpus supplied | NOT_APPLICABLE | Redacted corpus/unit evidence only |
| `codex_appserver::live_acceptance::live_app_server_reports_verified_quota` | Local login/healthy CLI required | Yes | **FAIL**: availability `NotConfigured`, expected `Ready` | Fixtures do not replace live positive |
| `claude::live_acceptance::live_auth_status_shape_parses` | Local Claude install | Yes | PASS, 3.73 s | Direct real binary shape exercised |
| `claude::live_acceptance::live_quota_end_serves_or_degrades_cleanly` | Local Claude install/session | Yes | SAFE DEGRADATION PASS, 2.96 s | Does not establish live-positive quota semantics |

Frontend breakdown was 8 + 29 + 2 + 3 + 5 = 47. UI trust/SWR behavior is exercised there; diagnostics production serialization is covered in Rust, not frontend tests.

## 7. P0 Claude trust remediation

The new-snapshot fix itself is correct:

- PTY and OAuth payloads map to `Coverage::UnverifiedSemantics` (`crates/usage-providers/src/claude.rs`, PTY payload around lines 1217–1250; OAuth around 1439–1490).
- `Coverage::UnverifiedSemantics` is non-authoritative because `is_authoritative()` admits only `Complete | Partial` (`crates/usage-core/src/models.rs:176–192`).
- tray selection requires fresh `allows_tray_metric()` (`src-tauri/src/refresh_flow.rs:394–433`).
- quota threshold, pace, and reset planning returns before quota processing when coverage is ineligible (`crates/usage-runtime/src/notifications.rs:120–127`).
- actual tests prove Fresh/Connected Claude 20% produces no tray/quota notifications, and verified Codex 61% beats unverified Claude 20%.
- UI explicitly labels `UnverifiedSemantics`; aggregation stores it in the separate unverified overview.

Legitimate `Coverage::Partial` was not globally downgraded: official OpenAI costs still emit and test `Partial` for incomplete/paginated results (`crates/usage-providers/src/openai_api.rs`).

However, the fix applies only when current RC2 strategies create a new payload. It does not repair already-persisted RC1 data; see the next section.

## 8. Legacy RC1/LKG trust migration

**P0 FAIL.** No migration 010, trust schema version, source fingerprint check, reclassification, or invalidation exists for RC1 Claude snapshots.

Evidence:

- `Storage::last_known_good` returns raw `payload_json` without source/trust metadata checks (`crates/usage-storage/src/lib.rs:781–801`).
- `stale_view_from_lkg` deserializes the raw `Snapshot` and changes only `freshness` (`crates/usage-runtime/src/snapshot.rs:13–21`).
- the persisted `app_snapshot` cache has no version column (`crates/usage-storage/migrations/001_initial.sql:31–33`) and `get_snapshot` returns a deserialized `AppSnapshot` unchanged (`src-tauri/src/commands/snapshot.rs:15–23`).

An external read-only probe constructed an RC1-equivalent Claude snapshot (`synthetic-v1`, `Partial`, 20% remaining) and invoked the actual RC2 `stale_view_from_lkg`:

```text
coverage=Partial
authoritative=true
tray_eligible_by_coverage=true
notification_eligible_by_coverage=true
freshness=Stale(0)
remaining=20
```

Precise side-effect answer: the LKG outcome is marked stale, so today's tray and quota-notification call sites suppress it on freshness; the separate app cache is UI-only and does not itself invoke delivery. Therefore the probe does not demonstrate an immediate notification after restart. It does demonstrate the forbidden persisted trust promotion: legacy unverified Claude remains an authoritative `Partial` in RC2 and a fresh cached DTO is served unchanged to the ordinary UI/overview surface. The audit request explicitly defines this absence of migration/invalidation as a P0 blocker, and Master Spec §2.6 also forbids it.

No RC1→RC2 legacy-Claude trust regression test exists. Existing LKG tests intentionally assert payload coverage survives and use non-Claude data.

## 9. Claude PTY/environment remediation

PASS for the scoped transport mechanics:

- child-only scrub list is closed to `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` (`crates/usage-host/src/process.rs:62–95`);
- `CLAUDE_CONFIG_DIR` remains a separately allowlisted scoped path and is not scrubbed;
- one-shot process uses `Command::env_remove`; forkpty uses `unsetenv` between fork and direct `execvp`;
- the parent process environment is not globally mutated by production code;
- the real audit shell contained a six-character `ANTHROPIC_AUTH_TOKEN`, but the scrubbed direct gate reported `CLAUDE_LOGGED_IN=false`;
- auth parsing consumes only the `loggedIn` boolean, with bounded stdout/stderr; raw output is not logged/stored (`claude.rs:457–491`).

The staged drive is bounded by one deadline, waits for first paint, aborts before typing on trust markers, writes exactly `/usage`, continuously rechecks trust/login evidence, requests `/exit`, then kills/reaps its owned child (`claude.rs:518–664`). No inference prompt or automatic trust acceptance was found. Unit and real-fake-executable tests cover timeout, cancellation, trust abort, output cap, env isolation, and exact command sequence.

## 10. Claude live qualification

Installed executable: `/Users/nuras/.npm-global/bin/claude`.

```text
live_auth_status_shape_parses                    PASS (3.73 s)
live_quota_end_serves_or_degrades_cleanly        SAFE DEGRADATION PASS (2.96 s)
CLAUDE_LIVE_POSITIVE                             BLOCKED_BY_ENVIRONMENT
```

With both ambient Anthropic credential variables removed, the current host reports `loggedIn=false`. No real subscription `/usage` shape was available. Unknown/new shapes fail closed in parser tests, but this is not live-positive evidence.

Independently of environment, RC2's production PTY result is always `UnverifiedSemantics`, while OAuth production wiring has `oauth_url: None` and is `NotConfigured` (`claude.rs:1260–1264`). The Master Spec requires a production-ready Claude quota source. Thus **Claude quota = FAIL/P1**, not PASS: safe degradation and non-authoritative display are safe behavior, but not delivery of the required authoritative product capability.

## 11. Codex live qualification

Installed executable candidate: `/usr/local/bin/codex`.

The opt-in live App Server test independently failed after 12.36 s:

```text
availability: NotConfigured
expected:     Ready
CODEX_LIVE_POSITIVE: FAIL on this host
```

The test's code performs health-gated candidate selection, bounded `initialize`, `account/read` with `refreshToken=false`, and `account/rateLimits/read`. Unit/fake-executable coverage for broken-shim skipping and A/B `CODEX_HOME` isolation passes. Because the current failure is at executable health availability and no defective production branch was isolated, it is recorded as an environmental/live qualification failure rather than an additional P0.

Top-level `~/.codex/auth.json` and `~/.claude.json` size, mtime, and inode were identical before/after live probes. A recursive `.codex` metadata hash changed, but the active Codex desktop audit task concurrently writes its own rollout/state/log databases; attribution to the read-only App Server probe is not possible. No Claude config files changed in the recent-file check. No credential contents were read.

## 12. Multi-profile quota isolation

PASS for routing/isolation mechanics, with the Claude production-readiness limitation above:

- `build_strategy` passes `custom_root` into Codex App Server and Claude PTY (`crates/usage-runtime/src/refresh.rs:917–957`).
- Codex child receives only `CODEX_HOME=<scope root>`; Claude child receives only `CLAUDE_CONFIG_DIR=<scope root>`.
- real fake executables prove A/B results differ by scoped root and cannot see each other's records.
- empty custom roots yield authentication/unavailable behavior rather than the default account.
- Claude OAuth is explicitly disabled for every custom root and remains default-only; missing custom auth cannot silently use default OAuth.
- local history roots, LKG scope keys, attempts, cooldowns, and notifications are account-scoped.

The required `multi-profile subscription quota` isolation is structurally present. Claude quota itself remains non-authoritative/unqualified, so this PASS must not be read as Claude quota production readiness.

## 13. Diagnostics PII/export

**P0 FAIL.** The call graph does use one sanitizer:

```text
Diagnostics DTO -> sanitize_diagnostics
  -> render_sanitized_copy
  -> diagnostics_export_payload -> JSON file export
```

No raw DTO serialization bypass was found (`src-tauri/src/commands/export.rs:64–79`; `diagnostics.rs:416–502`). Safe operational fields remain useful: provider/product, strategy, coverage, freshness, status class, attempt status/count/latency, timestamps, schema/fingerprint, and import counts.

The sanitizer nevertheless runs user/provider-controlled alias and warnings through regex-style `redact_free_text`, rather than dropping/classifying arbitrary identity prose. Its own test documents that bare codenames survive (`diagnostics.rs:792–815`). An external program called the actual public production sanitizer and serialized its result. Exact presence results:

```text
alice@example.com:false
/Users/alice/private/project:false
Bearer SUPERSECRET:false
sk-test-secret:false
MyPrivateProject:true
raw account identity note:true
```

That violates the explicit audit criterion that none of the six source strings survive actual export bytes. The copy/export equivalence test passes because both surfaces share the same flawed sanitized bundle; equivalence does not make the retained strings safe.

The repository `npm run check:pii` passes and CI invokes it. A temporary Git repository containing exactly all six requested injection lines also produced:

```text
PII tripwire: clean.
EXIT_CODE=0
```

The tripwire allowlists `@example.com`, `/Users/alice/`, and `sk-test`, and has no rule for arbitrary account/project identity prose. A separate non-allowlisted email/key probe correctly failed, showing the command runs but the requested policy coverage is incomplete.

## 14. CI/reproducibility

Local reproducibility checks PASS: `Cargo.lock` is tracked; all direct Cargo gates used `--locked`; `cargo metadata --locked --no-deps` passed before/after; the clean worktree and lockfile remained unchanged.

CI portable job uses `--locked` for check, test, and clippy. Intel job uses it for target check/test. **P1 FAIL:** the actual release command is `npm run tauri build -- --target x86_64-apple-darwin` without `--locked` (`.github/workflows/ci.yml:37–44`). `CARGO_NET_OFFLINE=true` prevents network access but does not prohibit Cargo from rewriting a lockfile using cached/index data; it is not lock enforcement.

The Intel job verifies architecture and lists bundles but does not publish/upload artifacts. Its `find ... | head -1` executable selection is broad, though a fresh GitHub runner limits stale-target risk.

## 15. Adaptive runtime regression

PASS in targeted code/tests: per-scope coalescing is registered before semaphore admission; cancellation covers queued/in-flight/disable/delete/shutdown; cooldown survives restart and is not bypassed by manual refresh; offline skips official HTTP; cadence partitions due/idle scopes; LKG serves stale data plus current error. No faster-degrade busy loop was found.

The legacy trust defect is a payload migration problem, not a scheduler regression.

## 16. Tray/overview trust

Current RC2 `UnverifiedSemantics` path: PASS. Fresh unverified Claude is excluded, pinned-ineligible falls back, verified 61% beats unverified 20%, and only-unverified yields no numeric tray metric. Aggregate data is separated into `unverified_overview`; UI shows an explicit warning.

Upgrade trust/overview path: **FAIL/P0** because a legacy cached `Partial` is still authoritative and can render without the unverified warning. The tray call site itself remains protected by LKG freshness, but the stored trust value is not corrected.

## 17. Notifications/mute

Current RC2 behavior: PASS in unit/integration evidence. Quota threshold, pace, and reset notices require fresh authoritative coverage; mixed verified/unverified behavior, account/profile title, persisted per-account mute, threshold deduplication, and account isolation pass.

Legacy LKG is stale and therefore does not currently emit quota notices. There is no dedicated pre-RC2 `Partial` migration regression test; the underlying authoritative flag remains unsafe as described in §8.

## 18. Intel artifact provenance

**P1 FAIL.** The remediation report claims a clean `fc007c3` artifact with DMG hash `00958b...` and binary hash `ac545f...`, then states that its worktree/target were deleted. No file with those claimed artifacts is present in the workspace, Downloads, or audit temp paths.

The only available named Intel artifact is stale:

```text
DMG:    /Users/nuras/Desktop/usage.ai/target/x86_64-apple-darwin/release/bundle/dmg/Usage.ai_1.3.0_x64.dmg
size:   6,811,933 bytes
mtime:  2026-09-11T15:53:39+0500
sha256: d1e0c8d6639403888e3fd612b789bfdcbdbe77603bb8eab0aa9e437b2cea89ce

binary: /Users/nuras/Desktop/usage.ai/target/x86_64-apple-darwin/release/usage-ai-app
size:   19,746,440 bytes
mtime:  2026-09-11T15:53:39+0500
sha256: 14fb85e4e8859692b3a46f396a9d5549c27345ab9fceb10e57ebe69fd649d63f
```

Its timestamp predates the RC1 tag/commit and its binary lacks RC2-unique trust-abort strings. It cannot be treated as RC2 merely because the bundle version says 1.3.0.

## 19. DMG/bundle validation

The stale DMG mounts read-only and unmounts cleanly. It contains the expected app; embedded and standalone binary hashes match. Static metadata:

```text
architecture                 x86_64
CFBundleIdentifier           com.nurasss.usageai
CFBundleShortVersionString   1.3.0
CFBundleVersion              1.3.0
LSMinimumSystemVersion       13.0
```

These values match tracked Tauri config, but this validates only the stale pre-RC1 bundle, not RC2. Consequently RC2 DMG/bundle validation = FAIL (artifact absent).

## 20. Packaged runtime/read-only smoke

**BLOCKED_BY_ENVIRONMENT / missing candidate artifact.** The available bundle is proven stale, so launching it would not test RC2-specific behavior. No independent RC2 30-second alive/tray/graceful-quit result can be credited. Packaged source-mutation comparison likewise cannot be performed for RC2.

## 21. Native notification qualification

`NATIVE_NOTIFICATION_SMOKE = BLOCKED_BY_ENVIRONMENT`.

The automation surface exposes no native apps, and no RC2 package exists. Unit planner/delivery wiring is not counted as a native notification appearance + mute smoke.

## 22. Launch-at-login qualification

`LAUNCH_AT_LOGIN_NATIVE_SMOKE = BLOCKED_BY_ENVIRONMENT`.

Autostart plugin wiring exists, but native registration/enable/disable/restore could not be exercised without a valid RC2 packaged app and native GUI control.

## 23. Clean-install GUI qualification

`CLEAN_INSTALL_GUI_SMOKE = BLOCKED_BY_ENVIRONMENT`.

No valid RC2 DMG exists and native app control is unavailable. The stale DMG was not installed or used as a substitute. Gatekeeper/security settings were not changed.

## 24. README/stale-artifact hygiene

RC2's committed README has no accidental dirty diff and none of the enumerated false claims. The user's README modification remains unmodified and excluded.

The old `rw.53342.Usage.ai_0.1.0_x64.dmg` remains under the local bundle tree, as required. No publish/release script that uploads all `target/**/*` was found. CI only lists matches and uses a fresh runner, so the stale file is advisory locally, not a separate publication blocker. Exact artifact provenance remains a blocker for the reason in §18.

Disk at audit start/end was approximately 7.8 GiB/1.8 GiB free while the isolated Rust target existed; the user's `target` is 7.2 GiB. No user artifact was deleted. The audit target is temporary and is removed after evidence collection.

## 25. Security regression sweep

| Check | Result |
|---|---|
| Arbitrary shell in production | PASS; structured direct exec, shell references are tests |
| Browser-cookie access | PASS; none found |
| Source credential writeback | PASS by code; top-level auth files unchanged in live probes |
| Secrets/PII in diagnostics | **FAIL/P0**; identity prose/codename survives export |
| Secrets in SQLite | PASS in targeted code/tests; no permanent Claude/Codex secret copy path |
| Cross-profile default credential fallback | PASS in routing/tests |
| Remote redirect with secrets | PASS; HTTP redirects disabled and hosts allowlisted |
| Unverified current side effects | PASS for new RC2 payloads |
| Legacy unverified authoritative state | **FAIL/P0** |
| Monitoring inference | PASS; Claude writes only `/usage`/`/exit`; Codex uses usage RPCs |
| Money precision | PASS; exact `Decimal` parsing/storage/aggregation tests |

## 26. Release matrix vs Master Spec v1.3

| Requirement | Status | Evidence/qualification |
|---|---|---|
| Codex quota | BLOCKED_BY_ENVIRONMENT | Live App Server `NotConfigured`; fixtures pass |
| Claude quota | **FAIL** | No production authoritative source; live positive unavailable |
| Codex history | PASS | Evidence/parser/import/account-isolation suite |
| Claude history | PASS | Evidence/parser/import/cache-bucket/isolation suite |
| Multi-profile | PASS | Scoped roots and identity/account isolation |
| Multi-profile quota | PASS | Account-scoped child env and A/B executable tests; capability quality separate |
| Credential ownership | PASS | Child-only scrub/read-only paths; auth files unchanged |
| No inference | PASS | Usage/status-only command/RPC trace |
| Trust/fidelity | **FAIL** | Legacy `Partial` remains authoritative |
| Diagnostics PII | **FAIL** | Production bytes retain two injected identity strings |
| Decimal money | PASS | Arbitrary-precision Decimal end-to-end tests |
| SWR/LKG | **FAIL** | Base SWR works; legacy trust migration does not |
| Adaptive scheduler | PASS | Coalescing/cancel/cooldown/offline/cadence tests |
| Tray | PASS | Current and legacy-LKG side effects are freshness/coverage gated; persisted trust still fails separately |
| Notifications | PASS | Current and stale call sites suppress ineligible quota alerts |
| Native notifications | BLOCKED_BY_ENVIRONMENT | No RC2 package/native surface |
| Launch at login | BLOCKED_BY_ENVIRONMENT | Wiring only, no native smoke |
| Intel artifact | **FAIL** | RC2 artifact absent; only stale pre-RC1 bundle exists |
| Packaged smoke | BLOCKED_BY_ENVIRONMENT | No valid RC2 package |
| Clean install | BLOCKED_BY_ENVIRONMENT | No valid RC2 DMG/native GUI control |
| Migration | **FAIL** | RC1 Claude trust state not migrated/versioned/invalidated |
| Locked reproducibility | **FAIL** | Local gates locked; Tauri release build CI is offline but not locked |
| Clean release tree | PASS | Exact detached RC2 tree stayed clean |
| Mandatory gates | PASS | 47 frontend, 293 Rust + 4 ignored, clippy 0 |
| Independent audit | **FAIL** | Audit completed but found P0/P1 blockers |

## 27. Remaining blockers

| Severity | Blocker | Exact source/function/test evidence |
|---|---|---|
| P0 | Legacy Claude `Partial` remains authoritative | `usage-runtime/src/snapshot.rs::stale_view_from_lkg`; `usage-storage/src/lib.rs::last_known_good`; `src-tauri/src/commands/snapshot.rs::get_snapshot`; external actual-library probe; no migration/test |
| P0 | Runtime diagnostics export retains identity/free-form values | `src-tauri/src/commands/diagnostics.rs::sanitize_diagnostics`; `file_export_sanitizes_hostile_fields` explicitly omits codename assertion; actual sanitizer probe retains `MyPrivateProject` and identity note |
| P1 | Required Claude quota is not production-ready | `ClaudePtyUsageStrategy::fetch` always `UnverifiedSemantics`; production OAuth URL is `None`; live-positive unavailable |
| P1 | RC2 artifact provenance invalid/absent | Claimed hashes absent; available bundle predates RC1 and has different hashes |
| P1 | Release build not lock-enforced in CI | `.github/workflows/ci.yml` Intel Tauri build uses only `CARGO_NET_OFFLINE=true` |
| Qualification | Codex live positive | Actual ignored test failed `NotConfigured` on this host |
| Qualification | Native notification/login/clean-install | No valid RC2 package and no native automation surface |

## 28. Final verdict

**RELEASE REJECTED** because at least one P0 is sufficient, and two are independently reproducible. Passing gates and the correct new-snapshot trust mapping do not override unsafe existing-user trust state or PII-bearing diagnostic export.

It is not safe to tag `1a691130963e4aa78840e514ead5ffed35280cb0` as final `v1.3.0`.

## 29. Exact next action

Do not create or push `v1.3.0`. Prepare a new release candidate only after all of the following are evidenced at exact source and artifact commit:

1. add a real RC1→new-version migration/invalidation test and mechanism for Claude persisted snapshot/app-cache trust;
2. make production diagnostics bytes remove/classify all six required injected strings and make the exact composite PII CI injection fail;
3. provide a production-authoritative, live-positive Claude quota path or revise the product version/spec through a separate normative decision (not by weakening this audit);
4. enforce the lockfile in the actual Tauri release build and retain a traceable exact-commit Intel `.app`/DMG;
5. rerun Codex/Claude live positives and the mandatory packaged/native qualification on that artifact.

Можно ли exact commit, на который указывает `v1.3.0-rc2`, прямо сейчас пометить финальным `v1.3.0` без нарушения Master Spec v1.3? **NO — P0 trust/privacy integrity blockers.**
