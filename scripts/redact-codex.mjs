#!/usr/bin/env node
// @ts-nocheck
/**
 * scripts/redact-codex.mjs
 * Strictly value-level allowlisted redaction of Codex JSONL session files.
 *
 * Enforces value-level allowlists for event types, plans, models, and numeric counters.
 * Generates package-local, non-linkable opaque IDs.
 * Schema output strictly matches production parser:
 * payload.type = "token_count"
 * payload.info.last_token_usage = { ... }
 */
import { createReadStream, createWriteStream, statSync, renameSync, unlinkSync, existsSync, writeFileSync } from 'node:fs';
import * as readline from 'node:readline';
import { once } from 'node:events';

const ALLOWED_EVENT_TYPES = new Set(['event_msg', 'event']);
const ALLOWED_PAYLOAD_TYPES = new Set(['token_count']);
const ALLOWED_PLANS = new Set(['free', 'plus', 'pro', 'team', 'business', 'enterprise']);
const ALLOWED_LIMIT_NAMES = new Set(['primary', 'secondary', 'default', 'codex']);
const CODEX_MODEL_REGEX = /^(gpt-[a-zA-Z0-9.-]+|o[1-9][a-zA-Z0-9.-]*|chatgpt-[a-zA-Z0-9.-]+|codex-[a-zA-Z0-9.-]+|text-embedding-[a-zA-Z0-9.-]+)$/i;
const SENSITIVE_WORDS = ['secret', 'key', 'token', 'bearer', 'auth', 'project', 'api', 'password', 'alice', 'private'];

export const MAX_INPUT_BYTES = 50 * 1024 * 1024; // 50 MiB
export const MAX_OUTPUT_BYTES = 50 * 1024 * 1024; // 50 MiB
export const MAX_DISTINCT_IDS = 50_000;

export function createRedactorContext(options = {}) {
  const maxDistinctIds = options?.maxDistinctIds ?? MAX_DISTINCT_IDS;
  const idMap = new Map();
  const counters = new Map();

  return {
    mapId(raw, prefix = 'id') {
      if (raw === null || raw === undefined) return undefined;
      if (typeof raw !== 'string' && typeof raw !== 'number') return undefined;
      const str = String(raw).trim();
      if (!str) return undefined;
      const key = `${prefix}:${str}`;
      let mapped = idMap.get(key);
      if (!mapped) {
        if (idMap.size >= maxDistinctIds) {
          throw new Error(`Exceeded maximum distinct identifiers (${maxDistinctIds})`);
        }
        const count = (counters.get(prefix) ?? 0) + 1;
        counters.set(prefix, count);
        mapped = `${prefix}_${String(count).padStart(4, '0')}`;
        idMap.set(key, mapped);
      }
      return mapped;
    },
    get size() {
      return idMap.size;
    }
  };
}

export function parseSafeInteger(val) {
  if (val === undefined) return undefined;
  if (val === null) return null;
  if (typeof val === 'number') {
    if (!Number.isFinite(val) || !Number.isInteger(val)) return null;
    if (val < 0 || val > Number.MAX_SAFE_INTEGER) return null;
    return val;
  }
  if (typeof val === 'string') {
    const trimmed = val.trim();
    if (!/^\d+$/.test(trimmed)) return null;
    try {
      const b = BigInt(trimmed);
      if (b < 0n || b > BigInt(Number.MAX_SAFE_INTEGER)) return null;
      return Number(b);
    } catch {
      return null;
    }
  }
  return null;
}

export function parseSafePercent(val) {
  if (val === undefined) return undefined;
  if (val === null) return null;
  let num;
  if (typeof val === 'number') {
    num = val;
  } else if (typeof val === 'string') {
    const trimmed = val.trim();
    if (!/^\d+(\.\d+)?$/.test(trimmed)) return null;
    num = Number(trimmed);
  } else {
    return null;
  }
  if (!Number.isFinite(num) || num < 0 || num > 100) return null;
  return num;
}

