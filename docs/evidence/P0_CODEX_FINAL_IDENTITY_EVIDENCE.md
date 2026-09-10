# 1. Evidence scope

This is the final narrow evidence audit of P0-05. It evaluates whether a Codex `token_count` observation has a 1:1 relationship with a real billable/logical model completion when `requestId` is absent.

Audited repository state:

- branch: `main`
- HEAD: `d99554e289c40df6a393fb8d5ad71132ff353b1d`
- production code: unchanged
- tests/fixtures/redactors: unchanged
- Claude/P0-06: out of scope and untouched

Evidence inputs and SHA-256:

| Artifact | SHA-256 |
| --- | --- |
| `P0_CODEX_DUPLICATE_EVIDENCE_CHECK.md` | `96a4710616c5f0373855c0dc13bc10bdc1d922a2b75538fd2ba07a150e1264e9` |
| `P0_CODEX_CONTROLLED_DUPLICATE_EVIDENCE.md` | `45383a89887e4602287eb90d849ce4dead159dce80b08542b8b603caef5b1f8a` |
| `codex-sanitized.jsonl` | `37713d63bcb8a3f7207e3414cf5d4d5b504e6b8e986e3d97d8160b1f4340c447` |
| `codex-controlled-1.jsonl` | `dbc25881a5028a4c7339b2bd559f0e91faf216d07ea2cce02a9ed02aace1f0a7` |

The audit used the current production `CodexJsonlStrategy`, `history_import::import_connection`, file-backed `Storage`, and SQLite aggregation. No theoretical parser-only conclusion was substituted for the end-to-end result.

# 2. Privacy constraints

Raw Codex rollout data was inspected only through an allowlisted structural extractor. It accessed/output only:

- top-level and payload event kind;
- timestamp and ordering/line ordinal;
- presence of `requestId`/`ordinal`;
- task lifecycle boundaries;
- numeric `last_token_usage` and `total_token_usage` counters;
- package-local hashes of structural tool/message IDs where correlation required them.

The extractor did not access or emit user/agent message text, tool payloads, paths, environment values, command arguments, file contents, prompt content, or assistant content. Matching source paths were not printed; sessions were referred to only as controlled/historical evidence.

# 3. Controlled lifecycle mapping

The controlled session contains three complete task lifecycles and three token observations:

| Task | Structural lifecycle | Agent messages | `token_count` observations |
| --- | --- | ---: | ---: |
| `task_001` | `task_started 09:56:40.146 → agent_message → token_count 09:56:43.993 → task_complete 09:56:44.010` | 1 | 1 |
| `task_002` | `task_started 09:56:47.336 → agent_message → token_count 09:56:49.614 → task_complete 09:56:49.648` | 1 | 1 |
| `task_003` | `task_started 09:56:53.404 → agent_message → token_count 09:56:58.676 → task_complete 09:56:58.691` | 1 | 1 |

Controlled counts:

```text
task_started  = 3
agent_message = 3
token_count   = 3
task_complete = 3
turn_aborted  = 0
```

For all three observations, the change in raw cumulative `total_token_usage` from the previous observation equals that event's `last_token_usage`. The final cumulative counters equal the sum of the three sanitized observations:

```text
input_tokens             43,355
cached_input_tokens      34,048
output_tokens               290
reasoning_output_tokens     234
total_tokens             43,645
```

This proves that this clean, tool-free controlled run has one usage observation per completed task and no close-time replay. It does not establish the same outer-task cardinality for agentic tasks containing internal model/tool loops.

# 4. Historical lifecycle mapping

The historical session has only three outer Codex task lifecycles but 33 token observations:

| Task | Start | End | End kind | `token_count` observations |
| --- | --- | --- | --- | ---: |
| `task_001` | `17:44:10.289` | `17:44:39.897` | `task_complete` | 2 |
| `task_002` | `17:47:58.205` | `17:53:13.658` | `task_complete` | 18 |
| `task_003` | `17:56:19.429` | `18:04:01.169` | `turn_aborted` | 13 |
| **Total** |  |  |  | **33** |

The multiple observations inside a task correlate with internal agent/model/tool activity. Therefore `user prompts == token_count` and `outer task_started == token_count` are both false for this corpus. Most of the 33 observations are legitimate internal model-usage steps, so deduplication based merely on outer task identity would be incorrect.

For observations 1–32, the raw cumulative counters advance exactly by the current event's `last_token_usage` counters. This provides a stronger semantic correlation than prompt count or timestamp adjacency:

```text
cumulative_after[n] - cumulative_after[n-1] == last_token_usage[n]
```

The invariant holds for 32 of 33 observations. The only exception is the terminal observation described next.

# 5. Ambiguous terminal pair

The former ambiguity is resolved as **Case B: replay telemetry inside one task lifecycle**.

Safe structural sequence:

```text
task_003
  ...
  reasoning
  custom_tool_call
  custom_tool_call_output
  token_count 18:03:49.041, ordinal 244
  item_completed
  reasoning
  custom_tool_call
  custom_tool_call_output
  token_count 18:04:01.134, ordinal 249
  message
  turn_aborted 18:04:01.169
```

Both token observations are inside `task_003`; no new `task_started` occurs between them. Structural agent activity does occur between them, so equality of `last_token_usage` alone would not have been sufficient to call the second event a duplicate.

