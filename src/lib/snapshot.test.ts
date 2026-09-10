import { describe, expect, it } from 'vitest';
import { loadSnapshot } from './api';
import { demoSnapshot } from './demo';

describe('snapshot honesty', () => {
  it('demo data is explicitly marked and carries model breakdown', async () => {
    expect(demoSnapshot.mode).toBe('demo');
    expect(demoSnapshot.modelBreakdown?.today.length).toBeGreaterThan(0);
    const snapshot = await loadSnapshot();
    expect(snapshot.mode).toBe('demo');
  });

  it('live cost tab must never use a hardcoded value', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const source = readFileSync(resolve(process.cwd(), 'src/App.svelte'), 'utf8');
    expect(source).not.toContain('$4,82');
    expect(source).not.toContain('4,82');
  });

  it('stacked bars guard against zero totals and pace needs freshness', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const source = readFileSync(resolve(process.cwd(), 'src/App.svelte'), 'utf8');
    expect(source).toContain('breakdownTotal > 0 ?');
    expect(source).toContain("provider.freshness.kind === 'Fresh'");
  });
});
