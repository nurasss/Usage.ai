# P0-05 Codex Duplicate Evidence Check

## 1. Scope and constraints

This is a narrow independent check of the remaining P0-05 question only.

Inputs:

- `P0_EXTERNAL_EVIDENCE_VALIDATION.md`
- `codex-sanitized.jsonl`
- the current production Codex parser, runtime history importer, SQLite storage, and storage aggregation APIs

Repository baseline:

- branch: `main`
- HEAD: `d99554e289c40df6a393fb8d5ad71132ff353b1d`
- worktree: dirty before this check; existing user changes were preserved
- `P0_EXTERNAL_EVIDENCE_VALIDATION.md` SHA-256: `70529d50f7640b15a870a4632f038058e79b52a89434ddad8cc6ccc6d036e75f`
- `codex-sanitized.jsonl` SHA-256: `37713d63bcb8a3f7207e3414cf5d4d5b504e6b8e986e3d97d8160b1f4340c447`

No Claude evidence or code was examined for a new semantic conclusion. Redactors and providers were not changed. No production code, test, or fixture was changed before or after the execution check.

## 2. Evidence contradiction

The evidence file contains 33 accepted `token_count` lines. Its final two lines have:

- equal `input_tokens = 152932`;
- equal `cached_input_tokens = 151936`;
- equal `output_tokens = 122`;
- equal `reasoning_output_tokens = 34`;
- equal `total_tokens = 153054`;
- equal primary and secondary rate-limit snapshots;
- different timestamps: `2026-08-31T18:03:49.041Z` and `2026-08-31T18:04:01.134Z`.

An independent projection comparison excluding timestamp returned `true` for equality. Neither line contains `requestId`, `request_id`, or `ordinal`.

`P0_EXTERNAL_EVIDENCE_VALIDATION.md` calls the pair a duplicate session-close flush and gives 32 logical observations as the expected result, but also states that the production offset fallback imports adjacent identical events separately. It nevertheless marks P0-05 PASS. That PASS is not supported by an end-to-end storage result.

## 3. Production-equivalent execution

A temporary Rust harness outside the repository performed this exact path:

```text
codex-sanitized.jsonl
→ usage_providers::codex::CodexJsonlStrategy
→ usage_runtime::history_import::import_connection
→ usage_storage::Storage::commit_import_batch
→ SQLite usage_records
→ Storage::token_totals_between / unverified_token_totals_between
```

The harness used the repository crates as path dependencies. It copied the unchanged evidence file into a temporary scoped Codex home under the production glob `sessions/**/rollout-*.jsonl`, created a normal active local account and source connection, opened a real file-backed production `Storage`, and invoked the public runtime importer. It then queried both the public storage aggregate and the resulting SQLite table. No test-only parser, importer, storage implementation, reconstructed JSON, or manually inserted `UsageRecord` was used.

Observed first-import result:

```text
files_discovered=1
files_imported=1
records_accepted=33
malformed=0
```

Observed second import of the unchanged file:

```text
files_discovered=1
files_imported=1
records_accepted=0
malformed=0
```

The second result confirms that the checkpoint prevents cross-import inflation. It does not remove duplicates already accepted within the first file pass.

## 4. SQLite result

After the first production import, SQLite contained 33 Codex `usage_records`:

| Metric | Actual production result | Raw-offset expected | Evidence-deduplicated expected |
| --- | ---: | ---: | ---: |
| Records / requests | **33** | 33 | 32 |
| Input tokens | **3,204,210** | 3,204,210 | 3,051,278 |
| Cached input tokens | **2,933,888** | 2,933,888 | 2,781,952 |
| Output tokens | **19,214** | 19,214 | 19,092 |
| Reasoning output tokens | **6,654** | 6,654 | 6,620 |
| Total tokens | **3,223,424** | 3,223,424 | 3,070,370 |

The public production aggregate returned:

```text
Storage::token_totals_between = [("codex", 3223424)]
Storage::unverified_token_totals_between = [("codex", 3223424)]
```

Thus both final observations are included in the stored/unverified aggregate. The authoritative overview currently excludes all `UnverifiedSemantics` records by design; that presentation isolation does not deduplicate the underlying Codex history and does not make P0-05 correct.

The last two stored rows were independently read back as:

