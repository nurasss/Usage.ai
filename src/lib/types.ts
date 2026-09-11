export type ConnectionState =
  | 'Connected' | 'Refreshing' | 'NotConfigured' | 'AuthenticationRequired'
  | 'SessionExpired' | 'PermissionDenied' | 'InsufficientScope'
  | 'ApplicationNotRunning' | 'RateLimited' | 'Cooldown' | 'Unavailable'
  | 'UnsupportedSource' | 'ParseError' | 'NetworkError' | 'Offline' | 'UnknownError';

export type Freshness = { kind: 'Fresh' } | { kind: 'Stale'; ageSeconds: number } | { kind: 'Unknown' };
export type Coverage = 'Complete' | 'Partial' | 'LocalClientOnly' | 'UnverifiedSemantics' | 'FromConnectionTime' | 'ProviderDelayed' | 'Unknown';
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
  glyph?: string;
  color?: string;
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
  currentError?: string;
  lastSuccessfulRefresh?: string;
  lastRefreshAttempt?: string;
  quotaFallback?: boolean;
  fallbackReason?: string;
  identityConfidence?: string;
  selectedSource?: string;
}

export interface OverviewSegment { label: string; value: number; color: string; }
export type OverviewRange = 'today' | 'yesterday' | '7days' | '30days';
export type OverviewMap = Record<OverviewRange, OverviewSegment[]>;
export interface Money { amount: string; currency: string; }
export interface ProviderCost {
  accountId: string; productId: string; label: string;
  reported: Money[]; estimated: Money[];
}
export interface AppSnapshot {
  mode: 'demo' | 'live';
  offline: boolean;
  providers: ProviderSnapshot[];
  overview: OverviewMap;
  modelBreakdown?: OverviewMap;
  accountBreakdown?: OverviewMap;
  projectBreakdown?: OverviewMap;
  costsByPeriod?: Record<string, ProviderCost[]>;
  unverifiedOverview?: OverviewMap;
  excludedUnverifiedCount?: Record<string, number>;
  nextRefreshAt: string;
}

export interface AppSettings {
  launchAtLogin: boolean; refreshIntervalMinutes: number | null; menuBarMode: 'icon' | 'iconMetric';
  globalShortcut: string; theme: 'system' | 'light' | 'dark'; retentionDays: number;
  quotaWarningPercent: number; notificationsEnabled: boolean;
  quietHoursStart?: string; quietHoursEnd?: string;
  trayProfileAccountId?: string | null;
}

export interface AttemptInfo {
  source: string; status: string; safeCode?: string; finishedAt: string; latencyMs?: number;
}

export interface Diagnostics {
  provider: string; product: string; accountAlias: string; accountId?: string;
  selectedSource?: string;
  connectionState: ConnectionState; lastRefreshAttempt?: string;
  lastSuccessfulRefresh?: string; lastDataObservedAt?: string;
  freshness: Freshness; coverage: Coverage; statusClass?: string;
  connectorVersion: string; parserVersion: string; schemaFingerprint?: string;
  capabilitiesDetected: string[]; cooldownUntil?: string; lastSafeErrorCode?: string;
  warnings?: string[]; recentAttempts?: AttemptInfo[];
}

export interface AccountInfo {
  id: string; providerId: string; productId?: string; label: string; lifecycle: string;
  connectionRef?: string; enabled: boolean; customPath?: string; identityConfidence?: string;
  notificationsMuted?: boolean;
}

export type AccountModel = 'localClient' | 'apiKey' | 'discoveryOnly';

export interface ProductDescriptor {
  providerId: string; productId: string; providerName: string; productName: string;
  glyph: string; color: string; capabilities: Capability[]; accountModel: AccountModel;
  allowedHosts: string[]; allowCustomPath: boolean; needsSecret: boolean;
  requiresProcess?: boolean;
  localGlob?: string; localDirEnv?: string; localDirName?: string;
  profileDirPrefix?: string;
  diagnosticsVersion: string;
}

export interface ImportStats {
  productId: string; filesDiscovered: number; filesImported: number;
  recordsAccepted: number; malformed: number; checkpointResets: number; warnings: string[];
}

export interface ProfileCandidate {
  rootHash: string; providerId: string; productId: string; rootHint: string;
  kind: string; status: string; boundAccountId?: string;
}

export interface Budget {
  accountId: string; productId: string; currency: string; amount: string; period: string;
}

export interface StorageStatus {
  usageRecords: number; costRecords: number; accounts: number; checkpoints: number;
  lastImport?: string; dbBytes?: number;
}

export interface AppInfo { version: string; updateChannel?: string; updatesEnabled: boolean; }