export function parseSafeModel(model) {
  if (typeof model !== 'string') return null;
  const trimmed = model.trim();
  if (trimmed.length < 2 || trimmed.length > 64) return null;
  if (!CODEX_MODEL_REGEX.test(trimmed)) return null;
  if (trimmed.includes('..') || trimmed.includes('/') || trimmed.includes('\\') || trimmed.includes('@') || trimmed.includes('sk-')) {
    return null;
  }
  const lower = trimmed.toLowerCase();
  for (const word of SENSITIVE_WORDS) {
    if (lower.includes(word)) return null;
  }
  return trimmed;
}

export function parseSafePlan(plan) {
  if (typeof plan !== 'string') return 'unknown';
  const lower = plan.trim().toLowerCase();
  if (ALLOWED_PLANS.has(lower)) return lower;
  return 'unknown';
}

export function parseSafeTimestamp(raw) {
  if (raw === undefined) return undefined;
  if (raw === null) return 'INVALID';
  if (typeof raw !== 'string' && typeof raw !== 'number') return 'INVALID';
  const d = new Date(raw);
  if (isNaN(d.getTime())) return 'INVALID';
  return d.toISOString();
}

function redactTokenUsage(usage) {
  if (!usage || typeof usage !== 'object') return null;

  const rawInput = 'input_tokens' in usage ? usage.input_tokens : usage.inputTokens;
  const rawCached = 'cached_input_tokens' in usage ? usage.cached_input_tokens : usage.cachedInputTokens;
  const rawOutput = 'output_tokens' in usage ? usage.output_tokens : usage.outputTokens;
  const rawReasoning = 'reasoning_output_tokens' in usage ? usage.reasoning_output_tokens : usage.reasoningOutputTokens;
  const rawTotal = 'total_tokens' in usage ? usage.total_tokens : usage.totalTokens;

  const hasInput = rawInput !== undefined;
  const hasCached = rawCached !== undefined;
  const hasOutput = rawOutput !== undefined;
  const hasReasoning = rawReasoning !== undefined;
  const hasTotal = rawTotal !== undefined;

  const input_tokens = parseSafeInteger(rawInput);
  const cached_input_tokens = parseSafeInteger(rawCached);
  const output_tokens = parseSafeInteger(rawOutput);
  const reasoning_output_tokens = parseSafeInteger(rawReasoning);
  const total_tokens = parseSafeInteger(rawTotal);

  if (
    (hasInput && input_tokens === null) ||
    (hasCached && cached_input_tokens === null) ||
    (hasOutput && output_tokens === null) ||
    (hasReasoning && reasoning_output_tokens === null) ||
    (hasTotal && total_tokens === null)
  ) {
    return null;
  }

  const result = {};
  if (input_tokens !== undefined && input_tokens !== null) result.input_tokens = input_tokens;
  if (cached_input_tokens !== undefined && cached_input_tokens !== null) result.cached_input_tokens = cached_input_tokens;
  if (output_tokens !== undefined && output_tokens !== null) result.output_tokens = output_tokens;
  if (reasoning_output_tokens !== undefined && reasoning_output_tokens !== null) result.reasoning_output_tokens = reasoning_output_tokens;
  if (total_tokens !== undefined && total_tokens !== null) result.total_tokens = total_tokens;

  return Object.keys(result).length > 0 ? result : null;
}

