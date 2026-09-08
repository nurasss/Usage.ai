# Usage.ai

> [Русская версия](README.md)

Local-first macOS menu bar utility for AI quota, usage, cost, and history. The first target is Intel (`x86_64-apple-darwin`) on macOS 13+.

The application never sends inference requests to monitor usage. Only data whose source, account, coverage and freshness the app can explain is shown.

## Development

Requirements: Node.js, Rust stable, Xcode Command Line Tools, and macOS 13 or newer.

```bash
npm install
npm run check
npm test
cargo test --workspace
npm run tauri dev
```

Build an Intel application and DMG on an Intel Mac:

```bash
rustup target add x86_64-apple-darwin
npm run tauri build -- --target x86_64-apple-darwin
file target/x86_64-apple-darwin/release/bundle/macos/Usage.ai.app/Contents/MacOS/usage-ai-app
```

## Current integration status (v1.1)

- Codex: `codex-local-jsonl` strategy — quota from all session tails, token history as request deltas.
- Claude Code: `claude-local-jsonl` strategy — history with `message.id + requestId` dedup; subscription quota blocked (no verified source).
- OpenAI API: one fetch per refresh (snapshot + costs in one transaction), bounded pagination; activates via the typed Admin configuration, otherwise honest `NotConfigured`.
- Other products: `DiscoveryOnly`, no fabricated metrics (Gate A: no new private sources until P0 closes).
- Architecture: `usage-runtime` (coalescing, bounded parallelism, cancellation, SWR last-known-good, persistent cooldowns), `usage-host` (scoped OS access), descriptors as the single UI source of truth.
- Security: typed Keychain IPC, Save dialogs for export, 0600 files, allow-listed HTTPS with no secret-bearing redirects.

See [Phase 0 discovery](docs/PHASE0_DISCOVERY.md) and the [integration matrix](docs/INTEGRATION_MATRIX.md).

## Privacy

Own API secrets are stored in macOS Keychain. SQLite, logs, fixtures, diagnostics, and exports contain no secrets, prompts, responses, emails, browser cookies, or raw provider responses. Local session files are read incrementally and never modified.
