# Claude Code Integration Spec

Status: **history Implemented (v1.1)** — strategy `claude-local-jsonl`, schema `claude-jsonl-assistant-v1`; **subscription quota V13-03/remediation**: ordered ends `claude-pty-usage` (SupportedClientApi, live-degrading) + `claude-oauth-usage` (OfficialApi, dormant contract) ahead of the history end. Window shapes are SYNTHETIC v1 (`Coverage::UnverifiedSemantics`, never authoritative) until a logged-in host reconciles them.

0. Source order (v1.3): `claude-pty-usage` first, `claude-oauth-usage` second, `claude-local-jsonl` (history end, quotas always empty) last. The PTY end is account-scoped (§11.5): every scope is driven with `CLAUDE_CONFIG_DIR` set to its own root (A/B isolation proven with per-root plans through the real forkpty backend); a root without login surfaces auth state, never another profile. The OAuth end stays default-only (no per-profile credential source exists). Custom `~/.claude-<slug>` profiles keep JSONL observation — no cross-profile credential fallback, ever.

0.1. PTY end (verified behaviorally, not yet against a subscription): installed `claude` CLI driven through an owned forkpty child (`/usage`, then `/exit`, nothing else — $0.0000 spend proven across all probes). Staged drive inside one caller-bounded deadline: first paint (nothing is ever typed before the session demonstrably paints; a foreign trust/approval prompt aborts before the first keystroke; a silent binary is `Unavailable`, never `Parse`), then `/usage`, render-until-decisive, always `/exit` + kill + reap. Fast login gate first (`claude auth status`, observed shape `{"loggedIn":bool,...}`, ~1s, non-interactive): not-logged-in is `NotConfigured` (skip without cooldown — the truth is "sign in"); only a proven login is `Ready`. Executable candidates are static (no PATH/shell); a dead shim fails the gate fast (no first-exists-wins flaw).

0.2. Screen parser is STRICT and locale-pinned (child spawns with `LANG=C LC_ALL=C`): ANSI-stripped text, `label-with-window-hint + ASCII-dot percent` stanzas, RFC3339 resets only, plans/tiers length-capped and never email-like. OBSERVED login markers (oauth authorize URL + paste-code prompt, captured 2026-09-10) classify as `AuthenticationRequired`. Anything else is `Parse` — the parser cannot fabricate from unknown screens. No denominator is ever synthesized from JSONL (§10.3.3); percents stand alone; windows stay `WindowKind::Unknown`.

0.3. OAuth end is a dormant contract: transport (Bearer via `FetchContext.secret` only, allow-listed hosts, bounded, 401/403/429/5xx classified) + parser + ordering tests are implemented and green, but production wires no URL and no host, so availability stays `NotConfigured`. No endpoint is fabricated; no Keychain/credential store is opened (on this host the OAuth token lives outside any file — `auth status` reports `loggedIn:false`, `hasAvailableSubscription:false`).

0.4. Fallback semantics (§8): quota-end failure never fails the refresh — the coordinator falls through to the history end (still `Connected`), and `quota_fallback` banners any shown quotas not served by `claude-pty-usage`. Auth failures set no cooldown (history keeps serving every refresh); Parse/Network failures take the standard transient backoff.

0.5. Live evidence 2026-09-10 (no subscription login on host): `auth status` shape parses live; full strategy degrades `NotConfigured` → `AuthenticationRequired`, bounded, no hang, no spend. Real quota remains BLOCKED on login — stated, not worked around.

0. v1.1 semantics: identity priority `message.id + requestId` → `uuid` → `path:offset`, so repeated streaming chunks share one record and never inflate totals; cache = read + creation; a reachable history source reports `Connected` with history capabilities only (no `subscriptionQuota`), an absent one reports `Unavailable`.

1. Data: model, input/output/cache token categories and request time.
2. Source: read-only Claude Code project JSONL.
3. Classification: local client history.
4. Permissions: read selected Claude projects directory.
5. Identity: not proven by file; roots remain separate connections.
6. Pools: no verified quota pool in this source.
7. Units: integer tokens.
8. Windows/reset: unavailable; no value is inferred.
9. API billing: unavailable from this source.
10. Delay: local event timestamp.
11. History: yes, from locally retained sessions only.
12. Pagination: byte checkpoint; malformed records are skipped with safe warnings.
13. Rate limits: no network request.
14. 401/403/429: not applicable to local import.
15. Stable fields: assistant message usage keys observed; unknown keys ignored.
16. Fixtures: synthetic assistant message with token categories.
17. Forbidden: message content, prompts, responses, credentials, email, raw events, personal path.
18. Closed client: existing history remains readable.
19. Auth mutation: credential file and Keychain item are not opened or changed.
20. Manual verification: compare imported token totals/models with Claude Code session statistics; quota needs a separately verified source.

