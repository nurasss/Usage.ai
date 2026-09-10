# P0 Remediation Round 2 Report

**Date:** 2026-09-09
**Baseline Revision:** `main / d99554e289c40df6a393fb8d5ad71132ff353b1d`
**Working Tree:** Dirty
**Pre-remediation Verdict:** `D — DATA INTEGRITY / SECURITY RISK` (from `TECHNICAL_AUDIT_P0_REAUDIT.md`)
**Status:** Remediated code-level P0 defects; External evidence remains `BLOCKED_BY_EXTERNAL_EVIDENCE`.
**Final Verdict Assessment:** Gate B passed; Gate A conditionally blocked pending real sanitized user corpora for Codex/Claude semantics.

---

## 1. Executive Summary

This report documents the remediation of defects identified in `TECHNICAL_AUDIT_P0_REAUDIT.md`.
No new providers were added. Antigravity, Z.ai, OpenCode, ChatGPT consumer, claude.ai, Gemini API, and Codex App Server remain strictly untouched.
No fake external evidence was fabricated. Codex (P0-05) and Claude (P0-06) evidence remains explicitly flagged as `BLOCKED_BY_EXTERNAL_EVIDENCE`.

All workspace tests, typechecks, linters, and frontend tests pass cleanly:
- **Rust workspace tests:** 163 passed, 0 failed, 1 ignored (real-corpus live probe)
- **Rust Clippy:** 0 warnings (`-D warnings` enforced across all targets)
- **Frontend tests:** 19 passed (5 test files)
- **Svelte / TypeScript check:** 0 errors, 0 warnings
- **Production Vite build:** succeeded

---

## 2. Remediation Details by P0 Issue

### P0-01: Migration 006 Data Integrity & Storage Permissions
- **Problem:** Migration 006 unconditionally updated accounts without checking if they were legacy or modern API accounts, overwriting valid OpenAI API credentials with `Unknown` identity confidence and invalid connection strings. Filesystem permissions on `usage.db` and sidecar files were not enforced.
- **Fix:**
  - Modified `crates/usage-storage/src/lib.rs` (`migrate_legacy_rows`) to only target unlinked rows (`connection_id IS NULL OR account_id = '00000000-0000-0000-0000-000000000000' OR account_id NOT IN (SELECT id FROM accounts)`).
  - Hardened database creation: parent directory set to `0o700`, database file set to `0o600`, and SQLite WAL/SHM sidecars set to `0o600`.
  - Added regression test cases A (legacy contamination), B (trustworthy OpenAI API retention), C (mixed DB), D (migration idempotence), and exact SQLite decimal roundtrip test.

### P0-02: Production HTTP 2 MiB Bounds & Streaming
- **Problem:** `ReqwestHttpHost` defaulted to a 1024-byte clamp (`max_bytes_default`), and buffered whole responses in memory via `.bytes()` without bounded streaming or select-based cancellation during network reads.
- **Fix:**
  - Updated `crates/usage-host/src/http.rs` to set `max_bytes_default` to `MAX_HTTP_BYTES` (2 MiB).
  - Implemented streaming body retrieval via `response.chunk()` within `tokio::select!` racing against cancellation tokens and deadlines.
  - Implemented cumulative byte size checks during chunk streaming to reject payloads exceeding limits before buffering them in memory.
  - Added loopback tests for 4 KiB payloads, limit enforcement, chunked streaming overflow, mid-stream cancellation, and timeouts.

### P0-03: OpenAI Exact Money Arithmetic
- **Problem:** Float conversions (`as_f64`, `from_f64`) in JSON parsing led to IEEE 754 precision loss for financial accounting.
- **Fix:**
  - Activated `serde_json/arbitrary_precision` and `rust_decimal/serde-with-arbitrary-precision`.
  - Rewrote decimal parsing in `crates/usage-providers/src/openai_api.rs` to extract exact string representations from `serde_json::Number` and parse directly into `rust_decimal::Decimal`.
  - Added tests verifying exact precision (e.g. `0.1234567890123456789` and `0.1 + 0.2 == 0.3`).

### P0-04: Cancellation Race Conditions & Refresh Lifecycle
- **Problem:** Refresh requests cancelled before semaphore acquisition still executed; disabled accounts executed in the background; cancelling an account before its first refresh did not register a tombstone.
- **Fix:**
  - `crates/usage-runtime/src/refresh.rs`: `cancel_account` installs a pre-cancelled token tombstone in `cancellations` map.
  - `refresh_scope` checks `cancel.is_cancelled()` immediately after semaphore permit acquisition.
  - `execute` checks whether the account is disabled or archived before initiating fetches and releases the storage lock before calling `stale_outcome` to prevent deadlocks.
  - Added tests for pre-refresh cancellation, post-semaphore admission cancellation, and storage-disabled account suppression.

### P0-05 & P0-06: External Evidence & Diagnostic Redaction
- **Problem:** Audit revealed that synthetic fixtures cannot stand in for real user corpora, and the audit falsely claimed full verification.
- **Fix:**
  - In `crates/usage-providers/src/descriptor.rs`, narrowed Codex glob from `*sessions/**/rollout-*.jsonl` to `sessions/**/rollout-*.jsonl;archived_sessions/**/rollout-*.jsonl` to eliminate sibling directory over-scanning.
  - Marked Claude coverage as `Coverage::UnverifiedSemantics`.
  - Created standalone Node.js redaction scripts `scripts/redact-codex.mjs` and `scripts/redact-claude.mjs` with allowlist-only field retention and deterministic SHA-256 opaque IDs.
  - Added redaction unit tests in `src/lib/redact.test.ts`.
  - Maintained status as `BLOCKED_BY_EXTERNAL_EVIDENCE` in documentation.

### P0-07: Stale-While-Revalidate (SWR) Integration & Component Testing
- **Problem:** SWR behavior (serving stale LKG while displaying stale banner and current error) lacked frontend component-level verification.
- **Fix:**
  - Created real Svelte 5 component test in `src/App.test.ts` using `@testing-library/svelte` asserting simultaneous rendering of LKG percentage, stale banner ("Данные устарели"), and network error banner ("⚠ Ошибка сети").
  - Configured `vite.config.ts` test environment with `resolve.conditions: ['browser']` for Svelte 5 component testing.
  - Added runtime integration test in `src-tauri/src/refresh_flow.rs` exercising the complete path from refresh invocation, HTTP retrieval, decimal parsing, SQLite storage, and DTO conversion.

---

## 3. Test Verification Matrix

| Area / Crate | Tests | Result | Notes |
|---|---|---|---|
| `usage_storage` | 24 | PASS | 006 migration cases A/B/C/D, 0600 file perms, exact Decimal roundtrip |
| `usage_host` | 39 | PASS | 2 MiB HTTP bounds, chunk streaming, cancellation, exact dir glob |
| `usage_providers` | 37 | PASS (1 ignored) | Decimal money, pagination, unverified semantics |
| `usage_runtime` | 45 | PASS | Pre-admission cancellation, tombstone, SWR state management |
| `usage_core` | 16 | PASS | Aggregations, IDs, notifications, models |
| `usage-ai-app` (Tauri) | 2 | PASS | SWR DTO preservation, E2E refresh-to-storage-to-dto |
| Frontend (Vitest) | 19 | PASS | Real Svelte 5 component test, format, snapshot, redact |
| TypeScript check | - | PASS | 0 errors, 0 warnings (`svelte-check`) |
| Rust Clippy | - | PASS | Clean across workspace with `-D warnings` |
| Vite Build | - | PASS | Production bundle generated |
