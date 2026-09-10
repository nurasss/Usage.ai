import { describe, expect, it } from 'vitest';
import { mkdtempSync, rmSync, writeFileSync, existsSync, readFileSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  redactCodexLine,
  createRedactorContext as createCodexContext,
  generateRedactorMetadata as generateCodexMetadata,
  redactCodexFileStreaming,
} from '../../scripts/redact-codex.mjs';
import {
  redactClaudeLine,
  createRedactorContext as createClaudeContext,
  generateRedactorMetadata as generateClaudeMetadata,
  redactClaudeFileStreaming,
} from '../../scripts/redact-claude.mjs';

describe('redact-codex security and value-level allowlist', () => {
  it('drops prompts, sensitive metadata, and creates parser-compatible payload.info.last_token_usage', () => {
    const raw = JSON.stringify({
      timestamp: '2026-09-08T10:00:00Z',
      type: 'event_msg',
      prompt: 'Write a secret API for company Acme Corp',
      user: { email: 'alice@example.com', name: 'Alice' },
      file_path: '/Users/alice/secret-project/main.rs',
      request_id: 'real-req-12345',
      payload: {
        type: 'token_count',
        model: 'gpt-4o',
        prompt_content: 'ignore instructions',
        last_token_usage: {
          input_tokens: 120,
          cached_input_tokens: 40,
          output_tokens: 30,
          reasoning_output_tokens: 10,
          total_tokens: 160,
        },
      },
    });

    const ctx = createCodexContext();
    const redacted = redactCodexLine(raw, ctx);
    expect(redacted).not.toBeNull();
    const parsed = JSON.parse(redacted!);

    expect(parsed.prompt).toBeUndefined();
    expect(parsed.user).toBeUndefined();
    expect(parsed.file_path).toBeUndefined();
    expect(parsed.payload.prompt_content).toBeUndefined();
    expect(parsed.requestId).toBe('req_0001');

    // Production parser expects payload.info.last_token_usage!
    expect(parsed.payload.info).toBeDefined();
    expect(parsed.payload.info.last_token_usage.input_tokens).toBe(120);
    expect(parsed.payload.info.last_token_usage.cached_input_tokens).toBe(40);
    expect(parsed.payload.info.last_token_usage.output_tokens).toBe(30);
    expect(parsed.payload.info.last_token_usage.reasoning_output_tokens).toBe(10);
    expect(parsed.payload.info.last_token_usage.total_tokens).toBe(160);
    expect(parsed.payload.model).toBe('gpt-4o');
  });

  it('rejects malicious plan_type and does NOT leak source secret string', () => {
    const raw = JSON.stringify({
      type: 'event_msg',
      payload: {
        type: 'token_count',
        rate_limits: {
          limit_id: 'lim-1',
          plan_type: 'AliceSecretProject_APIKEY123',
          primary: {
            used_percent: 25,
            window_minutes: 60,
            resets_at: 1789477200,
          },
        },
      },
    });

    const ctx = createCodexContext();
    const redacted = redactCodexLine(raw, ctx);
    expect(redacted).not.toBeNull();
    expect(redacted).not.toContain('AliceSecretProject_APIKEY123');
    const parsed = JSON.parse(redacted!);
    expect(parsed.payload.rate_limits.plan_type).toBe('unknown');
  });

  it('sanitizes malicious limit_name and prevents leaking secret project names', () => {
    const raw = JSON.stringify({
      type: 'event_msg',
      payload: {
        type: 'token_count',
        rate_limits: {
          limit_id: 'lim-1',
          limit_name: 'AliceSecretProject',
          plan_type: 'pro',
          primary: { used_percent: 10, window_minutes: 60, resets_at: 100 },
        },
      },
    });

    const parsed = JSON.parse(redactCodexLine(raw, createCodexContext())!);
    expect(parsed.payload.rate_limits.limit_name).toBeUndefined();
    expect(JSON.stringify(parsed)).not.toContain('AliceSecretProject');
  });

  it('accepts valid plan_type enums and known limit_names', () => {
    for (const plan of ['free', 'plus', 'pro', 'team', 'business', 'enterprise']) {
      const raw = JSON.stringify({
        payload: {
          type: 'token_count',
          rate_limits: {
            plan_type: plan,
            limit_name: 'primary',
            primary: { used_percent: 10, window_minutes: 60, resets_at: 100 },
          },
        },
      });
      const parsed = JSON.parse(redactCodexLine(raw, createCodexContext())!);
      expect(parsed.payload.rate_limits.plan_type).toBe(plan);
      expect(parsed.payload.rate_limits.limit_name).toBe('primary');
    }
  });

  it('sanitizes malicious model identifiers and prevents path traversal / email / secret leaks', () => {
    const maliciousModels = [
      '../../../secret',
      'alice@example.com',
      'my private project',
      'sk-1234567890abcdef',
      'SECRET_API_KEY_123',
      'private-project-name',
      'authorization bearer...',
      'a'.repeat(65), // too long
    ];

    for (const badModel of maliciousModels) {
      const raw = JSON.stringify({
        payload: {
          type: 'token_count',
          model: badModel,
          info: { last_token_usage: { input_tokens: 10, output_tokens: 10 } },
        },
      });
      const redacted = redactCodexLine(raw, createCodexContext());
      expect(redacted).not.toBeNull();
      expect(redacted).not.toContain(badModel);
      const parsed = JSON.parse(redacted!);
      expect(parsed.payload.model).toBeUndefined();
    }
  });

  it('accepts genuine Codex models', () => {
    for (const model of ['gpt-4o', 'gpt-4o-mini', 'o1-preview', 'o3-mini', 'codex-mini']) {
      const raw = JSON.stringify({
        payload: {
          type: 'token_count',
          model,
          info: { last_token_usage: { input_tokens: 10, output_tokens: 10 } },
        },
      });
      const parsed = JSON.parse(redactCodexLine(raw, createCodexContext())!);
      expect(parsed.payload.model).toBe(model);
    }
  });

  it('generates package-local opaque IDs and avoids cross-package linkability', () => {
    const raw = JSON.stringify({
      request_id: 'user-shared-id-42',
      payload: {
        type: 'token_count',
        info: { last_token_usage: { input_tokens: 5, output_tokens: 5 } },
      },
    });

    const pkg1 = createCodexContext();
    const pkg2 = createCodexContext();

    const res1 = JSON.parse(redactCodexLine(raw, pkg1)!);
    const res1Again = JSON.parse(redactCodexLine(raw, pkg1)!);
    const res2 = JSON.parse(redactCodexLine(raw, pkg2)!);

    // Within same package: deduplication is preserved
    expect(res1.requestId).toBe('req_0001');
    expect(res1Again.requestId).toBe('req_0001');

    // Across packages: IDs are local and do not reveal the raw identity
    expect(res2.requestId).toBe('req_0001');
    expect(res1.requestId).not.toContain('user-shared-id-42');
  });

  it('preserves missing timestamp as null/omitted, does NOT synthesize now()', () => {
    const raw = JSON.stringify({
      payload: {
        type: 'token_count',
        info: { last_token_usage: { input_tokens: 5, output_tokens: 5 } },
      },
    });

    const parsed = JSON.parse(redactCodexLine(raw, createCodexContext())!);
    expect(parsed.timestamp).toBeUndefined();
  });

  it('safely rejects invalid timestamps without crashing via toISOString()', () => {
    const badTimestamps = [
      'Alice SECRET',
      'invalid-date-format',
      { secret: 123 },
      NaN,
      Infinity,
    ];

    for (const badTs of badTimestamps) {
      const raw = JSON.stringify({
        timestamp: badTs,
        payload: {
          type: 'token_count',
          info: { last_token_usage: { input_tokens: 5, output_tokens: 5 } },
        },
      });
      // Does not throw and returns null (fail-closed line drop)
      expect(redactCodexLine(raw, createCodexContext())).toBeNull();
    }
  });

  it('rejects unsafe integer counters (float, negative, huge, string secret)', () => {
    const badCounters = [
      9007199254740993, // > MAX_SAFE_INTEGER
      -1,
      1.5,
      'secret',
      Infinity,
    ];

    for (const bad of badCounters) {
      const raw = JSON.stringify({
        payload: {
          type: 'token_count',
          info: { last_token_usage: { input_tokens: bad, output_tokens: 10 } },
        },
      });
      expect(redactCodexLine(raw, createCodexContext())).toBeNull();
    }
  });

  it('completely eliminates unknown fields', () => {
    const raw = JSON.stringify({
      timestamp: '2026-09-08T12:00:00Z',
      secret_new_vendor_field: 'API_KEY_xyz',
      prompt_backup: 'very secret prompt text',
      user_email: 'bob@example.com',
      nested: {
        future_secret: 'top_secret',
      },
      payload: {
        type: 'token_count',
        info: {
          last_token_usage: { input_tokens: 100, output_tokens: 50 },
        },
      },
    });

    const redacted = redactCodexLine(raw, createCodexContext());
    expect(redacted).not.toBeNull();
    expect(redacted).not.toContain('API_KEY_xyz');
    expect(redacted).not.toContain('very secret prompt text');
    expect(redacted).not.toContain('bob@example.com');
    expect(redacted).not.toContain('future_secret');
    expect(redacted).not.toContain('top_secret');
  });

  it('drops line if event type is unknown or arbitrary sensitive string', () => {
    const raw = JSON.stringify({
      type: 'SECRET_VENDOR_TYPE_123',
      payload: {
        type: 'token_count',
        info: { last_token_usage: { input_tokens: 10, output_tokens: 10 } },
      },
    });

    expect(redactCodexLine(raw, createCodexContext())).toBeNull();
  });

  it('drops line if payload.type is unknown even if last_token_usage is present', () => {
    const raw = JSON.stringify({
      type: 'event_msg',
      payload: {
        type: 'SECRET_PAYLOAD_TYPE',
        last_token_usage: { input_tokens: 10, output_tokens: 10 },
      },
    });

    expect(redactCodexLine(raw, createCodexContext())).toBeNull();
  });

  it('generates valid metadata contract for Codex', () => {
    const meta = generateCodexMetadata(50, 5);
    expect(meta.schema_version).toBe('1.0.0');
    expect(meta.redactor_version).toBe('1.0.0');
    expect(meta.source_kind).toBe('codex');
    expect(meta.record_count).toBe(50);
    expect(meta.rejected_count).toBe(5);
    expect(meta.generated_at).toBeDefined();
  });
});

