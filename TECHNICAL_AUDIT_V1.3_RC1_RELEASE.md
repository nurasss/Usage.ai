# Usage.ai v1.3.0-rc1 — final independent release audit

Audit date: 2026-09-11 (Asia/Almaty)  
Audit mode: read-only production/release audit; no production code, tests, fixtures, specification, README, commits, pushes, or tags were changed.  
Normative source: the supplied `Usage.ai_MASTER_SPEC_v1.3(1).md`, corresponding to the requested `Usage.ai_MASTER_SPEC_v1.3.md`. The normative file itself is not present in the audited Git tree.

## 1. Executive verdict

**RELEASE REJECTED**

It is **not safe** to create final tag `v1.3.0` on the current `v1.3.0-rc1` without violating the Master Spec v1.3 Definition of Done.

The decisive P0 defect is an authoritative-trust escalation in Claude quota:

1. The PTY parser explicitly documents its input shape as synthetic and not reconciled against a logged-in client (`crates/usage-providers/src/claude.rs:740-745`).
2. The successful PTY result explicitly adds `unverified_window_set`, but returns `Coverage::Partial` (`claude.rs:1090-1122`). The dormant OAuth implementation similarly adds `unverified_oauth_shape` and returns `Coverage::Partial` (`claude.rs:1308-1362`).
3. Central policy treats `Partial` as authoritative and consequently eligible for tray and quota notifications (`crates/usage-core/src/models.rs:176-192`).
4. The UI adapter treats the unverified PTY result as the Claude primary and does not flag it as quota fallback (`src-tauri/src/refresh_flow.rs:300-311`).

This is the Master Spec §28 P0 blocker “unverified data driving tray/notifications”, and may expose an authoritative numeric quota derived from an unqualified parser shape.

Further release-blocking gaps:

- **P1 / DoD:** a real Claude subscription login exists on the audit host, but the opt-in production-equivalent positive quota test failed with `Network` after its bounded timeout. `CLAUDE_LIVE_POSITIVE = FAIL`, not `BLOCKED_BY_ENVIRONMENT`.
- **P1 / multi-profile scope:** both Codex and Claude subscription quota ends are deliberately disabled for every custom profile root; custom profiles get local JSONL only (`crates/usage-runtime/src/refresh.rs:923-952`). This preserves isolation, but does not meet per-profile subscription-quota scope.
- **P1 privacy:** safe-copy redacts an account alias and warnings, but JSON diagnostics export serializes the unredacted DTO. User-controlled alias and identity-note text can therefore enter an exported diagnostics file (`src-tauri/src/commands/diagnostics.rs:235-240, 355-377`; `src-tauri/src/commands/export.rs:64-79`).
- **P1 reproducibility:** committed `Cargo.lock` records the six workspace packages at `1.0.0`; a clean RC snapshot fails `cargo check --workspace --locked` because the lockfile needs an update.

## 2. Repository state

Observed repository state:

```text
branch: main
HEAD: bd0ca57fca2b84938d4d79099d34763930b0645c
upstream: origin/main
ahead/behind: 0/0
staged changes: none
unstaged changes:
 M Cargo.lock
 M README.md
 M crates/usage-storage/src/lib.rs
```

`git diff --cached` was empty. The storage diff is formatting-only. The `Cargo.lock` diff changes the workspace package versions from `1.0.0` to `1.3.0` and is required for a locked clean build, but is not in HEAD/tag. The README diff is not in HEAD/tag and contains unsupported v1.3 claims; see §19.

The release tree is not clean. These pre-existing modifications were not changed by the audit. Only this report was added.

## 3. Tag/commit provenance

`v1.3.0-rc1` is an annotated tag:

```text
tag object: c7f2c829481173c6f33d659099288fb15227a94d
peeled commit: bd0ca57fca2b84938d4d79099d34763930b0645c
HEAD/main/origin-main: bd0ca57fca2b84938d4d79099d34763930b0645c
```

