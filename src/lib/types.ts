export type ConnectionState =
  | 'Connected' | 'Refreshing' | 'NotConfigured' | 'AuthenticationRequired'
  | 'SessionExpired' | 'PermissionDenied' | 'InsufficientScope'
  | 'ApplicationNotRunning' | 'RateLimited' | 'Cooldown' | 'Unavailable'
  | 'UnsupportedSource' | 'ParseError' | 'NetworkError' | 'Offline' | 'UnknownError';

export type Freshness = { kind: 'Fresh' } | { kind: 'Stale'; ageSeconds: number } | { kind: 'Unknown' };
export type Coverage = 'Complete' | 'Partial' | 'LocalClientOnly' | 'FromConnectionTime' | 'ProviderDelayed' | 'Unknown';
export type Capability = 'subscriptionQuota' | 'quotaResetTime' | 'apiTokens' | 'apiRequests' | 'apiCostReported' | 'apiCostEstimated' | 'balance' | 'credits' | 'modelBreakdown' | 'projectBreakdown' | 'historyRemote' | 'historyLocal' | 'localSessions' | 'multiAccount';

export interface Quota {
  poolId: string;
  windowId?: string;
  name: string;
  remainingPercent?: number;
  used?: number;
  limit?: number;
  unit: string;
  resetsAt?: string;
  windowStart?: string;
  windowKind: 'fixed' | 'rolling' | 'unknown';
  source: string;
}

export interface ProviderSnapshot {
  accountId: string;
  providerId: string;
  productId: string;
  providerName: string;
  productName: string;
  alias: string;
  planLabel?: string;
  connectionState: ConnectionState;
  freshness: Freshness;
  coverage: Coverage;
  fetchedAt: string;
  observedAt?: string;
  capabilities: Capability[];
  quotas: Quota[];
  tokensToday?: number;
  reportedCostToday?: { amount: string; currency: string };
  estimatedCostToday?: { amount: string; currency: string };
  balances?: Money[];
}

export interface OverviewSegment { label: string; value: number; color: string; }
export type OverviewRange = 'today' | 'yesterday' | '7days' | '30days';
export type OverviewMap = Record<OverviewRange, OverviewSegment[]>;
export interface Money { amount: string; currency: string; }
export interface AppSnapshot {
  mode: 'demo' | 'live';
  offline: boolean;
  providers: ProviderSnapshot[];
  overview: OverviewMap;
  modelBreakdown?: OverviewMap;
  accountBreakdown?: OverviewMap;
  projectBreakdown?: OverviewMap;
  nextRefreshAt: string;
}

export interface AppSettings {
  launchAtLogin: boolean; refreshIntervalMinutes: number | null; menuBarMode: 'icon' | 'iconMetric';
  globalShortcut: string; theme: 'system' | 'light' | 'dark'; retentionDays: number;
  quotaWarningPercent: number; notificationsEnabled: boolean;
  quietHoursStart?: string; quietHoursEnd?: string;
}

export interface Diagnostics {
  provider: string; product: string; accountAlias: string;
  connectionState: ConnectionState; lastRefreshAttempt?: string;
  lastSuccessfulRefresh?: string; lastDataObservedAt?: string;
  freshness: Freshness; coverage: Coverage; statusClass?: string;
  connectorVersion: string; parserVersion: string; schemaFingerprint?: string;
  capabilitiesDetected: string[]; cooldownUntil?: string; lastSafeErrorCode?: string;
}

export interface AccountInfo {
  id: string; providerId: string; productId?: string; label: string; lifecycle: string;
  connectionRef?: string; enabled: boolean; customPath?: string;
}

export interface Budget {
  accountId: string; productId: string; currency: string; amount: string; period: string;
}

export interface StorageStatus {
  usageRecords: number; costRecords: number; accounts: number; checkpoints: number;
  lastImport?: string; dbBytes?: number;
}

export interface AppInfo { version: string; updateChannel?: string; updatesEnabled: boolean; }