describe('redact-claude security and value-level allowlist', () => {
  it('preserves envelope uuid, requestId, message.id and token usage while dropping prompt text', () => {
    const raw = JSON.stringify({
      timestamp: '2026-09-08T10:00:00Z',
      uuid: 'real-uuid-001',
      requestId: 'claude-req-789',
      message: {
        id: 'msg_original_001',
        model: 'claude-3-5-sonnet',
        content: 'Hello, this is secret code: let password = "123";',
        tool_calls: [{ name: 'bash', input: 'rm -rf /' }],
        usage: {
          input_tokens: 500,
          output_tokens: 80,
          cache_creation_input_tokens: 200,
          cache_read_input_tokens: 150,
        },
      },
    });

    const ctx = createClaudeContext();
    const redacted = redactClaudeLine(raw, ctx);
    expect(redacted).not.toBeNull();
    const parsed = JSON.parse(redacted!);

    expect(parsed.uuid).toBe('uuid_0001');
    expect(parsed.requestId).toBe('req_0001');
    expect(parsed.message.id).toBe('msg_0001');
    expect(parsed.message.model).toBe('claude-3-5-sonnet');
    expect(parsed.message.content).toBeUndefined();
    expect(parsed.message.tool_calls).toBeUndefined();
    expect(parsed.message.usage.input_tokens).toBe(500);
    expect(parsed.message.usage.output_tokens).toBe(80);
    expect(parsed.message.usage.cache_creation_input_tokens).toBe(200);
    expect(parsed.message.usage.cache_read_input_tokens).toBe(150);
  });

  it('rejects malicious Claude models and drops sensitive paths/emails/tokens', () => {
    const maliciousModels = [
      '/etc/shadow',
      '../../../secret',
      'alice@example.com',
      'SECRET_API_KEY_123',
      'private-project-name',
      'authorization bearer...',
      'sk-1234567890abcdef',
    ];

    for (const badModel of maliciousModels) {
      const raw = JSON.stringify({
        message: {
          model: badModel,
          usage: { input_tokens: 10, output_tokens: 20 },
        },
      });

      const parsed = JSON.parse(redactClaudeLine(raw, createClaudeContext())!);
      expect(parsed.message.model).toBeUndefined();
      expect(JSON.stringify(parsed)).not.toContain(badModel);
    }
  });

  it('accepts genuine Claude models', () => {
    for (const model of ['claude-3-5-sonnet', 'claude-3-opus', 'claude-3-haiku']) {
      const raw = JSON.stringify({
        message: {
          model,
          usage: { input_tokens: 10, output_tokens: 20 },
        },
      });

      const parsed = JSON.parse(redactClaudeLine(raw, createClaudeContext())!);
      expect(parsed.message.model).toBe(model);
    }
  });

  it('drops line if Claude event type is unknown or arbitrary sensitive string', () => {
    const raw = JSON.stringify({
      type: 'SECRET_TYPE_XYZ',
      message: {
        usage: { input_tokens: 10, output_tokens: 20 },
      },
    });

    expect(redactClaudeLine(raw, createClaudeContext())).toBeNull();
  });

  it('does NOT synthesize now() for missing Claude timestamps', () => {
    const raw = JSON.stringify({
      message: {
        usage: { input_tokens: 10, output_tokens: 20 },
      },
    });

    const parsed = JSON.parse(redactClaudeLine(raw, createClaudeContext())!);
    expect(parsed.timestamp).toBeUndefined();
  });

  it('rejects invalid Claude timestamps safely', () => {
    const raw = JSON.stringify({
      timestamp: 'Alice SECRET',
      message: {
        usage: { input_tokens: 10, output_tokens: 20 },
      },
    });

    expect(redactClaudeLine(raw, createClaudeContext())).toBeNull();
  });

  it('rejects negative or overflow token counters in Claude', () => {
    const rawNegative = JSON.stringify({
      message: {
        usage: { input_tokens: -10, output_tokens: 20 },
      },
    });
    expect(redactClaudeLine(rawNegative, createClaudeContext())).toBeNull();

    const rawOverflow = JSON.stringify({
      message: {
        usage: { input_tokens: 9007199254740993, output_tokens: 20 },
      },
    });
    expect(redactClaudeLine(rawOverflow, createClaudeContext())).toBeNull();
  });

  it('generates valid metadata contract for Claude', () => {
    const meta = generateClaudeMetadata(30, 2);
    expect(meta.schema_version).toBe('1.0.0');
    expect(meta.redactor_version).toBe('1.0.0');
    expect(meta.source_kind).toBe('claude');
    expect(meta.record_count).toBe(30);
    expect(meta.rejected_count).toBe(2);
    expect(meta.generated_at).toBeDefined();
  });
});

