import type { AppSnapshot } from './types';

const now = Date.now();
export const demoSnapshot: AppSnapshot = {
  mode: 'demo',
  offline: false,
  nextRefreshAt: new Date(now + 4 * 60_000).toISOString(),
  overview: {
    today: [
      { label: 'Codex', value: 184_200, color: '#ff7a5c' },
      { label: 'Claude Code', value: 126_800, color: '#b895ff' },
      { label: 'OpenCode Zen', value: 64_000, color: '#46c9a7' }
    ],
    yesterday: [
      { label: 'Codex', value: 224_000, color: '#ff7a5c' },
      { label: 'Claude Code', value: 89_000, color: '#b895ff' }
    ],
    '7days': [
      { label: 'Codex', value: 1_310_000, color: '#ff7a5c' },
      { label: 'Claude Code', value: 860_000, color: '#b895ff' }
    ],
    '30days': [
      { label: 'Codex', value: 3_420_000, color: '#ff7a5c' },
      { label: 'Claude Code', value: 2_180_000, color: '#b895ff' },
      { label: 'OpenCode Zen', value: 890_000, color: '#46c9a7' }
    ]
  },
  modelBreakdown: {
    today: [
      { label: 'gpt-x', value: 184_200, color: '#8ea2ff' },
      { label: 'Модель не определена', value: 126_800, color: '#8ea2ff' }
    ],
    yesterday: [{ label: 'Модель не определена', value: 313_000, color: '#8ea2ff' }],
    '7days': [{ label: 'Модель не определена', value: 1_940_000, color: '#8ea2ff' }],
    '30days': [{ label: 'Модель не определена', value: 6_490_000, color: '#8ea2ff' }]
  },
  providers: [
    {
      accountId: 'demo-codex', providerId: 'openai', productId: 'codex', providerName: 'OpenAI', productName: 'Codex', alias: 'Локальная история', planLabel: 'Plus',
      connectionState: 'Connected', freshness: { kind: 'Fresh' }, coverage: 'LocalClientOnly', fetchedAt: new Date(now - 55_000).toISOString(),
      capabilities: ['subscriptionQuota', 'quotaResetTime', 'apiTokens', 'historyLocal', 'localSessions'], tokensToday: 184_200,
      quotas: [
        { poolId: 'five-hour', name: '5-часовой лимит', remainingPercent: 68, unit: 'percent', resetsAt: new Date(now + 2.3 * 3_600_000).toISOString(), windowStart: new Date(now - 2.7 * 3_600_000).toISOString(), windowKind: 'fixed', source: 'Codex local session' },
        { poolId: 'weekly', name: 'Недельный лимит', remainingPercent: 23, unit: 'percent', resetsAt: new Date(now + 3.8 * 86_400_000).toISOString(), windowKind: 'rolling', source: 'Codex local session' }
      ]
    },
    {
      accountId: 'demo-claude', providerId: 'anthropic', productId: 'claude-code', providerName: 'Anthropic', productName: 'Claude Code', alias: 'Рабочий', planLabel: 'Max',
      connectionState: 'Connected', freshness: { kind: 'Stale', ageSeconds: 780 }, coverage: 'Partial', fetchedAt: new Date(now - 780_000).toISOString(),
      capabilities: ['subscriptionQuota', 'quotaResetTime', 'apiTokens', 'historyLocal', 'modelBreakdown'], tokensToday: 126_800,
      quotas: [{ poolId: 'session', name: 'Текущая сессия', remainingPercent: 9, unit: 'percent', resetsAt: new Date(now + 42 * 60_000).toISOString(), windowKind: 'fixed', source: 'Demo fixture' }]
    },
    {
      accountId: 'demo-openai-api', providerId: 'openai', productId: 'openai-api', providerName: 'OpenAI', productName: 'API', alias: 'Личный проект',
      connectionState: 'InsufficientScope', freshness: { kind: 'Unknown' }, coverage: 'Unknown', fetchedAt: new Date(now - 1_800_000).toISOString(),
      capabilities: ['apiTokens', 'apiRequests', 'apiCostReported', 'projectBreakdown', 'multiAccount'], reportedCostToday: { amount: '4.82', currency: 'USD' },
      quotas: []
    }
  ]
};
