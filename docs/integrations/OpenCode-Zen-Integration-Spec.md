# OpenCode Zen Integration Spec

Status: **Discovery blocker — config exists, Zen usage/billing source not verified**.

1. Data: target local token history, Zen quota/usage/cost/balance if available. 2. Source: documented Zen billing/usage interface plus selected local OpenCode sessions. 3. Classification: unknown until docs/source inspection. 4. Permissions: scoped read access. 5. Identity: Zen billing-account fingerprint, not model name. 6. Pools: Zen must stay distinct from direct Anthropic/OpenAI APIs. 7. Units: tokens and provider-reported decimal currency/credits. 8. Windows: unknown. 9. Usage/cost: target/research. 10. Delay: unknown. 11. History: local target. 12. Pagination: byte checkpoints locally; documented cursor remotely. 13. Rate limits: unknown. 14. Errors: standard isolated mapping. 15. Stable fields: none captured. 16. Fixture: none until redacted schema capture. 17. Forbidden: provider keys in OpenCode config, prompts/responses, raw sessions, PII. 18. Closed client: local history yes; remote unknown. 19. Auth mutation: forbidden. 20. Verification: reconcile Zen scope/cost with Zen console and ensure Claude-via-Zen is never counted as direct Anthropic cost.

