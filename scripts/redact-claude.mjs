#!/usr/bin/env node
// @ts-nocheck
/**
 * scripts/redact-claude.mjs
 * Strictly value-level allowlisted redaction of Claude Code session JSONL files.
 *
 * Enforces value-level allowlists for event types, models, and numeric counters.
 * Generates package-local, non-linkable opaque IDs for uuid, requestId, and message.id.
 * Schema output strictly matches production parser:
 * envelope: timestamp, uuid, requestId, message: { id, model, usage: { ... } }
 */
import { createReadStream, createWriteStream, statSync, renameSync, unlinkSync, existsSync, writeFileSync } from 'node:fs';
import * as readline from 'node:readline';
import { once } from 'node:events';

const ALLOWED_EVENT_TYPES = new Set(['message', 'assistant']);
const CLAUDE_MODEL_REGEX = /^claude-[a-zA-Z0-9.-]{1,60}$/i;
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

export function parseSafeModel(model) {
  if (typeof model !== 'string') return null;
  const trimmed = model.trim();
  if (trimmed.length < 2 || trimmed.length > 64) return null;
  if (!CLAUDE_MODEL_REGEX.test(trimmed)) return null;
  if (trimmed.includes('..') || trimmed.includes('/') || trimmed.includes('\\') || trimmed.includes('@') || trimmed.includes('sk-')) {
    return null;
  }
  const lower = trimmed.toLowerCase();
  for (const word of SENSITIVE_WORDS) {
    if (lower.includes(word)) return null;
  }
  return trimmed;
}

export function parseSafeTimestamp(raw) {
  if (raw === undefined) return undefined;
  if (raw === null) return 'INVALID';
  if (typeof raw !== 'string' && typeof raw !== 'number') return 'INVALID';
  const d = new Date(raw);
  if (isNaN(d.getTime())) return 'INVALID';
  return d.toISOString();
}

const defaultContext = createRedactorContext();

export function redactClaudeLine(line, ctx = defaultContext) {
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

  // Value-level event kind
  const rawType = parsed.type;
  if (rawType !== undefined && !ALLOWED_EVENT_TYPES.has(String(rawType))) {
    return null;
  }

  const msg = parsed.message ?? (parsed.type === 'message' ? parsed : null);
  const usage = parsed.usage ?? msg?.usage;
  if (!usage || typeof usage !== 'object') {
    return null;
  }

  const rawInput = 'input_tokens' in usage ? usage.input_tokens : usage.inputTokens;
  const rawOutput = 'output_tokens' in usage ? usage.output_tokens : usage.outputTokens;
  const rawCreation = 'cache_creation_input_tokens' in usage ? usage.cache_creation_input_tokens : usage.cacheCreationTokens;
  const rawRead = 'cache_read_input_tokens' in usage ? usage.cache_read_input_tokens : usage.cacheReadTokens;

  const hasInput = rawInput !== undefined;
  const hasOutput = rawOutput !== undefined;
  const hasCreation = rawCreation !== undefined;
  const hasRead = rawRead !== undefined;

  // Safe counters
  const input_tokens = parseSafeInteger(rawInput);
  const output_tokens = parseSafeInteger(rawOutput);
  const cache_creation_input_tokens = parseSafeInteger(rawCreation);
  const cache_read_input_tokens = parseSafeInteger(rawRead);

  if (
    (hasInput && input_tokens === null) ||
    (hasOutput && output_tokens === null) ||
    (hasCreation && cache_creation_input_tokens === null) ||
    (hasRead && cache_read_input_tokens === null)
  ) {
    return null;
  }

  if (input_tokens === undefined && output_tokens === undefined) {
    return null;
  }

  // Timestamp
  let tsResult = undefined;
  if ('timestamp' in parsed || 'at' in parsed) {
    const rawTs = 'timestamp' in parsed ? parsed.timestamp : parsed.at;
    tsResult = parseSafeTimestamp(rawTs);
    if (tsResult === 'INVALID') {
      return null;
    }
  }

  // IDs
  const rawUuid = 'uuid' in parsed ? parsed.uuid : parsed.id;
  const rawReq = 'requestId' in parsed ? parsed.requestId : parsed.request_id;
  const rawMsg = msg && 'id' in msg ? msg.id : parsed.message_id;

  const uuid = ctx.mapId(rawUuid, 'uuid');
  const requestId = ctx.mapId(rawReq, 'req');
  const messageId = ctx.mapId(rawMsg, 'msg');

  // Model
  const rawModel = msg && 'model' in msg ? msg.model : parsed.model;
  const model = parseSafeModel(rawModel);

  const usageObj = {};
  if (input_tokens !== undefined && input_tokens !== null) usageObj.input_tokens = input_tokens;
  if (output_tokens !== undefined && output_tokens !== null) usageObj.output_tokens = output_tokens;
  if (cache_creation_input_tokens !== undefined && cache_creation_input_tokens !== null) usageObj.cache_creation_input_tokens = cache_creation_input_tokens;
  if (cache_read_input_tokens !== undefined && cache_read_input_tokens !== null) usageObj.cache_read_input_tokens = cache_read_input_tokens;

  const output = {
    type: 'message',
    message: {
      usage: usageObj,
    },
  };

  if (tsResult !== undefined) {
    output.timestamp = tsResult;
  }
  if (uuid) {
    output.uuid = uuid;
  }
  if (requestId) {
    output.requestId = requestId;
  }
  if (messageId) {
    output.message.id = messageId;
  }
  if (model) {
    output.message.model = model;
  }

  return JSON.stringify(output);
}

export function generateRedactorMetadata(accepted, rejected) {
  return {
    schema_version: '1.0.0',
    redactor_version: '1.0.0',
    source_kind: 'claude',
    generated_at: new Date().toISOString(),
    record_count: accepted,
    rejected_count: rejected,
  };
}

export async function redactClaudeFileStreaming(inputFile, outputFile, metadataFile, options = {}) {
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
      const redacted = redactClaudeLine(trimmed, ctx);
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
    console.error('Usage: node scripts/redact-claude.mjs <input.jsonl> <output.jsonl> [metadata.json]');
    process.exit(1);
  }
  try {
    const res = await redactClaudeFileStreaming(inputFile, outputFile, metadataFile);
    console.log(`Processed: ${res.total}, accepted: ${res.accepted}, rejected: ${res.rejected}`);
  } catch (err) {
    console.error(`Redaction failed: ${err.message}`);
    process.exit(1);
  }
}

if (process.argv[1] && process.argv[1].endsWith('redact-claude.mjs')) {
  runCli();
}
