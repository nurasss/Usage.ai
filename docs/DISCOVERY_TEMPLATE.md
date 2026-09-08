# Discovery template for new integrations (§49 + §29.5)

Fill this in before any coding. A new source/provider PR is not accepted without it (§47).

```text
Provider / Product
Client / version
Source class (OfficialAPI / SupportedClientAPI / LocalStructuredData /
  OfficialExport / PrivateClientAPI / BrowserSession / HTMLFallback)
Endpoint / protocol / path
Auth owner
Credential location
Read / write behavior (must be read-only for foreign credentials)
Account identity (proof, namespace, confidence)
Capabilities (declared, source-specific)
Quota semantics
Window semantics
Reset semantics
History semantics
Reported cost semantics
Coverage
Pagination
Rate limits
Offline behavior
Known schema versions
Security risks
Kill switch id
Fixture status
Manual verification source
```

Threat model for private integrations (browser cookie / internal RPC / OAuth reuse):

```text
Secret owner
Required scopes
Rotation behavior (Usage.ai never rotates foreign refresh tokens)
Source host (exact allow-list)
Redirect behavior (deny by default for secret-bearing requests)
Storage (Keychain service/account, no SQLite/logs/export)
Revocation path
Account identity proof
Expected breakage mode (must degrade to stale + safe error, never crash)
```
