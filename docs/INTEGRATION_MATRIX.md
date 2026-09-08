# Final integration matrix (working)

`Verified` means the observed source schema is supported in code. It does not imply every capability exists. `Blocked` deliberately renders no fabricated value. `Configured` means the connector is implemented and activates only with user-supplied credentials/paths.

| Product | Quota | Local history | API usage/cost | Balance | Status |
|---|---|---|---|---|---|
| ChatGPT consumer | no verified source | n/a | n/a | no verified source | Blocked by source |
| Codex | verified local snapshot | verified | n/a | observed but not persisted until semantics verified | Partial/verified |
| OpenAI API | n/a | optional | implemented (`openai-api-connector-v1`, Admin key required) | no verified endpoint — stays unknown | Configured |
| claude.ai | no verified source | partial through Claude Code only | n/a | no verified source | Blocked by source |
| Claude Code | no verified quota source | verified | n/a | no verified source | Partial/verified |
| Claude API | n/a | optional | account/scopes not configured | account/scopes not configured | Blocked by credentials/scopes |
| Gemini consumer | no verified source | no verified source | n/a | no verified source | Blocked by source |
| Antigravity | no stable RPC/source verified | no verified source | n/a | no verified source | Blocked by source |
| Gemini API | n/a | optional | Cloud project/scopes not configured | no verified source | Blocked by credentials/scopes |
| Z.ai GLM Coding Plan | no installed source verified | no installed source verified | n/a | no verified source | Blocked by source |
| Z.ai API | n/a | optional | account not configured | account not configured | Blocked by credentials |
| OpenCode Zen | no verified source | client config exists; history location unverified | no verified source | no verified source | Blocked by source |