function redactRateLimits(limits, ctx) {
  if (!limits || typeof limits !== 'object') return null;

  const rawLimitId = 'limit_id' in limits ? limits.limit_id : limits.limitId;
  const limit_id = ctx.mapId(rawLimitId, 'lim');
  const rawPlan = 'plan_type' in limits ? limits.plan_type : limits.planType;
  const plan_type = parseSafePlan(rawPlan);

  let primary = null;
  const rawPrimary = limits.primary;
  if (rawPrimary && typeof rawPrimary === 'object') {
    const rawPct = 'used_percent' in rawPrimary ? rawPrimary.used_percent : ('usedPercent' in rawPrimary ? rawPrimary.usedPercent : limits.primary_used_percent);
    const rawMin = 'window_minutes' in rawPrimary ? rawPrimary.window_minutes : ('windowMinutes' in rawPrimary ? rawPrimary.windowMinutes : limits.window_minutes);
    const rawReset = 'resets_at' in rawPrimary ? rawPrimary.resets_at : ('resetsAt' in rawPrimary ? rawPrimary.resetsAt : limits.resets_at);
    const used_percent = parseSafePercent(rawPct);
    const window_minutes = parseSafeInteger(rawMin);
    const resets_at = parseSafeInteger(rawReset);
    if (used_percent === null || window_minutes === null || resets_at === null) return null;
    primary = {
      used_percent: used_percent ?? 0,
      window_minutes: window_minutes ?? 0,
      resets_at: resets_at ?? 0,
    };
  } else if (limits.primary_used_percent !== undefined) {
    const used_percent = parseSafePercent(limits.primary_used_percent);
    const window_minutes = parseSafeInteger(limits.window_minutes);
    const resets_at = parseSafeInteger(limits.resets_at);
    if (used_percent === null || window_minutes === null || resets_at === null) return null;
    primary = {
      used_percent: used_percent ?? 0,
      window_minutes: window_minutes ?? 0,
      resets_at: resets_at ?? 0,
    };
  }

  let secondary = null;
  const rawSecondary = limits.secondary;
  if (rawSecondary && typeof rawSecondary === 'object') {
    const rawPct = 'used_percent' in rawSecondary ? rawSecondary.used_percent : ('usedPercent' in rawSecondary ? rawSecondary.usedPercent : limits.secondary_used_percent);
    const rawMin = 'window_minutes' in rawSecondary ? rawSecondary.window_minutes : rawSecondary.windowMinutes;
    const rawReset = 'resets_at' in rawSecondary ? rawSecondary.resets_at : rawSecondary.resetsAt;
    const used_percent = parseSafePercent(rawPct);
    const window_minutes = parseSafeInteger(rawMin);
    const resets_at = parseSafeInteger(rawReset);
    if (used_percent === null || window_minutes === null || resets_at === null) return null;
    secondary = {
      used_percent: used_percent ?? 0,
      window_minutes: window_minutes ?? 0,
      resets_at: resets_at ?? 0,
    };
  }

  const res = {
    limit_id,
    plan_type,
  };
  if (limits.limit_name && typeof limits.limit_name === 'string') {
    const rawLimitName = limits.limit_name.trim().toLowerCase();
    if (ALLOWED_LIMIT_NAMES.has(rawLimitName)) {
      res.limit_name = rawLimitName;
    }
  }
  if (primary) res.primary = primary;
  if (secondary) res.secondary = secondary;
  return res;
}

const defaultContext = createRedactorContext();

export function redactCodexLine(line, ctx = defaultContext) {
  const trimmed = line.trim();
  if (!trimmed || trimmed.startsWith('#')) return null;

  let parsed;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return null;
  }

  // Value-level event kind check
  const rawType = parsed.type;
  if (rawType !== undefined && !ALLOWED_EVENT_TYPES.has(String(rawType))) {
    return null;
  }

  const payload = parsed.payload;
  const rawPayloadType = payload?.type ?? parsed.type;
  if (rawPayloadType !== undefined && !ALLOWED_PAYLOAD_TYPES.has(String(rawPayloadType))) {
    return null;
  }

  // Timestamp validation
  let tsResult = undefined;
  if ('timestamp' in parsed) {
    tsResult = parseSafeTimestamp(parsed.timestamp);
    if (tsResult === 'INVALID') {
      return null; // safe drop without crash
    }
  }

  // Request ID
  const rawRequestId = 'request_id' in parsed ? parsed.request_id : parsed.requestId;
  const requestId = ctx.mapId(rawRequestId, 'req');

  // Token usage extraction
  const rawUsage = payload?.info?.last_token_usage ?? payload?.last_token_usage ?? parsed.last_token_usage;
  let tokenUsage = null;
  if (rawUsage) {
    tokenUsage = redactTokenUsage(rawUsage);
    if (!tokenUsage) return null; // malformed counters -> fail closed
  }

  // Rate limits extraction
  const rawLimits = payload?.rate_limits ?? parsed.rate_limits;
  let rateLimits = null;
  if (rawLimits) {
    rateLimits = redactRateLimits(rawLimits, ctx);
    if (!rateLimits) return null;
  }

  // Model validation
  const rawModel = payload?.model ?? payload?.info?.model ?? parsed.model;
  const model = parseSafeModel(rawModel);

  if (!tokenUsage && !rateLimits) {
    return null;
  }

  const output = {
    type: 'event_msg',
    payload: {
      type: 'token_count',
    },
  };

  if (tsResult !== undefined) {
    output.timestamp = tsResult;
  }
  if (requestId) {
    output.requestId = requestId;
  }
  if (model) {
    output.payload.model = model;
  }
  if (tokenUsage) {
    output.payload.info = {
      last_token_usage: tokenUsage,
    };
    if (model) {
      output.payload.info.model = model;
    }
  }
  if (rateLimits) {
    output.payload.rate_limits = rateLimits;
  }

  return JSON.stringify(output);
}

