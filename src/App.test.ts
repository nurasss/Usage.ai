// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import App from './App.svelte';
import * as api from './lib/api';
import type { AppSnapshot, ProviderSnapshot } from './lib/types';

vi.mock('./lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./lib/api')>();
  return {
    ...actual,
    loadSnapshot: vi.fn(),
    loadSettings: vi.fn().mockResolvedValue(actual.defaultSettings),
    loadDescriptors: vi.fn().mockResolvedValue([]),
    loadDiagnostics: vi.fn().mockResolvedValue([]),
    loadAccounts: vi.fn().mockResolvedValue([]),
    loadBudgets: vi.fn().mockResolvedValue([]),
    loadImportStats: vi.fn().mockResolvedValue([]),
    loadStorageStatus: vi.fn().mockResolvedValue(null),
    loadAppInfo: vi.fn().mockResolvedValue({ version: '1.0.0', updatesEnabled: false }),
    reportOnlineState: vi.fn().mockResolvedValue(undefined),
  };
});

describe('App.svelte SWR and real component rendering', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    cleanup();
  });

  it('renders LKG quota, stale banner, and current error simultaneously on stale provider', async () => {
    const staleProvider: ProviderSnapshot = {
      accountId: 'acc-codex-1',
      providerId: 'openai',
      productId: 'codex',
      providerName: 'OpenAI',
      productName: 'Codex',
      alias: 'Локальная история',
      connectionState: 'AuthenticationRequired',
      freshness: { kind: 'Stale', ageSeconds: 1080 },
      coverage: 'UnverifiedSemantics',
      fetchedAt: '2026-09-08T12:00:00Z',
      capabilities: ['subscriptionQuota', 'historyLocal'],
      quotas: [
        {
          poolId: 'codex-5h',
          name: '5-hour limit',
          remainingPercent: 87,
          used: 13,
          limit: 100,
          unit: 'percent',
          windowKind: 'rolling',
          source: 'Codex local',
        },
      ],
      currentError: 'AuthenticationRequired',
      lastSuccessfulRefresh: '2026-09-08T11:42:00Z',
      lastRefreshAttempt: '2026-09-08T12:00:00Z',
    };

    const snapshot: AppSnapshot = {
      mode: 'live',
      offline: false,
      providers: [staleProvider],
      overview: { today: [], yesterday: [], '7days': [], '30days': [] },
      nextRefreshAt: '2026-09-08T12:05:00Z',
      costsByPeriod: {},
    };

    vi.mocked(api.loadSnapshot).mockResolvedValue(snapshot);

    const { container } = render(App);

    // Wait for snapshot to load and elements to render
    const staleBanner = await screen.findByRole('status');
    expect(staleBanner).not.toBeNull();
    expect(staleBanner.textContent).toContain('Данные устарели');
    expect(staleBanner.textContent).toContain('⚠ Нужна авторизация');
    expect(staleBanner.textContent).toContain('Последний успех:');
    expect(staleBanner.textContent).toContain('Последняя попытка:');

    // LKG quota metric is visible simultaneously
    expect(container.textContent).toContain('5-hour limit');
    expect(container.textContent).toContain('Осталось 87%');

    // Provider heading and account label are present
    expect(container.textContent).toContain('OpenAI');
    expect(container.textContent).toContain('Codex');
    expect(container.textContent).toContain('Локальная история');

    // Fully healthy state is NOT displayed without warning
    expect(container.querySelector('.dot.Connected')).toBeNull();
    expect(container.querySelector('.stale-state')).not.toBeNull();
  });

  it('renders fresh provider without stale warning banner', async () => {
    const freshProvider: ProviderSnapshot = {
      accountId: 'acc-api-1',
      providerId: 'openai',
      productId: 'openai-api',
      providerName: 'OpenAI',
      productName: 'API',
      alias: 'Production Key',
      connectionState: 'Connected',
      freshness: { kind: 'Fresh' },
      coverage: 'Complete',
      fetchedAt: '2026-09-08T12:00:00Z',
      capabilities: ['apiCostReported', 'multiAccount'],
      quotas: [],
      currentError: undefined,
      lastSuccessfulRefresh: '2026-09-08T12:00:00Z',
      lastRefreshAttempt: '2026-09-08T12:00:00Z',
    };

    const snapshot: AppSnapshot = {
      mode: 'live',
      offline: false,
      providers: [freshProvider],
      overview: { today: [], yesterday: [], '7days': [], '30days': [] },
      nextRefreshAt: '2026-09-08T12:05:00Z',
      costsByPeriod: {},
    };

    vi.mocked(api.loadSnapshot).mockResolvedValue(snapshot);

    const { container } = render(App);

    await screen.findByRole('heading', { name: /OpenAI \/ API/ });
    expect(container.querySelector('.stale-state')).toBeNull();
    expect(container.textContent).not.toContain('Данные устарели');
    expect(container.textContent).not.toContain('Источник требует внимания');
    expect(container.textContent).toContain('Production Key');
  });

  it('renders error/empty state instead of stale old-data when provider has no LKG', async () => {
    const errorProviderNoLkg: ProviderSnapshot = {
      accountId: 'acc-err-1',
      providerId: 'openai',
      productId: 'codex',
      providerName: 'OpenAI',
      productName: 'Codex',
      alias: 'Локальная история',
      connectionState: 'AuthenticationRequired',
      freshness: { kind: 'Unknown' },
      coverage: 'UnverifiedSemantics',
      fetchedAt: '2026-09-08T12:00:00Z',
      capabilities: ['subscriptionQuota'],
      quotas: [],
      currentError: 'AuthenticationRequired',
    };

    const snapshot: AppSnapshot = {
      mode: 'live',
      offline: false,
      providers: [errorProviderNoLkg],
      overview: { today: [], yesterday: [], '7days': [], '30days': [] },
      nextRefreshAt: '2026-09-08T12:05:00Z',
      costsByPeriod: {},
    };

    vi.mocked(api.loadSnapshot).mockResolvedValue(snapshot);

    const { container } = render(App);

    await screen.findByText(/Codex/);

    // Shows attention/stale banner with error
    const staleBanner = await screen.findByRole('status');
    expect(staleBanner).not.toBeNull();
    expect(staleBanner.textContent).toContain('⚠ Нужна авторизация');

    // Shows unavailable message, NOT quota progress bar or fabricated old data
    expect(container.querySelector('.progress')).toBeNull();
    expect(container.querySelector('.unavailable')).not.toBeNull();
    expect(container.textContent).toContain('Источник не предоставляет подтверждённые quota‑метрики');
    expect(container.textContent).not.toContain('Осталось');
    expect(container.querySelector('.dot.Connected')).toBeNull();
  });

  it('renders fresh unverified semantics card with warning banner, unverified dot, and non-authoritative notice', async () => {
    const unverifiedFreshProvider: ProviderSnapshot = {
      accountId: 'acc-codex-fresh',
      providerId: 'openai',
      productId: 'codex',
      providerName: 'OpenAI',
      productName: 'Codex',
      alias: 'Локальная история',
      connectionState: 'Connected',
      freshness: { kind: 'Fresh' },
      coverage: 'UnverifiedSemantics',
      fetchedAt: '2026-09-08T12:00:00Z',
      capabilities: ['subscriptionQuota', 'historyLocal'],
      quotas: [
        {
          poolId: 'codex-5h',
          name: '5-hour limit',
          remainingPercent: 92,
          used: 8,
          limit: 100,
          unit: 'percent',
          windowKind: 'rolling',
          source: 'Codex local',
        },
      ],
      tokensToday: 1500,
    };

    const snapshot: AppSnapshot = {
      mode: 'live',
      offline: false,
      providers: [unverifiedFreshProvider],
      overview: { today: [], yesterday: [], '7days': [], '30days': [] },
      nextRefreshAt: '2026-09-08T12:05:00Z',
      costsByPeriod: {},
      excludedUnverifiedCount: { today: 1, yesterday: 0, '7days': 1, '30days': 1 },
    };

    vi.mocked(api.loadSnapshot).mockResolvedValue(snapshot);

    const { container } = render(App);

    await screen.findByRole('heading', { name: /OpenAI \/ Codex/ });

    // The card MUST NOT look like a normal trusted green card
    expect(container.querySelector('.dot.Connected')).toBeNull();
    expect(container.querySelector('.dot.unverified')).not.toBeNull();
    expect(container.textContent).toContain('Семантика не подтверждена');

    // Prominent warning banner MUST be rendered on the card
    const unverifiedBanner = container.querySelector('[data-testid="unverified-banner"]');
    expect(unverifiedBanner).not.toBeNull();
    expect(unverifiedBanner?.textContent).toContain('⚠ Неподтверждённые данные');
    expect(unverifiedBanner?.textContent).toContain('Семантика этого источника не подтверждена официальным API/документацией');

    // Token metric on the unverified card MUST be marked explicitly as unverified
    expect(container.textContent).toContain('Токены сегодня (неподтверждённые)');

    // Overview MUST display the exclusion note for unverified sources
    expect(container.textContent).toContain('⚠ Неподтверждённые источники (1) исключены из авторитарного итога');
  });

  it('renders explicit error banner when initial load fails with no LKG', async () => {
    vi.mocked(api.loadSnapshot).mockRejectedValue(new Error('backend offline'));

    render(App);

    const errorAlert = await screen.findByRole('alert');
    expect(errorAlert.textContent).toContain('Не удалось загрузить данные');
    expect(screen.getByRole('button', { name: 'Повторить' })).not.toBeNull();
  });
});
