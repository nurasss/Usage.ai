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
