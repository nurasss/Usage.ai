# Security and privacy model

Usage.ai is a read-only, local-first aggregator. There is no Usage.ai backend or telemetry path.

## Trust boundaries

- Frontend: renders DTOs and issues named IPC commands only. CSP disallows remote scripts/content.
- Backend: owns filesystem, network, SQLite, Keychain, and notifications.
- Adapters: declare exact hosts; secret-bearing redirects outside the allow-list are rejected by connector implementations.
- Foreign clients: selected files are opened read-only. Usage.ai does not change tokens, ownership, permissions, login state, or config.

## Secret handling

Usage.ai-owned keys use Keychain service `com.nurasss.usageai`. SQLite stores a connection reference, never a secret. Redaction treats authorization words and secret-shaped strings as sensitive. Exports are constructed from a field whitelist.

## Explicit exclusions

No browser-cookie connector, mass Keychain scan, raw provider-response logging, prompt/response storage, remote HTML execution, arbitrary frontend shell command, or updater exists in v1 until a signed release channel is configured.

