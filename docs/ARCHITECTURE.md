# Architecture

The workspace follows the Master Spec boundaries:

- `crates/usage-core`: provider-neutral models, capability system, aggregation, scheduler/backoff, notification deduplication, safe diagnostics.
- `crates/usage-providers`: isolated connectors/parsers/mappers and redacted synthetic fixtures.
- `crates/usage-storage`: SQLite migrations, idempotent upserts, import checkpoints, retention, safe export.
- `src-tauri`: tray lifecycle, IPC, Keychain abstraction, app data ownership, native plugins.
- `src`: Svelte panel, overview, provider cards, settings, keyboard and accessibility behavior.

The UI has no direct network, filesystem, or Keychain access. Provider adapters declare capabilities and network allow-lists. A failure is scoped to one account/source.

## Data invariants

- Missing is distinct from zero.
- Money is decimal text and remains grouped by currency.
- Reported cost supersedes an estimate for the same account, billing scope, and period.
- Cache/reasoning token categories do not increase `total_tokens` when the source already reports a total.
- Source observation time and local fetch time are separate.
- Local path identities are SHA-256 fingerprints; paths never enter diagnostics or export.

## Refresh flow

1. Return the last valid in-memory/SQLite snapshot immediately.
2. Compute countdowns in the UI without a request.
3. Refresh each enabled account independently with a source timeout.
4. Validate, then atomically replace a snapshot.
5. Use `Retry-After`, otherwise exponential backoff with jitter. Authentication errors do not retry aggressively.