The requested milestone commits all exist, are ancestors of `main`, and are linearly ordered:

| Milestone | Expected | Verified revision/status |
|---|---:|---|
| V13-00 P0 seal | tag `v1.2-p0` | PASS; annotated tag peels to `50c2e7b` |
| V13-01 multi-profile | `1919644` | PASS |
| V13-02 Codex quota | `f9c02dc` | PASS |
| V13-03 Claude quota | `de48e18` | PASS |
| V13-04 fidelity/diagnostics | `06c1b8a` | PASS |
| V13-05 scheduler | `cb2b290` | PASS |
| V13-06 tray/notifications | `cc1fa84` | PASS |
| V13-07 release | `bd0ca57` | PASS, but expected map omits intermediate `98f0cb0` |

Actual tail is `cc1fa84 -> 98f0cb0 -> bd0ca57`. The extra commit changes versions and adds a v8-to-v9 migration test; final `bd0ca57` adds an offline test.

## 4. Mandatory gates

The six requested gates were run independently against a `git archive` of peeled RC commit `bd0ca57` in a temporary directory outside the repository. Build output used the existing external Cargo target cache to avoid exhausting the disk.

| Gate | Result | Observed evidence |
|---|---|---|
| `npm run check` | PASS with warning | 0 errors; 1 Svelte accessibility warning (`App.svelte:327`, label not associated with a control); version script reported 1.3.0 |
| `npm test -- --run` | PASS | 5 files, **47 passed**, 0 failed |
| `npm run build` | PASS with warning | Vite production build succeeded; same Svelte accessibility warning |
| `cargo check --workspace` | PASS | All workspace crates checked as 1.3.0; command updated the temporary stale lockfile |
| `cargo test --workspace` | PASS | **275 passed**, 0 failed, **4 ignored** |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS | **0 Clippy warnings** |

The reference claim “277 Rust passed” is inaccurate for this commit. The commit contains 279 Rust test attributes: 275 passed plus 4 ignored. V13 history shows monotonically increasing test attributes (277 at `cc1fa84`, 278 at `98f0cb0`, 279 at `bd0ca57`), so there is no evidence that tests silently disappeared.

Additional reproducibility gate:

```text
cargo check --workspace --locked
FAIL: Cargo.lock needs to be updated
```

The unlocked mandatory gates are green, but the committed revision is not cleanly reproducible under normal locked dependency discipline.

## 5. Version consistency

| Surface | Value | Status |
|---|---:|---|
| Root `Cargo.toml` workspace package | 1.3.0 | PASS |
| `package.json` | 1.3.0 | PASS |
| `src-tauri/tauri.conf.json` | 1.3.0 | PASS |
| Standalone `.app` Info.plist | 1.3.0 / build 1.3.0 | PASS |
| DMG embedded `.app` Info.plist | 1.3.0 / build 1.3.0 | PASS |
| Committed `Cargo.lock` workspace entries | 1.0.0 | FAIL |

The version script reports 1.3.0 but does not catch the committed lockfile mismatch.

## 6. Milestone verification

- V13-00 ancestry/tag: PASS.
- V13-01 isolation foundation: logic/tests PASS; per-profile quota coverage is incomplete (§7).
- V13-02 Codex design: PASS for bounded, shell-free, health-gated architecture; current live qualification failed (§8).
- V13-03 Claude design: FAIL for production readiness and trust classification (§9).
- V13-04 fidelity: FAIL because synthetic Claude PTY is presented as primary authoritative; diagnostics safe-copy passes but file export does not (§10).
- V13-05 adaptive scheduler: logic PASS; native wake lifecycle not observed (§11).
- V13-06 tray/notification logic: backend/UI wiring present, but Claude trust escalation invalidates authoritative-only guarantees (§12-13).
- V13-07 release: FAIL due P0/P1 findings, stale lockfile, missing native qualifications, and dirty release worktree.