export function generateRedactorMetadata(accepted, rejected) {
  return {
    schema_version: '1.0.0',
    redactor_version: '1.0.0',
    source_kind: 'codex',
    generated_at: new Date().toISOString(),
    record_count: accepted,
    rejected_count: rejected,
  };
}

export async function redactCodexFileStreaming(inputFile, outputFile, metadataFile, options = {}) {
  const maxInputBytes = options.maxInputBytes ?? MAX_INPUT_BYTES;
  const maxOutputBytes = options.maxOutputBytes ?? MAX_OUTPUT_BYTES;
  const maxDistinctIds = options.maxDistinctIds ?? MAX_DISTINCT_IDS;

  const stat = statSync(inputFile);
  if (stat.size > maxInputBytes) {
    throw new Error(`Input file size (${stat.size} bytes) exceeds maximum allowed (${maxInputBytes} bytes). Please choose a smaller session file.`);
  }

  const tempOutputFile = `${outputFile}.tmp.${process.pid}.${Date.now()}`;
  const inputStream = createReadStream(inputFile, { encoding: 'utf8' });
  const rl = readline.createInterface({ input: inputStream, crlfDelay: Infinity });
  const outputStream = createWriteStream(tempOutputFile, { encoding: 'utf8' });

  const ctx = createRedactorContext({ maxDistinctIds });
  let accepted = 0;
  let rejected = 0;
  let bytesWritten = 0;

  try {
    for await (const line of rl) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith('#')) continue;
      const redacted = redactCodexLine(trimmed, ctx);
      if (redacted) {
        const lineWithNewline = redacted + '\n';
        const lineBytes = Buffer.byteLength(lineWithNewline, 'utf8');
        bytesWritten += lineBytes;
        if (bytesWritten > maxOutputBytes) {
          throw new Error(`Output file exceeded maximum allowed size (${maxOutputBytes} bytes).`);
        }
        if (!outputStream.write(lineWithNewline)) {
          await once(outputStream, 'drain');
        }
        accepted += 1;
      } else {
        rejected += 1;
      }
    }

    outputStream.end();
    await once(outputStream, 'finish');

    // Atomic rename to final output
    renameSync(tempOutputFile, outputFile);

    const meta = generateRedactorMetadata(accepted, rejected);
    if (metadataFile) {
      writeFileSync(metadataFile, JSON.stringify(meta, null, 2) + '\n', 'utf8');
    }
    return { accepted, rejected, total: accepted + rejected, metadata: meta };
  } catch (err) {
    outputStream.destroy();
    if (existsSync(tempOutputFile)) {
      try { unlinkSync(tempOutputFile); } catch {}
    }
    throw err;
  }
}

async function runCli() {
  const [, , inputFile, outputFile, metadataFile] = process.argv;
  if (!inputFile || !outputFile) {
    console.error('Usage: node scripts/redact-codex.mjs <input.jsonl> <output.jsonl> [metadata.json]');
    process.exit(1);
  }
  try {
    const res = await redactCodexFileStreaming(inputFile, outputFile, metadataFile);
    console.log(`Processed: ${res.total}, accepted: ${res.accepted}, rejected: ${res.rejected}`);
  } catch (err) {
    console.error(`Redaction failed: ${err.message}`);
    process.exit(1);
  }
}

if (process.argv[1] && process.argv[1].endsWith('redact-codex.mjs')) {
  runCli();
}