The decisive field is the raw numeric `total_token_usage`, which is a cumulative session counter. At both observations it is exactly:

| Counter | Ordinal 244 | Ordinal 249 | Delta |
| --- | ---: | ---: | ---: |
| Input | 3,051,278 | 3,051,278 | **0** |
| Cached input | 2,781,952 | 2,781,952 | **0** |
| Output | 19,092 | 19,092 | **0** |
| Reasoning output | 6,620 | 6,620 | **0** |
| Total | 3,070,370 | 3,070,370 | **0** |

Both observations also repeat the same `last_token_usage` values and rate-limit snapshot. A real newly billable usage observation would advance at least the cumulative counters; ordinal 249 advances none of them. It is therefore a replay/stale re-emission of the preceding usage state, not an additional usage delta.

The prior controlled report's inference that the historical pair represented two legitimate executions merely because clean exit did not reproduce a duplicate is invalid. Clean exit is not the only condition capable of causing stale telemetry re-emission; the historical pair occurs immediately before `turn_aborted` after additional internal structural events.

# 6. task_count vs token_count

| Evidence | Started tasks | Completed tasks | Aborted tasks | Token observations | Cumulative advances |
| --- | ---: | ---: | ---: | ---: | ---: |
| Controlled | 3 | 3 | 0 | 3 | 3 |
| Historical | 3 | 2 | 1 | 33 | **32** |

Historical per-task distribution:

| Task | Token observations | Result |
| --- | ---: | --- |
| `task_001` | 2 | Both advance cumulative usage |
| `task_002` | 18 | All 18 advance cumulative usage |
| `task_003` | 13 | First 12 advance; terminal ordinal 249 has zero cumulative delta |

The relevant semantic count is not the number of manual prompts or outer tasks. It is the number of observations that advance Codex cumulative usage. In the historical evidence:

```text
raw token_count events       = 33
logical usage advances       = 32
zero-delta replay events     = 1
```

This exactly matches the earlier evidence-deduplicated expectation of 32 observations and totals of 3,070,370 overall, 19,092 output, and 6,620 reasoning tokens.

# 7. Production identity evaluation

The proposition under audit is disproved:

> `token_count` is not guaranteed to have a 1:1 relationship with a new billable/logical model usage delta when `requestId` is absent.

The historical raw format has no `requestId`. It does contain top-level ordinals in this session, including distinct ordinals 244 and 249 for the terminal pair. The controlled format has neither request ID nor ordinal on its token observations. Thus the current parser chooses:

- `{path_hash}:ordinal:{ordinal}` for the historical raw format;
- `{path_hash}:{record_offset}` where neither request ID nor ordinal is available;
- `{path_hash}:{record_offset}` for the supplied sanitized historical file because redaction did not preserve ordinal.

Neither ordinal nor byte offset is semantic usage identity: both distinguish the zero-delta replay from the preceding real usage observation.

The current production-equivalent sanitized pipeline was rerun:

```text
codex-sanitized.jsonl
→ production CodexJsonlStrategy
→ production import_connection
→ production file-backed SQLite Storage
→ production token aggregate
```

Observed result:

```text
first import:  records_accepted=33, malformed=0
second import: records_accepted=0
stored rows:   33
aggregate:     3,223,424 total tokens
```

The final rows have different offset identities (`...:13995` and `...:14530`), so both survive the storage uniqueness constraint. Aggregate comparison:

| Metric | Production actual | Cumulative evidence truth | Overcount |
| --- | ---: | ---: | ---: |
| Records/requests | 33 | 32 | 1 |
| Input | 3,204,210 | 3,051,278 | 152,932 |
| Cached input | 2,933,888 | 2,781,952 | 151,936 |
| Output | 19,214 | 19,092 | 122 |
| Reasoning output | 6,654 | 6,620 | 34 |
| Total | 3,223,424 | 3,070,370 | 153,054 |

Checkpointing prevents the unchanged file from being imported twice, but it does not prevent this intra-file replay from becoming a second `UsageRecord`.

# 8. P0-05 verdict

## **P0-05 FAIL**

Evidence is now sufficient; the issue is no longer blocked by ambiguity:

1. Controlled lifecycle evidence confirms normal one-observation behavior for simple tasks.
2. Historical lifecycle evidence confirms that agentic tasks legitimately contain multiple model/tool usage observations.
3. Raw cumulative counters distinguish those legitimate internal steps from the terminal replay.
4. The 33rd observation has zero cumulative delta across every token counter.
5. Production assigns it a distinct ordinal/offset identity, stores it, and sums it.

This is **intra-file double counting**. No heuristic is needed to reach the verdict, and no dedup heuristic or production remediation was implemented during this audit.

# 9. Final P0 gate consequence

P0-05 is not closed. The previous P0-05 PASS and any overall P0 Gate A conclusion depending on it must be withdrawn.

A future remediation must use the newly proven cumulative-counter semantics and file/session scope while preserving legitimate multiple internal usage advances within one outer task. It must not collapse records merely because their `last_token_usage`, timestamp proximity, task identity, or rate-limit snapshot happens to match. Required regression evidence is the historical terminal sequence in which the cumulative delta is zero, plus a multi-step agentic task in which consecutive observations advance cumulative counters and must remain distinct.

Production code, tests, fixtures, redactors, Claude, and provider architecture remain unchanged by this audit.

# **P0-05 FAIL**
