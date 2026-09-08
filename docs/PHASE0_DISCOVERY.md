# Phase 0 discovery — 2026-09-08

This report records observations, not assumptions. No credential contents, account identity, personal paths, prompts, or responses were captured.

## Target environment

| Item | Observed value |
|---|---|
| macOS | 13.7.8 (22H730) |
| Architecture | x86_64 |
| CPU | Intel Core i5-7360U 2.30 GHz |
| Xcode | 13.3 |
| Rust | 1.98.1, host `x86_64-apple-darwin` |
| Node.js / npm | 25.9.0 / 11.12.1 |
| Minimum macOS selected | 13.0, pending packaged-app smoke test |
| Distribution | personal ad-hoc `.app` / `.dmg`; updater disabled |

## Installed clients and observed sources

| Product | Client | Version | Source observed | Result |
|---|---|---:|---|---|
| ChatGPT | `/Applications/ChatGPT.app` | 26.831.20005 | no permitted documented quota source verified | Discovery blocker |
| Codex | local sessions and Codex app | app 4.0.8 | `sessions/**/rollout-*.jsonl` | token history and rate-limit snapshots verified read-only |
| Claude Code | local sessions | schema observed | `projects/**/*.jsonl` | token/model history verified read-only |
| claude.ai | Claude desktop | 1.40609.0 | no permitted documented quota source verified | Discovery blocker |
| Antigravity | desktop app | 2.12.2 | application data exists; no RPC contract probed | Discovery blocker |
| OpenCode | config present | version not established | no Zen usage source verified | Discovery blocker |
| Z.ai | not observed as a standalone installed client | unknown | none | Discovery blocker |

## Verified schemas

Codex session events expose a `token_count` payload with last/total token categories and optional dynamic `rate_limits` containing a limit identifier, primary/secondary windows, `used_percent`, window length, Unix reset timestamp, credits metadata, and plan type. The adapter validates percentages and derives remaining percentage without converting it to tokens.

Claude Code assistant events expose model and usage categories including input, output, cache read, cache creation, and optional extra categories. The importer uses input + output as the request total and stores cache separately, preventing double counting.

## Permissions and safety

- Reads are scoped to explicitly known session roots or a user-selected custom path.
- Foreign auth/config files are not written or refreshed.
- Keychain has not been scanned. Usage.ai requests only its own service/item after the user configures an account.
- Browser storage is excluded.
- Antigravity caches/session storage were not parsed because no stable permission/source contract is verified.

## Remaining manual gates

- Run and visually compare each real integration against the authoritative client on the same account and time window.
- Supply/configure API accounts and scopes for OpenAI, Anthropic, Gemini/Cloud, Z.ai, and Zen when those products are desired.
- Decide whether consumer subscription products with no permitted source remain explicit blockers or are removed from v1 scope.
- Smoke-test tray positioning, wake behavior, notifications, launch-at-login, `.app`, and `.dmg` on this Mac.
- Measure p95 cached panel open, cold start, idle CPU, and memory using the packaged build.

