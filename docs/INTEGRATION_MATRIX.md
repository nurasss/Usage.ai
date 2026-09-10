# Final integration matrix (v1.1)

One status per product (§39): `Implemented` / `Partial` / `DiscoveryReady` / `DiscoveryOnly` / `Blocked` / `Disabled`.
A `DiscoveryOnly` card never passes the provider DoD. No metric is fabricated while source/auth verification remains incomplete.

| Product | Baseline v1.0 | v1.1 |
|---|---|---|
| Codex local quota | Partial | Implemented parser/all-files tail scan; real `token_count` shape probe passed, window semantics remain `Unknown` |
| Codex history | Partial | Rotation-aware import and stable source ids; request/delta meaning remains `UnverifiedSemantics` pending redacted corpus reconciliation |
| Codex App Server | Missing | Research only — frozen by Gate A, no production code |
| ChatGPT consumer | DiscoveryOnly | DiscoveryOnly — no promise without a permitted source |
| OpenAI API Costs | Partial | Implemented (single logical fetch, bounded pagination, one-transaction commit) |
| OpenAI API Usage | Missing | Missing — no verified tokens endpoint for the available scopes |
| Claude Code history | Partial | Implemented reducer (out-of-order `message.id + requestId`/UUID identity, cache read+creation buckets); real-client semantics still require source reconciliation |
| Claude subscription quota | Missing | Missing — implemented only after a verified source |
| Claude API | Missing | Missing — implemented only with Admin usage scope |
| claude.ai | DiscoveryOnly | DiscoveryOnly — no promise without a permitted source |
| Antigravity | DiscoveryOnly | DiscoveryOnly — frozen by Gate A, candidates only |
| Gemini API | DiscoveryOnly | DiscoveryOnly — separate integration after auth/billing validation |
| Z.ai Coding / API | DiscoveryOnly / Missing | DiscoveryOnly / Missing — contract validation first |
| OpenCode local / Go / Zen | Missing / Missing / DiscoveryOnly | Unchanged — product separation required before any billing work |