## 7. Multi-profile regression

**Isolation logic: PASS. Product scope: FAIL.**

Evidence independently inspected and executed includes:

- canonical-root uniqueness enforced in `source_connections` and tested by `same_root_cannot_silently_rebind_between_accounts`;
- default/custom rows separated by `default_and_custom_roots_never_share_rows` and `connections_isolate_default_and_custom_roots`;
- A/B/A routing plus disable/delete isolation covered by `profile_a_b_a_isolation_with_disable_and_delete`;
- per-scope refresh/cancellation and account-lifecycle validation are keyed by account scope;
- cooldown table key is `(account_id, source)`;
- notification mute is stored per account and independently round-tripped;
- explicit custom root resolution does not rebind to default.

However, production `build_strategy` returns `None` for Codex App Server, Claude PTY, and Claude OAuth whenever `custom_root.is_some()` (`crates/usage-runtime/src/refresh.rs:923-950`). Therefore custom Codex/Claude profiles cannot receive subscription quota at all. Isolation is safe, but the Master Spec goal of quota per local profile and §11.6 profile-scoped quota snapshots are not complete.

## 8. Codex live quota

**Design inspection: PASS. Current live requalification: FAIL/BLOCKED by local client health.**

Production flow is:

```text
static allowlisted candidates
-> bounded initialize health probe
-> positive-only health cache
-> owned child: initialize, account/read(refreshToken=false), account/rateLimits/read
-> bounded parser/mapper
-> account-scoped snapshot
```

Verified properties:

- no PATH lookup and no shell;
- executable existence is insufficient; each uncached candidate is health-gated;
- negative health is not cached;
- probe timeout 8 seconds, RPC timeout 20 seconds, response bound 512 KiB;
- cancellation and timeout kill only the owned child;
- `refreshToken` is explicitly false;
- JSONL fallback is a separate strategy and is non-authoritative.

The opt-in real test was run with `USAGE_AI_LIVE_CODEX=1`. It failed at availability: `NotConfigured` instead of expected `Ready`; no quota response was obtained. This does not prove a code regression by itself because the installed client/shims may be unhealthy, but it prevents this audit from independently renewing the previous live claim.

## 9. Claude quota qualification

**CLAUDE_LIVE_POSITIVE = FAIL**

Actual production order is:

```text
claude-pty-usage
-> claude-oauth-usage (production oauth_url = None; dormant)
-> claude-local-jsonl (history only)
```

The primary performs a bounded, shell-free `claude auth status`, then owns a PTY and writes only `/usage` followed by `/exit`. Parsing accepts percentage-bearing session/week/model/limit lines; reset time is accepted only as RFC3339. It uses a synthetic-v1 screen grammar and marks window kind `Unknown`.

Audit host evidence:

- a real installed Claude CLI exists;
- safe `auth status` returned `loggedIn=true` (only the boolean was retained);
- `live_auth_status_shape_parses`: PASS;
- `live_quota_end_serves_or_degrades_cleanly`: **FAIL**, returning `Network` after the bounded PTY timeout;
- therefore no live window count, normalized window kinds, percentages, reset semantics, plan, or identity could be safely qualified.

The OAuth end is not a production fallback: `oauth_url` is hard-coded `None` in runtime wiring (`crates/usage-runtime/src/refresh.rs:947-950`). Both parser shapes remain explicitly unverified, yet return authoritative `Partial`. Claude quota is not production-ready.

## 10. Fidelity / PII

**Source DTO/UI fields: mostly PASS. Claude fidelity and diagnostics export: FAIL.**

Provider DTOs carry account/profile, selected source, coverage, freshness, last attempt, last success, current error, fallback indicator and fallback reason. SWR can display stale data and current error together.

Claude PTY is incorrectly treated as the primary verified source despite `unverified_window_set`; it receives `Partial`, which central policy treats as authoritative. OAuth is flagged as fallback in DTO, but its `Partial` coverage still permits authoritative side effects.

