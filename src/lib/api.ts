import { invoke } from '@tauri-apps/api/core';
import { demoSnapshot } from './demo';
import type { AccountInfo, ProfileCandidate, AppInfo, AppSettings, AppSnapshot, Budget, Diagnostics, ImportStats, OverviewMap, ProductDescriptor, StorageStatus } from './types';

export const inTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const emptyMap = (): OverviewMap => ({ today: [], yesterday: [], '7days': [], '30days': [] });

function withDefaults(snapshot: AppSnapshot): AppSnapshot {
  const pick = (m?: OverviewMap): OverviewMap => ({
    today: m?.today ?? [],
    yesterday: m?.yesterday ?? [],
    '7days': (m as Record<string, unknown> | undefined)?.['7days'] as never[] ?? [],
    '30days': m?.['30days'] ?? []
  });
  return {
    ...snapshot,
    overview: pick(snapshot.overview),
    modelBreakdown: snapshot.modelBreakdown ? pick(snapshot.modelBreakdown) : emptyMap(),
    accountBreakdown: snapshot.accountBreakdown ? pick(snapshot.accountBreakdown) : emptyMap(),
    projectBreakdown: snapshot.projectBreakdown ? pick(snapshot.projectBreakdown) : emptyMap(),
    costsByPeriod: snapshot.costsByPeriod ?? {}
  };
}

// P0-3: inside the packaged app a backend/IPC failure is an explicit
// error, never a silent fallback to synthetic demo numbers. Demo data
// exists only for browser development (no Tauri runtime).
export async function loadSnapshot(): Promise<AppSnapshot> {
  if (!inTauri()) return demoSnapshot;
  return withDefaults(await invoke<AppSnapshot>('get_snapshot'));
}

export async function refreshAll(): Promise<AppSnapshot> {
  if (!inTauri()) return { ...demoSnapshot, nextRefreshAt: new Date(Date.now() + 300_000).toISOString() };
  return withDefaults(await invoke<AppSnapshot>('refresh_all'));
}

export async function exportHistory(format: 'csv' | 'json'): Promise<string> {
  if (!inTauri()) return `Демо: экспорт ${format.toUpperCase()} доступен в приложении`;
  return invoke<string>('export_history', { format });
}

export async function exportDiagnostics(): Promise<string> {
  if (!inTauri()) return 'Демо: экспорт диагностики доступен в приложении';
  return invoke<string>('export_diagnostics');
}

export async function loadDiagnostics(): Promise<Diagnostics[]> {
  if (!inTauri()) return [];
  try { return await invoke<Diagnostics[]>('get_diagnostics'); }
  catch { return []; }
}

export async function loadAccounts(): Promise<AccountInfo[]> {
  if (!inTauri()) return [];
  try { return await invoke<AccountInfo[]>('list_accounts'); }
  catch { return []; }
}

export async function removeAccount(accountId: string, deleteHistory: boolean): Promise<string> {
  if (!inTauri()) return 'Демо: управление аккаунтами доступно в приложении';
  return invoke<string>('remove_account', { accountId, deleteHistory });
}

export async function addAccount(providerId: string, productId: string, alias: string, secret?: string): Promise<AccountInfo> {
  if (!inTauri()) throw new Error('demo');
  if (productId === 'openai-api' && secret) {
    return invoke<AccountInfo>('configure_openai_admin_connection', { label: alias, secret });
  }
  return invoke<AccountInfo>('add_account', { providerId, productId, alias, secret: secret || null });
}

export async function updateAccount(accountId: string, patch: { alias?: string; enabled?: boolean; customPath?: string | null }): Promise<AccountInfo> {
  if (!inTauri()) throw new Error('demo');
  return invoke<AccountInfo>('update_account', {
    accountId,
    alias: patch.alias ?? null,
    enabled: patch.enabled ?? null,
    customPath: patch.customPath === undefined ? null : patch.customPath
  });
}

export async function testConnection(accountId: string): Promise<Diagnostics> {
  if (!inTauri()) throw new Error('demo');
  return invoke<Diagnostics>('test_connection', { accountId });
}

export async function loadBudgets(): Promise<Budget[]> {
  if (!inTauri()) return [];
  try { return await invoke<Budget[]>('list_budgets'); }
  catch { return []; }
}

export async function saveBudget(accountId: string, productId: string, currency: string, amount: string): Promise<Budget> {
  if (!inTauri()) throw new Error('demo');
  return invoke<Budget>('save_budget', { accountId, productId, currency, amount });
}

export async function deleteBudget(accountId: string, productId: string, currency: string): Promise<boolean> {
  if (!inTauri()) return false;
  return invoke<boolean>('delete_budget', { accountId, productId, currency });
}

export async function loadStorageStatus(): Promise<StorageStatus | null> {
  if (!inTauri()) return null;
  try { return await invoke<StorageStatus>('get_storage_status'); }
  catch { return null; }
}

export async function loadAppInfo(): Promise<AppInfo> {
  if (!inTauri()) return { version: '1.0.0', updatesEnabled: false };
  try { return await invoke<AppInfo>('get_app_info'); }
  catch { return { version: '1.0.0', updatesEnabled: false }; }
}

export async function loadDescriptors(): Promise<ProductDescriptor[]> {
  if (!inTauri()) return [];
  return invoke<ProductDescriptor[]>('get_descriptors');
}

export async function loadImportStats(): Promise<ImportStats[]> {
  if (!inTauri()) return [];
  try { return await invoke<ImportStats[]>('get_import_stats'); }
  catch { return []; }
}

export async function copyDiagnostics(accountId: string): Promise<string> {
  if (!inTauri()) return '';
  return invoke<string>('copy_diagnostics_text', { accountId });
}

export async function reportOnlineState(online: boolean): Promise<void> {
  if (!inTauri()) return;
  try { await invoke('report_online_state', { online }); } catch { /* observed state only */ }
}

export const defaultSettings: AppSettings = { launchAtLogin: false, refreshIntervalMinutes: 5, menuBarMode: 'icon', globalShortcut: 'Ctrl+Alt+U', theme: 'system', retentionDays: 90, quotaWarningPercent: 20, notificationsEnabled: false };
export async function loadSettings(): Promise<AppSettings> { if (!inTauri()) return defaultSettings; return invoke<AppSettings>('get_settings'); }
export async function saveSettings(settings: AppSettings): Promise<void> { if (!inTauri()) return; await invoke('save_settings', { settings }); }

export async function scanCandidates(): Promise<ProfileCandidate[]> {
  if (!inTauri()) return [];
  try { return await invoke<ProfileCandidate[]>('scan_candidates'); }
  catch { return []; }
}

export async function connectCandidate(rootHash: string, alias: string): Promise<AccountInfo> {
  if (!inTauri()) throw new Error('demo');
  return invoke<AccountInfo>('connect_candidate', { rootHash, alias });
}

export async function ignoreCandidate(rootHash: string): Promise<boolean> {
  if (!inTauri()) return false;
  try { return await invoke<boolean>('ignore_candidate', { rootHash }); }
  catch { return false; }
}