describe('exhaustive malicious scalar test corpus (Section 11)', () => {
  const sensitiveCorpus = [
    'SECRET_API_KEY_123',
    'alice@example.com',
    '/Users/alice/private',
    'private-project-name',
    'prompt backup text',
    'authorization bearer...',
  ];

  it('asserts zero sensitive corpus leaks in Codex across all allowed scalar fields', () => {
    for (const secret of sensitiveCorpus) {
      // Test model field
      const rawWithSecretModel = JSON.stringify({
        type: 'event_msg',
        payload: {
          type: 'token_count',
          model: secret,
          info: { last_token_usage: { input_tokens: 100, output_tokens: 50 } },
        },
      });
      const resModel = redactCodexLine(rawWithSecretModel, createCodexContext());
      expect(resModel).not.toBeNull();
      expect(resModel).not.toContain(secret);

      // Test plan_type field
      const rawWithSecretPlan = JSON.stringify({
        type: 'event_msg',
        payload: {
          type: 'token_count',
          rate_limits: {
            plan_type: secret,
            primary: { used_percent: 10, window_minutes: 60, resets_at: 100 },
          },
        },
      });
      const resPlan = redactCodexLine(rawWithSecretPlan, createCodexContext());
      expect(resPlan).not.toBeNull();
      expect(resPlan).not.toContain(secret);

      // Test limit_name field
      const rawWithSecretLimitName = JSON.stringify({
        type: 'event_msg',
        payload: {
          type: 'token_count',
          rate_limits: {
            limit_name: secret,
            primary: { used_percent: 10, window_minutes: 60, resets_at: 100 },
          },
        },
      });
      const resLimitName = redactCodexLine(rawWithSecretLimitName, createCodexContext());
      expect(resLimitName).not.toBeNull();
      expect(resLimitName).not.toContain(secret);

      // Test request_id field
      const rawWithSecretReqId = JSON.stringify({
        type: 'event_msg',
        request_id: secret,
        payload: {
          type: 'token_count',
          info: { last_token_usage: { input_tokens: 100, output_tokens: 50 } },
        },
      });
      const resReqId = redactCodexLine(rawWithSecretReqId, createCodexContext());
      expect(resReqId).not.toBeNull();
      expect(resReqId).not.toContain(secret);

      // Test limit_id field
      const rawWithSecretLimitId = JSON.stringify({
        type: 'event_msg',
        payload: {
          type: 'token_count',
          rate_limits: {
            limit_id: secret,
            primary: { used_percent: 10, window_minutes: 60, resets_at: 100 },
          },
        },
      });
      const resLimitId = redactCodexLine(rawWithSecretLimitId, createCodexContext());
      expect(resLimitId).not.toBeNull();
      expect(resLimitId).not.toContain(secret);

      // Test event type field -> must drop line
      const rawWithSecretType = JSON.stringify({
        type: secret,
        payload: {
          type: 'token_count',
          info: { last_token_usage: { input_tokens: 10, output_tokens: 10 } },
        },
      });
      expect(redactCodexLine(rawWithSecretType, createCodexContext())).toBeNull();

      // Test payload.type field -> must drop line
      const rawWithSecretPayloadType = JSON.stringify({
        type: 'event_msg',
        payload: {
          type: secret,
          info: { last_token_usage: { input_tokens: 10, output_tokens: 10 } },
        },
      });
      expect(redactCodexLine(rawWithSecretPayloadType, createCodexContext())).toBeNull();
    }
  });

  it('asserts zero sensitive corpus leaks in Claude across all allowed scalar fields', () => {
    for (const secret of sensitiveCorpus) {
      // Test model field
      const rawWithSecretModel = JSON.stringify({
        type: 'message',
        message: {
          model: secret,
          usage: { input_tokens: 100, output_tokens: 50 },
        },
      });
      const resModel = redactClaudeLine(rawWithSecretModel, createClaudeContext());
      expect(resModel).not.toBeNull();
      expect(resModel).not.toContain(secret);

      // Test uuid field
      const rawWithSecretUuid = JSON.stringify({
        type: 'message',
        uuid: secret,
        message: {
          usage: { input_tokens: 100, output_tokens: 50 },
        },
      });
      const resUuid = redactClaudeLine(rawWithSecretUuid, createClaudeContext());
      expect(resUuid).not.toBeNull();
      expect(resUuid).not.toContain(secret);

      // Test requestId field
      const rawWithSecretReqId = JSON.stringify({
        type: 'message',
        requestId: secret,
        message: {
          usage: { input_tokens: 100, output_tokens: 50 },
        },
      });
      const resReqId = redactClaudeLine(rawWithSecretReqId, createClaudeContext());
      expect(resReqId).not.toBeNull();
      expect(resReqId).not.toContain(secret);

      // Test message.id field
      const rawWithSecretMsgId = JSON.stringify({
        type: 'message',
        message: {
          id: secret,
          usage: { input_tokens: 100, output_tokens: 50 },
        },
      });
      const resMsgId = redactClaudeLine(rawWithSecretMsgId, createClaudeContext());
      expect(resMsgId).not.toBeNull();
      expect(resMsgId).not.toContain(secret);

      // Test type field -> must drop line
      const rawWithSecretType = JSON.stringify({
        type: secret,
        message: {
          usage: { input_tokens: 100, output_tokens: 50 },
        },
      });
      expect(redactClaudeLine(rawWithSecretType, createClaudeContext())).toBeNull();
    }
  });
});