PII tripwire results:

- `npm run check:pii` on the repository: PASS (`PII tripwire: clean`).
- Independent positive probe in an external temporary Git repository: PASS; an unallowlisted email caused non-zero failure.
- Negative allowed repository content: PASS.
- The script is not part of `npm run check`, the GitHub CI workflow, or the production export path.

Safe-copy uses an explicit rendering allowlist and redacts secret-shaped aliases/warnings (`diagnostics.rs:254-283`). File export does not use that path: `diagnostics_export_payload` serializes `get_diagnostics_inner`, whose `account_alias` and `identity_note` are raw (`diagnostics.rs:235-240, 355-377`). This violates the requirement that diagnostics/export contain no email, raw token, private path, or arbitrary user text if a hostile value is used as an alias/note.

## 11. Adaptive scheduler

**LOGIC = PASS. NATIVE WAKE = BLOCKED_BY_ENVIRONMENT.**

Verified implementation/tests:

- active cadence 120 seconds;
- normal floor 300 seconds, with configured 5/10/15/30/60 minute choices;
- offline interval multiplied by 4 plus request-level offline skipping/backoff;
- manual refresh is immediate but cannot bypass server cooldown;
- wake is detected after a gap greater than three configured intervals;
- due partition prevents idle physical fetches and serves LKG;
- coalescing is registered before semaphore admission;
- persisted cooldown is keyed by account/source and restored after coordinator restart;
- queued/in-flight/shutdown/account cancellation tests pass;
- one physical execution for concurrent production triggers is tested.

No unbounded provider polling path was found. A real sleep/wake event was not performed.

## 12. Tray

**LOGIC = FAIL because of Claude trust escalation. GUI = BLOCKED_BY_ENVIRONMENT.**

Pin selection is exposed in settings and persisted as `trayProfileAccountId`. Unknown/deleted/unavailable pinned accounts fall back to automatic selection. Selection otherwise filters for fresh coverage allowed by `allows_tray_metric`.

That filter is sound for `UnverifiedSemantics`, but not for Claude, because synthetic Claude results are mislabeled `Partial`, and `Partial` is allowed. Thus an unverified Claude percentage can win automatic selection or be shown for a pinned account.

The packaged process initialized tray lifecycle, but no native app accessibility surface was available to validate pin behavior visually.

## 13. Notifications

**LOGIC = FAIL because of Claude trust escalation. NATIVE_NOTIFICATION_SMOKE = BLOCKED_BY_ENVIRONMENT.**

Positive code evidence:

- notification titles include profile alias/provider/window;
- per-account mute persists in SQLite and is applied before planning;
- deduplication is persistent;
- verified providers can produce threshold notices;
- explicit `UnverifiedSemantics` inputs are suppressed;
- cross-account keys include account ID.

Failure: Claude PTY/OAuth use `Coverage::Partial`; `Partial` passes `allows_quota_notifications`, so synthetic/unverified Claude data can produce authoritative native quota notices.

There is no separate safe native-notification test endpoint. Creating artificial persistent usage records was not authorized, so no notification was emitted and no visual PASS is claimed.

## 14. Launch at login

**LOGIC = PASS. LAUNCH_AT_LOGIN_SMOKE = BLOCKED_BY_ENVIRONMENT.**

The application installs `tauri-plugin-autostart`; saving settings calls `enable()`/`disable()` through the plugin and persists the setting atomically via a user-only temporary file and rename (`src-tauri/src/commands/settings.rs:57-67`). The setting is present in the real UI.

No native application automation surface was available. The audit did not alter the user's login-item registration, reboot/log out, or claim a native relaunch PASS.

## 15. Intel artifact inspection

Artifacts found:

