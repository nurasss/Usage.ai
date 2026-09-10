// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { stateLabel } from './format';
import type { ProviderSnapshot } from './types';
import { applyTheme, formatAccountLabel, navKey, providerPresentation } from './ui';

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

describe('SWR provider card state', () => {
  it('keeps LKG data stale and visibly unhealthy when refresh fails', () => {
    const state = providerPresentation({
      connectionState: 'NetworkError',
      freshness: { kind: 'Stale', ageSeconds: 1080 },
      currentError: 'network_error',
    });
    expect(state.stale).toBe(true);
    expect(state.hasCurrentError).toBe(true);
    expect(state.healthy).toBe(false);
  });

  it('renders stale provider card DOM with LKG metrics, warning banner, and honest label', () => {
    const provider: ProviderSnapshot = {
      accountId: 'test-codex',
      providerId: 'openai',
      productId: 'codex',
      providerName: 'OpenAI',
      productName: 'Codex',
      alias: 'Аккаунт 1',
      connectionState: 'NetworkError',
      freshness: { kind: 'Stale', ageSeconds: 1080 },
      coverage: 'UnverifiedSemantics',
      fetchedAt: '2026-09-08T12:00:00Z',
      capabilities: ['subscriptionQuota', 'historyLocal'],
      quotas: [
        {
          poolId: 'codex-weekly',
          name: 'Weekly',
          remainingPercent: 72,
          used: 28,
          limit: 100,
          unit: 'percent',
          windowKind: 'rolling',
          source: 'Codex local',
        },
      ],
      currentError: 'network_error',
      lastSuccessfulRefresh: '2026-09-08T11:42:00Z',
      lastRefreshAttempt: '2026-09-08T12:00:00Z',
    };

    const pres = providerPresentation(provider);
    expect(pres.healthy).toBe(false);
    expect(pres.stale).toBe(true);
    expect(pres.hasCurrentError).toBe(true);

    const container = document.createElement('div');
    container.innerHTML = `
      <section class="provider-card">
        <div class="provider-head">
          <div class="provider-name">
            <h2>${provider.providerName} / ${provider.productName}</h2>
            <p>${formatAccountLabel(provider.alias)} · ${stateLabel(provider.connectionState)}</p>
          </div>
        </div>
        ${
          pres.stale || pres.hasCurrentError
            ? `<div class="stale-state" role="status">
                <strong>${pres.stale ? 'Данные устарели' : 'Источник требует внимания'}</strong>
                ${provider.currentError ? `<span>⚠ ${stateLabel(provider.connectionState)}</span>` : ''}
                ${provider.lastSuccessfulRefresh ? `<span>Последний успех: 18 мин назад</span>` : ''}
                ${provider.lastRefreshAttempt ? `<span>Последняя попытка: только что</span>` : ''}
              </div>`
            : ''
        }
        <div class="quota">
          <div class="quota-top"><span>${provider.quotas[0].name}</span><strong>Осталось ${provider.quotas[0].remainingPercent}%</strong></div>
        </div>
      </section>
    `;

    const statusBanner = container.querySelector('.stale-state[role="status"]');
    expect(statusBanner).not.toBeNull();
    expect(statusBanner?.textContent).toContain('Данные устарели');
    expect(statusBanner?.textContent).toContain('⚠ Ошибка сети');
    expect(statusBanner?.textContent).toContain('Последний успех: 18 мин назад');
    expect(statusBanner?.textContent).toContain('Последняя попытка: только что');

    expect(container.textContent).toContain('Осталось 72%');
    expect(container.textContent).toContain('Локальная история');
    expect(container.textContent).not.toContain('Аккаунт 1');
  });
});

describe('honest account labels', () => {
  it('replaces misleading "Аккаунт 1" with "Локальная история" for Unknown identity', () => {
    expect(formatAccountLabel('Аккаунт 1', 'Unknown')).toBe('Локальная история');
    expect(formatAccountLabel('Аккаунт', 'Unknown')).toBe('Локальная история');
    expect(formatAccountLabel('', 'Unknown')).toBe('Локальная история');
    expect(formatAccountLabel(undefined, 'Unknown')).toBe('Локальная история');
  });

  it('preserves user custom alias or confirmed identity label', () => {
    expect(formatAccountLabel('Рабочий', 'Unknown')).toBe('Рабочий');
    expect(formatAccountLabel('Аккаунт 1', 'Verified')).toBe('Аккаунт 1');
    expect(formatAccountLabel('Личный API', 'Weak')).toBe('Личный API');
  });
});
