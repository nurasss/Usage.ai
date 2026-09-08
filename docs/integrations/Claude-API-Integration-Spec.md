# Claude API Integration Spec

Status: **Discovery — account and billing scopes not configured**.

1. Data: target usage and reported cost. 2. Source: documented Anthropic Admin/usage interfaces only. 3. Classification: official API. 4. Permissions: least-privileged reporting scope. 5. Identity: organization/workspace identifier fingerprint. 6. Pools: none; Claude consumer/Code quota stays separate. 7. Units: tokens/requests and decimal currency. 8. Windows: provider reporting buckets. 9. Usage/cost: target, unverified. 10. Delay: measure and expose. 11. History: remote range if allowed. 12. Pagination: documented cursor. 13. Rate limits: honor response headers. 14. Errors: 401/403 no aggressive retry; 429 account cooldown. 15. Stable fields: documented schema only. 16. Fixture: redacted aggregates. 17. Forbidden: keys, headers, member data, prompts/responses, raw bodies. 18. Closed client: yes. 19. Auth mutation: Usage.ai key only in Keychain. 20. Verification: compare same workspace/time buckets with authoritative console export.