```text
new DMG: target/x86_64-apple-darwin/release/bundle/dmg/Usage.ai_1.3.0_x64.dmg
size: 6,811,933 bytes
mtime: 2026-09-11 15:53:39 +0500
SHA-256: d1e0c8d6639403888e3fd612b789bfdcbdbe77603bb8eab0aa9e437b2cea89ce

standalone app executable SHA-256:
14fb85e4e8859692b3a46f396a9d5549c27345ab9fceb10e57ebe69fd649d63f
```

Architecture and bundle metadata:

```text
Mach-O 64-bit executable x86_64
CFBundleIdentifier = com.nurasss.usageai
CFBundleShortVersionString = 1.3.0
CFBundleVersion = 1.3.0
LSMinimumSystemVersion = 13.0
```

`hdiutil verify` reported a valid checksum. The DMG mounted read-only and contained `Usage.ai.app` plus an Applications link. Its executable hash exactly matched the standalone app; metadata and architecture also matched. Detach completed cleanly.

Artifact limitations:

- app is unsigned (`codesign`: “code object is not signed at all”); a quarantined download/standard Gatekeeper clean-install path was not qualified;
- artifact timestamp precedes final `bd0ca57` commit by about six minutes. The only later source change is a Rust test, so production code is equivalent to `98f0cb0`, but the artifact is not provenance-bound to the final commit;
- stale committed lockfile prevents claiming a clean locked build from the tagged tree.

## 16. Packaged runtime smoke

**BASIC RUNTIME = PASS. FULL GUI/CLEAN-INSTALL = BLOCKED_BY_ENVIRONMENT.**

On the supported x86_64 macOS 13.7.8 host, the packaged executable:

- launched directly from the standalone bundle;
- remained alive for 36 seconds;
- initialized scheduled Claude/Codex refresh and tray lifecycle;
- produced no recent macOS crash report;
- exited promptly on `SIGTERM`.

This is runtime sanity only. It is not a visual tray/window qualification, notification smoke, login-item relaunch, DMG drag-install, or downloaded/quarantined clean install.

## 17. Read-only source mutation check

Top-level size/mtime/mode metadata for the available stores was recorded before and after the actual live probes and packaged smoke:

```text
Codex auth.json: unchanged
Codex sessions directory: unchanged
Codex archived_sessions: absent
Claude top-level config file: unchanged
Claude config directory: unchanged
```

No credential values were read or reported. The app-server request explicitly uses `refreshToken=false`, and the Claude driver only used `auth status`, `/usage`, and `/exit`.

A broader recursive Codex-session metadata hash changed during an earlier interval in which an external Codex session was concurrently active; the actual controlled probe interval's top-level metadata remained unchanged. Therefore the audit claims no observed source-store mutation by Usage.ai, but does not attribute unrelated concurrent session-file activity to the app.

## 18. Release matrix vs Master Spec v1.3

Each requirement is assessed separately.

