# Claude Code Integration Spec

Status: **local history verified; subscription quota blocked**. Parser version `claude-code-local-session-v1`.

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

