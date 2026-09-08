# OpenAI API Integration Spec

Status: **Implemented (v1.1)** — strategy `openai-api-costs` (`OfficialApi`), connector `openai-api-connector-v1`, parser `openai-costs-page-v1`. Live data appears only after the user configures an Admin connection; otherwise the product stays `NotConfigured`/`AuthenticationRequired` with no fabricated values.

0. v1.1 pipeline: one logical refresh performs one bounded paginated fetch set whose single parsed payload feeds both the snapshot and the cost import in one storage transaction; an unfinished tail degrades coverage to `Partial` (`pagination_bounded_partial`) instead of truncating silently; typed `configure_openai_admin_connection` is the only secret-writing IPC.

1. Data: reported organization cost buckets by day, split by project (`billing_scope_id`) and line item; no quota pools from this source.
2. Source: documented `GET https://api.openai.com/v1/organization/costs` (allow-listed host only, HTTPS, no redirects with secret, rustls WebPKI roots).
3. Classification: official API.
4. Permissions: Admin API key with costs read scope; 403 surfaces as `InsufficientScope`, never as zero spend.
5. Identity: `SHA-256("openai-api" || key)` truncated to 16 hex chars — stable, non-privileged, never the key itself.
6. Pools: none; subscription quota is separate.
7. Units: decimal currency grouped by currency (never converted/summed across currencies); org total vs project subsets deduplicated by the aggregation rule.
8. Windows: API reporting buckets; countdown semantics do not apply.
9. API usage/cost: Reported cost imported idempotently for the trailing 30 days; token/request counts are not supplied by this endpoint and stay unknown.
10. Delay: labeled `ProviderDelayed`; `observed_at` is the newest bucket end, `fetched_at` is local time.
11. History: remote reporting range (30 days), merged with local history by `(account, product, source, source_record_id)`.
12. Pagination: single page `limit=180`; `has_more=true` degrades coverage to `Partial` with no silent truncation.
13. Rate limits: 429 honors `Retry-After` into a per-account cooldown; manual refresh never bypasses it.
14. Errors: 401 authentication, 403 insufficient scope, 429 cooldown, 3xx redirect rejected, schema drift is `ParseError`, timeout is 15 s per source.
15. Stable fields: only `data[].{start_time,end_time,results[].{amount.{value,currency},project_id,line_item}}`; incomplete rows are skipped with warnings.
16. Fixture: synthetic cost page in `openai_api::tests` (no real key, org, or project).
17. Forbidden: Admin/API key, headers, email, raw responses with PII — redacted at the HTTP boundary; Keychain holds only Usage.ai-configured keys.
18. Closed client: yes, independent API connection.
19. Auth mutation: none on foreign credentials; Usage.ai-owned key only, Keychain service `com.nurasss.usageai`.
20. Verification: reconcile totals with provider billing/usage console for the same organization, project, bucket, and timezone.