| Requirement | Status | Reason |
|---|---|---|
| Codex verified quota | BLOCKED_BY_ENVIRONMENT | design/tests strong; current live App Server availability failed |
| Claude quota | FAIL | logged-in positive path timed out; OAuth dormant; synthetic results escalated to authoritative |
| Codex history isolation | PASS | evidence tests and scoped import regression pass |
| Claude history isolation | PASS | reducer/import/profile isolation tests pass |
| OpenAI API exact Decimal cost | PASS | lexical JSON number -> Decimal -> SQLite TEXT -> Decimal aggregate -> string DTO; targeted tests pass |
| Multi-profile isolation | PASS | no observed cross-account contamination |
| Multi-profile subscription quota | FAIL | quota ends disabled for custom roots |
| Credential ownership/read-only | PASS | scoped access, no refresh/writeback path; controlled metadata unchanged |
| Zero-inference monitoring | PASS | only quota/status commands and usage endpoints found; Claude sends only `/usage` and `/exit` |
| Explicit confidence-aware fallback | FAIL | Codex fallback honest; Claude PTY synthetic primary is not honest |
| Source fidelity visible | FAIL | fields exist, but Claude fidelity value is wrong |
| Diagnostics PII | FAIL | safe-copy passes; file export serializes raw user-controlled DTO text |
| SWR/LKG | PASS | LKG + stale + current error path and tests pass |
| Adaptive refresh | PASS | policy/coalescing/cooldown/offline tests pass |
| Tray pin persistence | PASS | production persistence and logic tests pass |
| Tray authoritative-only | FAIL | synthetic Claude `Partial` is eligible |
| Named notifications | PASS | title includes profile/provider/window |
| Mute persistence/isolation | PASS | per-account storage and UI wiring pass |
| Notifications authoritative-only | FAIL | synthetic Claude `Partial` is eligible |
| Native notification smoke | BLOCKED_BY_ENVIRONMENT | no safe native test endpoint/GUI surface |
| Launch-at-login implementation | PASS | Tauri autostart enable/disable and persistence wired |
| Launch-at-login native smoke | BLOCKED_BY_ENVIRONMENT | not performed; no native app automation surface |
| Migration safety | PASS | v8->v9, historical DB, atomic migration tests pass; live DB integrity OK |
| SQLite permissions/integrity | PASS | DB/WAL/SHM are 0600; integrity OK; schema v9; no FK violation output |
| Secrets absent from current SQLite | PASS | schema/query-level marker checks found none; no raw values dumped |
| Intel architecture | PASS | standalone and DMG app are x86_64 |
| Bundle version/identifier | PASS | 1.3.0 and `com.nurasss.usageai` |
| DMG integrity/content | PASS | checksum valid; expected app; embedded/standalone binary identical |
| Packaged basic runtime | PASS | alive 36 seconds; no immediate crash |
| Clean Intel install / full GUI smoke | BLOCKED_BY_ENVIRONMENT | no quarantined install/tray-window visual qualification |
| Mandatory six gates | PASS | all returned success; warnings noted |
| Locked reproducible build | FAIL | committed Cargo.lock stale |
| Clean release tree | FAIL | three pre-existing unstaged modifications |
| Independent audit | PASS | this report; audit found release blockers |

## 19. README / repository hygiene

The current README change is working-tree-only; it is not in HEAD or `v1.3.0-rc1`. It must not enter the final release tree without separate proof/correction. Unsupported/misleading claims in the diff include:

- monitoring Gemini and Ollama as shipped providers, although both are out of v1.3 scope/discovery-only;
- both Intel and Apple Silicon builds, while the inspected artifact is x86_64 only and Apple Silicon is not a gate;
- `usage-providers` drivers for Gemini/Ollama;
- encrypted token/cache storage, while credential policy uses scoped Keychain/read-only client stores and SQLite is not described as encrypted;
- Tailwind CSS, which is not declared in package dependencies;
- “official quota APIs” as a blanket statement, while Codex uses supported-client RPC and Claude uses an unqualified PTY parser.

The stale artifact `target/x86_64-apple-darwin/release/bundle/macos/rw.53342.Usage.ai_0.1.0_x64.dmg` remains present (45,127,680 bytes; SHA-256 `ffa236082cc4ef82b635ca9e67108302b7e39701de329b05846dea6b629c6255`). It was not deleted. CI uses a fresh target directory and does not upload artifacts, so no proven automated stale-publish path exists; however its diagnostic `find ... '*.app' -o ... '*.dmg'` would list all matching bundles in a non-clean target.

Disk status: Data volume is approximately 96% used with about 4.4 GiB available; repository `target/` is approximately 12 GiB. Existing-cache gates passed, but a truly fresh full Intel rebuild may be affected by disk pressure. No build cache was deleted.

The requested normative filename is absent from Git. The audit used the supplied external copy. This weakens release provenance and should be resolved before the next RC without modifying the document during this audit.

## 20. Ignored/live tests

Four tests are ignored by the normal Rust suite:

