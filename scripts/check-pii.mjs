// PII/secret tripwire (§13.1, V13-04): scans tracked text for
// secret-shaped material. Known-safe placeholders stay allowlisted
// (example.invalid, REDACTED, *-test fakes). Fails closed: any hit
// outside the allowlist breaks the gate.
// Usage: node scripts/check-pii.mjs
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const TRACKED = execFileSync('git', ['ls-files', '-z'], { encoding: 'buffer' })
  .toString()
  .split('\0')
  .filter(Boolean)
  .filter((f) => !f.startsWith('target/') && !f.includes('/target/'));
// Untracked-but-visible files too: a secret must trip the wire
// before it is ever committed.
let UNTRACKED = [];
try {
  UNTRACKED = execFileSync('git', ['ls-files', '-o', '--exclude-standard', '-z'], { encoding: 'buffer' })
    .toString()
    .split('\0')
    .filter(Boolean);
} catch { /* read-only fallback: tracked only */ }
const FILES = [...new Set([...TRACKED, ...UNTRACKED])];

// [pattern, description]; applied per line.
const RULES = [
  [/sk-[A-Za-z0-9\-_]{8,}/, 'openai-like key'],
  [/xox[baprs]-[A-Za-z0-9\-]+/, 'slack-like token'],
  [/gh[op]_[A-Za-z0-9]{8,}/, 'github-like token'],
  [/AIza[A-Za-z0-9\-_]{10,}/, 'google-like key'],
  [/-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----/, 'pem private key'],
  [/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/, 'email address'],
  [/["']?[A-Za-z0-9]{32,}["']?/, 'long opaque token (32+ alnum)'],
];

// Allowlisted literals/filenames: placeholders and documented test
// fakes (audit probe strings, redaction unit fixtures). Each entry is
// a deliberate, reviewable exception — never a live credential.
const ALLOW_PATH = [/^scripts\/check-pii\.mjs$/, /CHANGELOG/i, /fixtures\/real\//];
const ALLOW_TEXT = [
  'example.invalid',
  '@example.com',
  '/Users/alice/',
  'REDACTED',
  'redacted',
  '[redacted',
  'sk-test',
  'sk-original',
  'sk-proj-secret',
  'SECRET_API_KEY_123',
  'Bearer abcdef',
  'sk-1234567890abcdef',
  'sk-abc123XYZ4567890',
  'sk-AAAAAAAAAAAAAAAA',
  'abcdefghijklmnopqrstuvwxyz',
  'oauth-test-token',
  'test-token',
  'token-secret',
  'api_key_secret',
  'password=123',
  'xoxb-123456789012',
  'ghp_abcdefgh12345678',
  'MIIBPA',
  '@2x.png',
  'user@example.com',
  'someone@example.com',
  'nuras',
];
// Git SHAs / content hashes in audit docs and lockfiles are not
// credentials; skip the opaque-token rule on hash-context lines.
const HASH_CONTEXT = /sha[-_ ]?256|sha[-_ ]?512|\bcommit\b|revision|baseline|\bHEAD\b|checksum|integrity/i;

let failures = 0;
for (const file of FILES) {
  if (ALLOW_PATH.some((re) => re.test(file))) continue;
  let text;
  try {
    text = readFileSync(file, 'utf8');
  } catch {
    continue; // binary
  }
  if (text.includes('\0')) continue;
  for (const [lineNo, line] of text.split('\n').entries()) {
    for (const [re, what] of RULES) {
      const hit = line.match(re);
      if (!hit) continue;
      const allowed = ALLOW_TEXT.some((a) => line.includes(a));
      if (allowed) continue;
      // Bare-token rule refinements: pure 40/64-hex are content
      // hashes, digit-less runs are identifiers, hash-context lines
      // are audit metadata — none are credentials.
      if (what.startsWith('long opaque')) {
        const token = hit[0];
        if (/^[0-9a-f]{40}$/.test(token) || /^[0-9a-f]{64}$/.test(token)) continue;
        if (!/\d/.test(token)) continue;
        if (HASH_CONTEXT.test(`${file} ${line.slice(0, 120)}`)) continue;
      }
      if (file.endsWith('.lock') || file.endsWith('.snap')) continue;
      console.error(`PII tripwire: ${what} in ${file}:${lineNo + 1}: ${line.trim().slice(0, 90)}`);
      failures += 1;
    }
  }
}
if (failures > 0) {
  console.error(`\nPII tripwire FAILED: ${failures} hit(s). Redact or allowlist explicitly.`);
  process.exit(1);
}
console.log('PII tripwire: clean.');