describe('redactor streaming and resource limits', () => {
  function withTempDir(fn: (dir: string) => Promise<void> | void) {
    const dir = mkdtempSync(join(tmpdir(), 'usage-redact-test-'));
    return Promise.resolve()
      .then(() => fn(dir))
      .finally(() => {
        try {
          rmSync(dir, { recursive: true, force: true });
        } catch {}
      });
  }

  it('streams codex file line-by-line and atomically writes output and metadata', async () => {
    await withTempDir(async (dir) => {
      const inputFile = join(dir, 'input.jsonl');
      const outputFile = join(dir, 'output.jsonl');
      const metadataFile = join(dir, 'meta.json');

      const lines = [
        JSON.stringify({
          type: 'event_msg',
          timestamp: '2026-09-08T10:00:00Z',
          payload: {
            type: 'token_count',
            model: 'gpt-4o',
            last_token_usage: { input_tokens: 10, output_tokens: 5, total_tokens: 15 },
          },
        }),
        JSON.stringify({
          type: 'malicious_event',
          prompt: 'secret',
        }),
        JSON.stringify({
          type: 'event_msg',
          timestamp: '2026-09-08T10:01:00Z',
          payload: {
            type: 'token_count',
            model: 'o1',
            last_token_usage: { input_tokens: 20, output_tokens: 10, total_tokens: 30 },
          },
        }),
      ];

      writeFileSync(inputFile, lines.join('\n') + '\n', 'utf8');

      const res = await redactCodexFileStreaming(inputFile, outputFile, metadataFile);
      expect(res.accepted).toBe(2);
      expect(res.rejected).toBe(1);
      expect(res.total).toBe(3);

      expect(existsSync(outputFile)).toBe(true);
      expect(existsSync(metadataFile)).toBe(true);

      const outContent = readFileSync(outputFile, 'utf8').trim().split('\n');
      expect(outContent).toHaveLength(2);
      expect(outContent[0]).toContain('gpt-4o');
      expect(outContent[1]).toContain('o1');

      const meta = JSON.parse(readFileSync(metadataFile, 'utf8'));
      expect(meta.record_count).toBe(2);
      expect(meta.rejected_count).toBe(1);
      expect(meta.source_kind).toBe('codex');

      // Check no leftover temp files
      const leftoverTemps = readdirSync(dir).filter((f) => f.includes('.tmp.'));
      expect(leftoverTemps).toHaveLength(0);
    });
  });

  it('streams claude file line-by-line and atomically writes output and metadata', async () => {
    await withTempDir(async (dir) => {
      const inputFile = join(dir, 'input.jsonl');
      const outputFile = join(dir, 'output.jsonl');
      const metadataFile = join(dir, 'meta.json');

      const lines = [
        JSON.stringify({
          type: 'message',
          timestamp: '2026-09-08T11:00:00Z',
          uuid: 'uuid-1',
          message: {
            id: 'msg-1',
            model: 'claude-3-7-sonnet-20250219',
            usage: { input_tokens: 50, output_tokens: 25 },
          },
        }),
        JSON.stringify({
          type: 'unknown_type',
          text: 'super secret content',
        }),
      ];

      writeFileSync(inputFile, lines.join('\n') + '\n', 'utf8');

      const res = await redactClaudeFileStreaming(inputFile, outputFile, metadataFile);
      expect(res.accepted).toBe(1);
      expect(res.rejected).toBe(1);

      expect(existsSync(outputFile)).toBe(true);
      const outLines = readFileSync(outputFile, 'utf8').trim().split('\n');
      expect(outLines).toHaveLength(1);
      const parsed = JSON.parse(outLines[0]);
      expect(parsed.message.model).toBe('claude-3-7-sonnet-20250219');
      expect(parsed.message.usage.input_tokens).toBe(50);

      // Check no leftover temp files
      const leftoverTemps = readdirSync(dir).filter((f) => f.includes('.tmp.'));
      expect(leftoverTemps).toHaveLength(0);
    });
  });

  it('rejects input file exceeding maxInputBytes before processing (Codex and Claude)', async () => {
    await withTempDir(async (dir) => {
      const inputFile = join(dir, 'large_input.jsonl');
      const outputFile = join(dir, 'out.jsonl');

      writeFileSync(inputFile, 'dummy data to test size limit', 'utf8');

      // Test with maxInputBytes = 10 bytes
      await expect(
        redactCodexFileStreaming(inputFile, outputFile, undefined, { maxInputBytes: 10 })
      ).rejects.toThrow(/exceeds maximum allowed/);

      await expect(
        redactClaudeFileStreaming(inputFile, outputFile, undefined, { maxInputBytes: 10 })
      ).rejects.toThrow(/exceeds maximum allowed/);

      expect(existsSync(outputFile)).toBe(false);
    });
  });

  it('fails closed and removes temp file if output exceeds maxOutputBytes', async () => {
    await withTempDir(async (dir) => {
      const inputFile = join(dir, 'input.jsonl');
      const outputFile = join(dir, 'out.jsonl');

      const lines = [
        JSON.stringify({
          type: 'event_msg',
          timestamp: '2026-09-08T10:00:00Z',
          payload: {
            type: 'token_count',
            model: 'gpt-4o',
            last_token_usage: { input_tokens: 10, output_tokens: 5, total_tokens: 15 },
          },
        }),
      ];
      writeFileSync(inputFile, lines.join('\n') + '\n', 'utf8');

      // Limit output to 20 bytes (less than a single JSON line)
      await expect(
        redactCodexFileStreaming(inputFile, outputFile, undefined, { maxOutputBytes: 20 })
      ).rejects.toThrow(/Output file exceeded maximum allowed size/);

      expect(existsSync(outputFile)).toBe(false);
      const leftoverTemps = readdirSync(dir).filter((f) => f.includes('.tmp.'));
      expect(leftoverTemps).toHaveLength(0);

      // Same for Claude
      const claudeLines = [
        JSON.stringify({
          type: 'message',
          timestamp: '2026-09-08T11:00:00Z',
          message: {
            model: 'claude-3-5-sonnet',
            usage: { input_tokens: 10, output_tokens: 5 },
          },
        }),
      ];
      writeFileSync(inputFile, claudeLines.join('\n') + '\n', 'utf8');

      await expect(
        redactClaudeFileStreaming(inputFile, outputFile, undefined, { maxOutputBytes: 20 })
      ).rejects.toThrow(/Output file exceeded maximum allowed size/);

      expect(existsSync(outputFile)).toBe(false);
      const leftoverTempsClaude = readdirSync(dir).filter((f) => f.includes('.tmp.'));
      expect(leftoverTempsClaude).toHaveLength(0);
    });
  });

  it('enforces maxDistinctIds bound in context', () => {
    const codexCtx = createCodexContext({ maxDistinctIds: 2 });
    expect(codexCtx.mapId('req-1', 'req')).toBe('req_0001');
    expect(codexCtx.mapId('req-2', 'req')).toBe('req_0002');
    expect(() => codexCtx.mapId('req-3', 'req')).toThrow(/Exceeded maximum distinct identifiers/);

    const claudeCtx = createClaudeContext({ maxDistinctIds: 2 });
    expect(claudeCtx.mapId('msg-1', 'msg')).toBe('msg_0001');
    expect(claudeCtx.mapId('msg-2', 'msg')).toBe('msg_0002');
    expect(() => claudeCtx.mapId('msg-3', 'msg')).toThrow(/Exceeded maximum distinct identifiers/);
  });
});