| Test | Reason/requirement | Audit result |
|---|---|---|
| `codex::tests::real_codex_corpus_probe_is_opt_in_and_never_checked_in` | needs explicit `CODEX_REAL_FIXTURE` pointing to user-supplied real/redacted corpus | Not run; corresponding checked-in controlled real-corpus evidence tests passed |
| `codex_appserver::live_acceptance::live_app_server_reports_verified_quota` | needs installed logged-in healthy Codex App Server and `USAGE_AI_LIVE_CODEX=1` | Run; **FAIL**, availability `NotConfigured` vs `Ready` |
| `claude::live_acceptance::live_auth_status_shape_parses` | needs installed Claude CLI and `USAGE_AI_LIVE_CLAUDE=1` | Run; **PASS** |
| `claude::live_acceptance::live_quota_end_serves_or_degrades_cleanly` | needs installed Claude CLI/login and `USAGE_AI_LIVE_CLAUDE=1` | Run with confirmed login; **FAIL**, `Network` timeout |

Ignored tests are acceptable as opt-in mechanics. Here, however, the failed Claude positive test corresponds directly to a required production-ready DoD item and is a release gap.

## 21. P0/P1 regression assessment

### P0

1. **Unverified Claude quota is authoritative.** Synthetic/unreconciled PTY and OAuth shapes return `Partial`; central policy lets `Partial` drive tray and quota notifications. This is an explicit Master Spec P0 blocker.

### P1

1. **Claude positive production path failed on a logged-in host; OAuth fallback is dormant.**
2. **Custom Codex/Claude profiles have no subscription-quota strategy.** Isolation passes by removing functionality, not by account-scoped quota binding.
3. **Diagnostics file export bypasses safe-copy redaction for raw alias/identity-note text.**
4. **RC commit has a stale Cargo.lock and fails `--locked`.**
5. **Required native clean-install, tray/window, notification, and launch-at-login smokes remain unqualified.** These are environmental qualification gaps rather than demonstrated code defects.

No credential mutation, cross-profile row contamination, inference request, money `f64` conversion, destructive migration, or overlapping coalesced physical fetch was found in the inspected paths/tests.

## 22. Final verdict

**RELEASE REJECTED**

Answer to the release question:

> No. The current `v1.3.0-rc1` cannot safely receive final tag `v1.3.0` without violating the Master Spec v1.3 Definition of Done.

This is not merely an environmental hold. A P0 trust-classification defect exists in production code, and Claude production readiness has a real logged-in positive-path failure. The dirty/stale lockfile and incomplete native qualification add independent release gaps.

## 23. Exact next action

Do **not** create or push `v1.3.0` on `bd0ca57fca2b84938d4d79099d34763930b0645c`.

Minimum release requalification checklist for a new RC:

1. Remove the P0 Claude trust escalation: every unreconciled/synthetic Claude quota result must be non-authoritative end-to-end and unable to drive tray/notifications.
2. Qualify the real logged-in Claude `/usage` shape and make the positive production-equivalent live test pass, or remove Claude quota from the v1.3 production-ready claim/scope through an explicitly revised normative release decision.
3. Provide account-scoped subscription-quota behavior for custom Codex/Claude profiles, or explicitly revise the normative per-profile quota requirement.
4. Apply the same allowlist/redaction contract to diagnostics file export and add a production-export positive/negative PII test.
5. Commit the correct `Cargo.lock`; require `cargo check/test/clippy --locked` on a clean candidate.
6. Exclude or substantiate the dirty README claims and produce a clean release worktree.
7. Rebuild `.app`/DMG from the exact new RC commit, record provenance, and complete clean-install, tray/window, native notification, and launch-at-login smokes on supported Intel macOS.
8. Repeat this independent read-only audit on the new RC.

No tag, commit, push, code remediation, README change, artifact deletion, or source-owned credential/session modification was performed by this audit.
