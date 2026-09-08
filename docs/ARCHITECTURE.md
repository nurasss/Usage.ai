# Architecture (v1.1)

Provider code never getsambient OS access. `src-tauri` is composition only.

```text
Svelte UI (descriptors-driven, capability-driven, stable account-aware IDs)
  │ narrow typed IPC (no generic secret command, Save dialogs for export)
  ▼
src-tauri: platform/composition only
  commands/  platform/{tray,window,lifecycle,hosts}  refresh_flow  dto  appstate
  │ delegates to
  ▼
usage-runtime (testable orchestration)
  registry (descriptors + kill-switches) · refresh (coalescing, semaphore,
  cancellation generations, wired backoff, persistent cooldowns, offline-aware)
  snapshot (SWR last-known-good) · history_import (scoped, batched, rotation-aware)
  account_router (no auto-merge) · notifications (planner; platform delivers)
  │             │                     │
  ▼             ▼                     ▼
usage-core  usage-storage           usage-providers
typed IDs   repositories            descriptors + strategy pipeline
policy      snapshots LKG           connector/parser/mapper split
outcomes    attempts/cooldowns      fixtures per schema case
            batch transactions
            retention categories
  │                                   │
  └───────── scoped HostServices ──────┘
                    ▼
               usage-host
  Clock / Logger(redacting) / Keychain(typed) / HTTP(allow-list, no redirects)
  Files(scoped roots, symlink policy, dev/ino identity) / Network(observed)
```

Key contracts:

- One logical OpenAI refresh = one shared parsed payload for snapshot and costs (P0-1).
- A failed refresh never deletes last-known-good; the UI gets stale data + current error (P0-2).
- Packaged backend failures render an explicit error state, never demo numbers (P0-3).
- `window_kind` is `Unknown` until window semantics are proven; countdown expiry never proves a reset.
- Unknown models, projects and accounts render as labeled unknowns, never as zero.
- Reported and Estimated costs never merge without explicit user choice; currencies never convert.
- Local identity without proof is `Unknown`/`Weak` and is never auto-merged across accounts.