```text
...:13995  2026-08-31T18:03:49.041+00:00  in=152932 cached=151936 out=122 reasoning=34 total=153054
...:14530  2026-08-31T18:04:01.134+00:00  in=152932 cached=151936 out=122 reasoning=34 total=153054
```

The excess over the evidence-deduplicated expectation is exactly one copy of line 33:

```text
records   +1
input     +152932
cached    +151936
output    +122
reasoning +34
total     +153054
```

## 5. Exact production mechanism

`crates/usage-providers/src/codex.rs:166-175` selects identity in this order:

1. source `request_id`;
2. source `ordinal`;
3. fallback `{path_hash}:{record_offset}`.

The real sanitized lines provide neither of the first two identities, so the byte offsets `13995` and `14530` produce distinct `source_record_id` values.

`crates/usage-runtime/src/history_import.rs:121-142` forwards every parsed record from the chunk to one `commit_import_batch`; there is no intra-chunk semantic reducer.

`crates/usage-storage/src/lib.rs:12` upserts on the storage uniqueness identity that includes `source_record_id`. Because the two offset IDs differ, there is no conflict. `commit_import_batch` executes the upsert once for each of the 33 records and reports all 33 accepted. `token_totals_between` and `unverified_token_totals_between` then sum both rows.

Therefore the answer to the main question is:

> **Yes. The current production import stores both lines as independent UsageRecords and both enter aggregate totals. There is no current mechanism that collapses this pair.**

## 6. Why no remediation was applied

Against the evidence report's declared 32-observation expectation, the current implementation exhibits **intra-file double counting**. However, this one sanitized file is not sufficient to derive the collision semantics required for a safe fix.

The available candidate fingerprint would consist only of:

- file/session scope;
- equal token counters;
- equal rate-limit snapshot;
- adjacency;
- timestamps 12.093 seconds apart;
- end-of-file position.

Timestamp cannot be semantic identity, as required. Removing timestamp leaves no request/turn identifier. Two genuinely distinct requests can in principle have the same input, cache, output, reasoning, total, and unchanged rounded rate-limit snapshot. A global or file-local content hash would then collapse a real turn. Adding adjacency or a time window lowers collision probability but does not establish semantic identity. The current evidence contains no safe lifecycle marker or external observed turn count that proves the second emission was a flush rather than a distinct request.

The statement in the prior report that lines 32/33 “prove” a session-close duplicate is an inference from equality, adjacency, and EOF. The sanitized artifact itself contains no session-close event and cannot independently prove that inference. Implementing dedup from this single case would violate the instruction not to guess and not to collapse real turns merely because token counts happen to match.

Accordingly, no production change, regression fixture, or new dedup fingerprint was added in this check.

## 7. Minimum additional Codex evidence

Collect one controlled, independently annotated Codex session-close case, ideally two repetitions for confidence, with all of the following:

1. A fresh isolated session file and its complete sanitized `token_count` sequence.
2. A manual sidecar manifest containing only safe facts: exact number of prompts submitted, exact number of completed model responses observed, UTC time at which the final response completed, and UTC time at which the session/client was closed. No prompt or response content is needed.
3. Confirmation that no second prompt, retry, resume, or background model request occurred between final response completion and close.
4. Safe sanitized lifecycle/event-kind markers immediately surrounding the final `token_count` emissions, if the raw log contains such markers. Only enum kind, ordering, and relative/UTC timestamp are needed; payload text must remain absent.
5. The byte-order result showing whether one completed response produces two identical terminal `token_count` events specifically during close/flush.

The decisive case is:

```text
manual completed responses = N
sanitized token_count observations = N + 1
the extra adjacent identical observation is bracketed by a proven close/flush lifecycle event
```

If that behavior reproduces, a safe implementation can be designed around a file/session-scoped close-flush state transition rather than an unconditional content hash. If lifecycle markers are unavailable, at minimum provide two controlled sessions with manually verified turn counts and repeated EOF behavior; the remaining collision risk must then be stated explicitly before choosing an adjacency policy.

## 8. Verdict

The prior `P0-05 PASS` must be withdrawn. Production-equivalent execution proves raw-offset behavior and aggregate inflation relative to the report's 32-observation expectation, but the current single evidence file does not establish a collision-safe semantic identity for remediation.

# **P0-05 BLOCKED — more evidence required**
