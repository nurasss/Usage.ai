import type { ProviderSnapshot } from './types';

export type ThemeSetting = 'system' | 'light' | 'dark';

/** Account-aware stable DOM/navigation key: provider + product + account. */
export function navKey(providerId: string, productId: string, accountId: string): string {
  const safe = (value: string) => value.replace(/[^a-zA-Z0-9_-]/g, '_');
  return `${safe(providerId)}__${safe(productId)}__${safe(accountId)}`;
}

/** Apply the theme setting to the document root. `system` removes the
 * override so the OS preference (media query) applies. */
export function applyTheme(theme: ThemeSetting): void {
  if (typeof document === 'undefined') return;
  if (theme === 'system') document.documentElement.removeAttribute('data-theme');
  else document.documentElement.setAttribute('data-theme', theme);
}

/** Stale data and the latest source failure are separate UI concepts. */
export function providerPresentation(
  provider: Pick<ProviderSnapshot, 'connectionState' | 'freshness' | 'currentError'>,
): { stale: boolean; hasCurrentError: boolean; healthy: boolean } {
  const stale = provider.freshness.kind === 'Stale';
  const hasCurrentError = Boolean(provider.currentError);
  return {
    stale,
    hasCurrentError,
    healthy: !stale && !hasCurrentError && provider.connectionState === 'Connected',
  };
}

/**
 * Honest account label: never display misleading "Аккаунт 1" or generic
 * "Аккаунт" for unverified/unattributed local sources without confirmed identity.
 */
export function formatAccountLabel(alias?: string, identityConfidence?: string): string {
  const trimmed = alias?.trim() ?? '';
  const isGeneric = /^Аккаунт(\s*\d+)?$/i.test(trimmed);
  if (!trimmed || (isGeneric && (!identityConfidence || identityConfidence === 'Unknown'))) {
    return 'Локальная история';
  }
  return trimmed;
}
