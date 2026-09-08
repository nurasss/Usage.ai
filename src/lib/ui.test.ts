// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { applyTheme, navKey } from './ui';

describe('account-aware navigation keys', () => {
  it('combines provider, product and account', () => {
    expect(navKey('openai', 'codex', 'abc-123')).toBe('openai__codex__abc-123');
  });
  it('two accounts of one product never share a key', () => {
    expect(navKey('openai', 'openai-api', 'a1')).not.toBe(navKey('openai', 'openai-api', 'a2'));
  });
  it('sanitizes unsafe characters', () => {
    expect(navKey('a/b', 'c d', 'e.f')).toBe('a_b__c_d__e_f');
  });
});

describe('theme application', () => {
  it('sets and clears the root override', () => {
    applyTheme('dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    applyTheme('light');
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    applyTheme('system');
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });
});
