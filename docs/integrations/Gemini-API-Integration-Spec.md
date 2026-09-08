# Gemini API Integration Spec

Status: **Discovery — Google Cloud project/scopes not configured**.

1. Data: target usage and Cloud-reported cost where accessible. 2. Source: documented Google Cloud Monitoring/Billing interfaces. 3. Classification: official API. 4. Permissions: scoped Cloud project/billing read roles. 5. Identity: project/billing account IDs fingerprinted. 6. Pools: API quotas are kept separate by metric/location/model. 7. Units: requests/tokens and decimal billing currency. 8. Windows: source-defined alignment/rolling windows. 9. Usage/cost: research/target. 10. Delay: Cloud reporting delay must be measured. 11. History: remote time series. 12. Pagination: documented page token. 13. Rate limits: per Google metadata. 14. Errors: 401 auth, 403 scope, 429 project cooldown. 15. Stable fields: documented schema only. 16. Fixture: redacted metric points. 17. Forbidden: service-account private key in DB/export/log, labels with PII, raw response. 18. Closed client: yes. 19. Auth mutation: Usage.ai-owned auth only. 20. Verification: reconcile project, metric, and billing period with Cloud Console.

