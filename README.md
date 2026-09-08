# Usage.ai

Local-first macOS menu bar utility for AI quota, usage, cost, and history. The first target is Intel (`x86_64-apple-darwin`) on macOS 13+.

The application never sends inference requests to monitor usage. Unknown metrics remain unknown; mock data is visibly marked.

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
file target/x86_64-apple-darwin/release/bundle/macos/Usage.ai.app/Contents/MacOS/Usage.ai
```

## Current integration status

- Codex: verified read-only local session schema for quota snapshots and token history on the target Mac.
- Claude Code: verified read-only local JSONL token history; subscription quota source is not yet verified.
- OpenAI API: connector implemented (`GET /v1/organization/costs`, allow-listed HTTPS, Keychain-held Admin key); activates only after the user adds an account, otherwise honest `NotConfigured`.
- Other required products: adapter boundaries and Discovery records exist, but no metric is fabricated while source/auth verification remains incomplete.
- Panel honesty: cost tabs render only stored Reported/Estimated values, unknown models are labeled `Модель не определена`, and empty periods render `Нет данных` instead of zero.
- Diagnostics: per-provider safe diagnostics (`get_diagnostics`, separate `export_diagnostics`) with no secrets; accounts support add/enable/test-connection/custom-path/archive vs full delete.
- Budgets: monthly Reported-vs-budget alerts per currency, no conversion; balances render only when a source provides them.
- Refresh safety: per-source 15s timeout, per-account cooldown (manual refresh never bypasses it), single wake refresh, offline banner via cache + `navigator.onLine`.

See [Phase 0 discovery](docs/PHASE0_DISCOVERY.md) and the [integration matrix](docs/INTEGRATION_MATRIX.md).

## Privacy

Own API secrets are stored in macOS Keychain. SQLite, logs, fixtures, diagnostics, and exports contain no secrets, prompts, responses, emails, browser cookies, or raw provider responses. Local session files are read incrementally and never modified.

