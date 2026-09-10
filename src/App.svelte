<script lang="ts">
  import { onMount } from 'svelte';
  import { addAccount, connectCandidate, defaultSettings, deleteBudget, exportDiagnostics, exportHistory, ignoreCandidate, loadAccounts, loadAppInfo, loadBudgets, loadDescriptors, loadDiagnostics, loadImportStats, loadSettings, loadSnapshot, loadStorageStatus, refreshAll, removeAccount, reportOnlineState, saveBudget, saveSettings, scanCandidates, testConnection, updateAccount } from './lib/api';
  import { formatAge, formatCompact, formatCountdown, stateLabel } from './lib/format';
  import { applyTheme, formatAccountLabel, navKey, providerPresentation } from './lib/ui';
  import type { AccountInfo, AppInfo, AppSettings, AppSnapshot, Budget, Diagnostics, ImportStats, ProductDescriptor, ProfileCandidate, ProviderSnapshot, Quota, StorageStatus } from './lib/types';

  let snapshot: AppSnapshot | null = null;
  let loadError = '';
  let period: 'today' | 'yesterday' | '7days' | '30days' = 'today';
  let metric: 'tokens' | 'cost' = 'tokens';
  let breakdown: 'product' | 'model' | 'account' | 'project' = 'product';
  let refreshing = false;
  let notice = '';
  let now = Date.now();
  let selected = 'overview';
  let settings: AppSettings = { ...defaultSettings };
  let diagnostics: Diagnostics[] = [];
  let accounts: AccountInfo[] = [];
  let budgets: Budget[] = [];
  let descriptors: ProductDescriptor[] = [];
  let candidates: ProfileCandidate[] = [];
  let candidateAlias = '';
  let importStats: ImportStats[] = [];
  let storageStatus: StorageStatus | null = null;
  let appInfo: AppInfo = { version: '1.0.0', updatesEnabled: false };
  let online = typeof navigator === 'undefined' ? true : navigator.onLine;
  let providerFilter = '';
  let newAlias = '';
  let newProduct = 'openai-api';
  let newSecret = '';
  let testResult = '';
  let budgetAmount = '';
  let budgetCurrency = 'USD';
  let budgetAccount = '';

  function descriptorFor(providerId: string, productId: string): ProductDescriptor | undefined {
    return descriptors.find((d) => d.providerId === providerId && d.productId === productId);
  }

  function glyphFor(provider: ProviderSnapshot): string {
    return provider.glyph ?? descriptorFor(provider.providerId, provider.productId)?.glyph ?? '•';
  }

  function colorFor(provider: ProviderSnapshot): string {
    return provider.color ?? descriptorFor(provider.providerId, provider.productId)?.color ?? '#292929';
  }

  function hasCapability(provider: ProviderSnapshot, capability: string): boolean {
    return provider.capabilities.includes(capability as never);
  }

  onMount(() => {
    loadSnapshot().then((value) => { snapshot = value; loadError = ''; }).catch(() => { loadError = 'Не удалось загрузить данные. Проверьте подключение и нажмите «Повторить».'; });
    loadSettings().then((value) => { settings = value; applyTheme(settings.theme); });
    loadDescriptors().then((value) => descriptors = value).catch(() => {});
    loadDiagnostics().then((value) => diagnostics = value);
    loadAccounts().then((value) => accounts = value);
    scanCandidates().then((value) => candidates = value.filter((c) => c.status === 'pending')).catch(() => {});
    loadBudgets().then((value) => budgets = value);
    loadImportStats().then((value) => importStats = value);
    loadStorageStatus().then((value) => storageStatus = value);
    loadAppInfo().then((value) => appInfo = value);
    void reportOnlineState(online);
    let unlisten: (() => void) | undefined;
    if ('__TAURI_INTERNALS__' in window) import('@tauri-apps/api/event').then(({ listen }) => listen('open-settings', () => selected = 'settings')).then((stop) => unlisten = stop);
    const timer = window.setInterval(() => now = Date.now(), 1_000);
    const onOnline = () => { online = true; void reportOnlineState(true); };
    const onOffline = () => { online = false; void reportOnlineState(false); };
    window.addEventListener('online', onOnline);
    window.addEventListener('offline', onOffline);
    // After sleep/wake the page becomes visible again: run a single current
    // refresh instead of replaying a missed queue.
    const onVisible = () => { if (document.visibilityState === 'visible' && selected !== 'settings') void refresh(); };
    document.addEventListener('visibilitychange', onVisible);
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') window.close();
      if (event.metaKey && event.key.toLowerCase() === 'r') { event.preventDefault(); void refresh(); }
      if (event.metaKey && event.key === ',') { event.preventDefault(); selected = 'settings'; }
    };
    window.addEventListener('keydown', key);
    return () => { window.clearInterval(timer); window.removeEventListener('keydown', key); window.removeEventListener('online', onOnline); window.removeEventListener('offline', onOffline); document.removeEventListener('visibilitychange', onVisible); unlisten?.(); };
  });

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      snapshot = await refreshAll();
      loadError = '';
      diagnostics = await loadDiagnostics();
      accounts = await loadAccounts();
      candidates = await scanCandidates().catch(() => candidates);
      budgets = await loadBudgets();
      importStats = await loadImportStats();
      storageStatus = await loadStorageStatus();
      notice = 'Данные обновлены';
    }
    catch { loadError = 'Не удалось обновить данные.'; notice = 'Не удалось обновить данные'; }
    finally { refreshing = false; window.setTimeout(() => notice = '', 2400); }
  }

  async function exportData(format: 'csv' | 'json') {
    try { notice = await exportHistory(format); }
    catch { notice = 'Экспорт отменён'; }
    window.setTimeout(() => notice = '', 3000);
  }

  async function exportDiag() {
    try { notice = await exportDiagnostics(); }
    catch { notice = 'Экспорт отменён'; }
    window.setTimeout(() => notice = '', 3000);
  }

  async function dropAccount(id: string, history: boolean) {
    notice = await removeAccount(id, history);
    accounts = await loadAccounts();
    budgets = await loadBudgets();
    candidates = await scanCandidates().catch(() => candidates);
    try { snapshot = await loadSnapshot(); } catch { /* keep last good view */ }
    window.setTimeout(() => notice = '', 3000);
  }

  async function rescanCandidates() {
    try {
      candidates = (await scanCandidates()).filter((c) => c.status === 'pending');
      notice = candidates.length ? `Найдено профилей: ${candidates.length}` : 'Новых профилей нет';
    } catch { notice = 'Не удалось сканировать профили'; }
    window.setTimeout(() => notice = '', 3000);
  }

  async function connectProfile(candidate: ProfileCandidate) {
    try {
      await connectCandidate(candidate.rootHash, candidateAlias.trim() || candidate.rootHint);
      candidateAlias = '';
      accounts = await loadAccounts();
      candidates = await scanCandidates().catch(() => candidates);
      snapshot = await refreshAll();
      notice = 'Профиль подключён';
    } catch { notice = 'Не удалось подключить: корень уже привязан или недоступен'; }
    window.setTimeout(() => notice = '', 3000);
  }

  async function ignoreProfile(candidate: ProfileCandidate) {
    await ignoreCandidate(candidate.rootHash);
    candidates = candidates.filter((c) => c.rootHash !== candidate.rootHash);
  }

  function productIdOf(selection: string): [string, string] {
    const found = descriptors.find((d) => d.productId === selection);
    if (found) return [found.providerId, found.productId];
    return ['', selection];
  }

  async function createAccount() {
    try {
      const [providerId, productId] = productIdOf(newProduct);
      const descriptor = descriptorFor(providerId, productId);
      const defaultAlias = descriptor?.accountModel === 'localClient' ? 'Локальная история' : 'Основной';
      await addAccount(providerId, productId, newAlias || defaultAlias, newSecret || undefined);
      newAlias = ''; newSecret = '';
      accounts = await loadAccounts();
      notice = 'Аккаунт добавлен';
    } catch { notice = 'Не удалось добавить: проверьте название и ключ'; }
    window.setTimeout(() => notice = '', 3000);
  }

  async function toggleAccount(account: AccountInfo) {
    try { await updateAccount(account.id, { enabled: !account.enabled }); accounts = await loadAccounts(); }
    catch { notice = 'Не удалось обновить аккаунт'; window.setTimeout(() => notice = '', 2500); }
  }

  async function probeAccount(id: string) {    try {
      const result = await testConnection(id);
      testResult = `${result.product}: ${stateLabel(result.connectionState)}`;
    } catch { testResult = 'Проверка не удалась'; }
    window.setTimeout(() => testResult = '', 4000);
  }

  async function createBudget() {
    try {
      const account = accounts.find((a) => a.id === budgetAccount) ?? accounts[0];
      if (!account) { notice = 'Сначала добавьте аккаунт'; window.setTimeout(() => notice = '', 2500); return; }
      await saveBudget(account.id, account.productId ?? 'openai-api', budgetCurrency, budgetAmount);
      budgets = await loadBudgets();
      budgetAmount = '';
      notice = 'Бюджет сохранён';
    } catch { notice = 'Некорректная сумма или валюта'; }
    window.setTimeout(() => notice = '', 3000);
  }

  async function removeBudget(budget: Budget) {
    await deleteBudget(budget.accountId, budget.productId, budget.currency);
    budgets = await loadBudgets();
  }

  async function persistSettings() {
    try { await saveSettings(settings); applyTheme(settings.theme); notice = 'Настройки сохранены'; }
    catch { notice = 'Не удалось сохранить: shortcut занят или настройка недоступна'; }
    window.setTimeout(() => notice = '', 3000);
  }

  function scrollToKey(key: string) {
    selected = key;
    document.getElementById(key)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  function severity(quota: Quota): string {
    if (quota.remainingPercent === undefined) return 'neutral';
    if (quota.remainingPercent <= 0) return 'exhausted';
    if (quota.remainingPercent <= 10) return 'critical';
    if (quota.remainingPercent <= 25) return 'warning';
    return 'normal';
  }

  function modelLabel(value: string | undefined): string {
    return value && value.trim() ? value : 'Модель не определена';
  }

  function providerKey(provider: ProviderSnapshot): string {
    return navKey(provider.providerId, provider.productId, provider.accountId);
  }

  $: offline = snapshot ? (snapshot.offline || !online) : !online;
  $: periodCosts = snapshot?.costsByPeriod?.[period] ?? [];
  $: costRows = periodCosts.length ? periodCosts : [];
  $: breakdownSegments = snapshot ? (breakdown === 'product' ? (snapshot.overview[period] ?? []) : breakdown === 'model' ? (snapshot.modelBreakdown?.[period] ?? []) : breakdown === 'account' ? (snapshot.accountBreakdown?.[period] ?? []) : (snapshot.projectBreakdown?.[period] ?? [])) : [];
  $: breakdownTotal = breakdownSegments.reduce((sum, item) => sum + item.value, 0);
  $: visibleProviders = snapshot ? snapshot.providers.filter((p) => !providerFilter.trim() || `${p.providerName} ${p.productName} ${p.alias}`.toLowerCase().includes(providerFilter.trim().toLowerCase())) : [];
  $: connectableDescriptors = descriptors.filter((d) => d.accountModel !== 'discoveryOnly');
  $: blockedDescriptors = descriptors.filter((d) => d.accountModel === 'discoveryOnly');
  $: selectedDescriptor = descriptors.find((d) => d.productId === newProduct);
</script>

<svelte:head><title>Usage.ai</title></svelte:head>

<div class="app-shell" class:loading={!snapshot && !loadError}>
  <aside aria-label="Навигация">
    <button class:active={selected === 'overview'} on:click={() => { selected = 'overview'; document.getElementById('overview')?.scrollIntoView({ behavior: 'smooth' }); }} aria-label="Обзор" title="Обзор">⌁</button>
    <div class="nav-rule"></div>
    {#if snapshot}
      {#each snapshot.providers as provider (providerKey(provider))}
        <button class:active={selected === providerKey(provider)} on:click={() => scrollToKey(providerKey(provider))} aria-label={`${provider.providerName} ${provider.productName} ${provider.alias}`} title={`${provider.providerName} · ${provider.productName} · ${provider.alias}`}>{glyphFor(provider)}</button>
      {/each}
    {/if}
    <div class="nav-spacer"></div>
    <button on:click={() => exportData('csv')} aria-label="Экспорт CSV" title="Экспорт CSV">⇩</button>
    <button class:active={selected === 'settings'} on:click={() => selected = selected === 'settings' ? 'overview' : 'settings'} aria-label="Настройки" title="Настройки">⚙</button>
  </aside>

  <main>
    {#if loadError && !snapshot}
      <section class="settings-page" role="alert">
        <div class="eyebrow">USAGE.AI</div>
        <h1>Данные недоступны</h1>
        <p class="muted">{loadError}</p>
        <div class="button-row"><button class="save" on:click={refresh} disabled={refreshing}>{refreshing ? 'Повтор…' : 'Повторить'}</button></div>
      </section>
    {:else if !snapshot}
      <div class="skeleton-wrap" aria-label="Загрузка"><div></div><div></div><div></div></div>
    {:else if selected === 'settings'}
      <section class="settings-page">
        <div class="eyebrow">USAGE.AI</div>
        <h1>Настройки</h1>
        <div class="setting-group">
          <h2>Основные</h2>
          <label><span>Запускать при входе</span><input type="checkbox" bind:checked={settings.launchAtLogin} /></label>
          <label><span>Интервал обновления</span><select aria-label="Интервал обновления" bind:value={settings.refreshIntervalMinutes}><option value={1}>1 минута</option><option value={5}>5 минут</option><option value={10}>10 минут</option><option value={15}>15 минут</option><option value={30}>30 минут</option><option value={60}>60 минут</option><option value={null}>Вручную</option></select></label>
          <label><span>Строка меню</span><select aria-label="Режим строки меню" bind:value={settings.menuBarMode}><option value="icon">Только значок</option><option value="iconMetric">Значок + метрика</option></select></label>
          <label><span>Оформление</span><select aria-label="Оформление" bind:value={settings.theme} on:change={() => applyTheme(settings.theme)}><option value="system">Системное</option><option value="light">Светлое</option><option value="dark">Тёмное</option></select></label>
          <label><span>Глобальная клавиша</span><input class="shortcut-input" aria-label="Глобальная клавиша" bind:value={settings.globalShortcut} /></label>
        </div>
        <div class="setting-group">
          <h2>Данные</h2>
          <label><span>Хранить историю</span><select aria-label="Срок хранения" bind:value={settings.retentionDays}><option value={90}>90 дней</option><option value={30}>30 дней</option><option value={365}>1 год</option></select></label>
          {#if storageStatus}
            <dl><div><dt>Записей usage</dt><dd>{storageStatus.usageRecords}</dd></div><div><dt>Записей cost</dt><dd>{storageStatus.costRecords}</dd></div><div><dt>Аккаунтов</dt><dd>{storageStatus.accounts}</dd></div><div><dt>Импорт</dt><dd>{storageStatus.checkpoints} файлов · {storageStatus.lastImport ?? '—'}</dd></div><div><dt>База</dt><dd>{storageStatus.dbBytes ? `${(storageStatus.dbBytes / 1024).toFixed(0)} КБ` : '—'}</dd></div></dl>
          {/if}
          {#if importStats.length}
            <dl>{#each importStats as stat}<div><dt>{stat.productId}</dt><dd>{stat.filesImported}/{stat.filesDiscovered} файлов · {stat.recordsAccepted} записей · ошибок: {stat.malformed} · сбросов: {stat.checkpointResets}</dd></div>{/each}</dl>
          {/if}
          <div class="button-row"><button class="secondary" on:click={() => exportData('csv')}>Экспорт CSV</button><button class="secondary" on:click={() => exportData('json')}>Экспорт JSON</button><button class="secondary" on:click={exportDiag}>Диагностика</button></div>
        </div>
        <div class="setting-group">
          <h2>Уведомления</h2>
          <label><span>Уведомления включены</span><input type="checkbox" bind:checked={settings.notificationsEnabled} /></label>
          <label><span>Порог остатка</span><input class="number-input" type="number" min="0" max="100" bind:value={settings.quotaWarningPercent} aria-label="Порог остатка" /></label>
          <label><span>Тихие часы с</span><input class="shortcut-input" type="time" aria-label="Тихие часы с" bind:value={settings.quietHoursStart} /></label>
          <label><span>Тихие часы до</span><input class="shortcut-input" type="time" aria-label="Тихие часы до" bind:value={settings.quietHoursEnd} /></label>
        </div>
        <div class="setting-group">
          <h2>Бюджеты</h2>
          {#if !budgets.length}<p class="muted">Бюджетов нет. Фактические расходы сравниваются помесячно, валюты не конвертируются.</p>
          {:else}
            {#each budgets as budget}
              <label><span>{budget.productId} · {budget.amount} {budget.currency}</span><button class="secondary" on:click={() => removeBudget(budget)}>Удалить</button></label>
            {/each}
          {/if}
          <label><span>Аккаунт</span><select aria-label="Аккаунт бюджета" bind:value={budgetAccount}>{#each accounts as account}<option value={account.id}>{formatAccountLabel(account.label, account.identityConfidence)} · {account.productId ?? account.providerId}</option>{/each}</select></label>
          <label><span>Сумма</span><input class="number-input" type="number" min="1" step="1" aria-label="Сумма бюджета" bind:value={budgetAmount} /></label>
          <label><span>Валюта</span><input class="shortcut-input" aria-label="Валюта бюджета" bind:value={budgetCurrency} /></label>
          <div class="button-row"><button class="secondary" on:click={createBudget}>Сохранить бюджет</button></div>
        </div>
        <div class="setting-group">
          <h2>Обнаруженные профили</h2>
          <div class="button-row"><button class="secondary" on:click={rescanCandidates}>Сканировать</button></div>
          {#if !candidates.filter((c) => c.status === 'pending').length}
            <p class="muted">Новых профилей нет. Сканируются только известные места: стандартный корень, `~/.codex-*` / `~/.claude-*`, добавленные вручную пути.</p>
          {:else}
            {#each candidates.filter((c) => c.status === 'pending') as candidate}
              <label><span>{candidate.rootHint} · {candidate.kind}{candidate.boundAccountId ? ' · уже привязан' : ''}</span></label>
              <div class="button-row"><input class="shortcut-input" aria-label="Название профиля" bind:value={candidateAlias} placeholder={candidate.rootHint} /><button class="secondary" on:click={() => connectProfile(candidate)} disabled={!!candidate.boundAccountId}>Подключить</button><button class="secondary" on:click={() => ignoreProfile(candidate)}>Игнорировать</button></div>
            {/each}
          {/if}
        </div>
        <div class="setting-group">
          <h2>Аккаунты ({accounts.length})</h2>
          <label><span>Название</span><input class="shortcut-input" aria-label="Название аккаунта" bind:value={newAlias} placeholder="Личный" /></label>
          <label><span>Продукт</span><select aria-label="Продукт аккаунта" bind:value={newProduct}>{#each connectableDescriptors as option}<option value={option.productId}>{option.providerName} {option.productName}{option.needsSecret ? ' (ключ)' : ' (локально)'}</option>{/each}{#if !connectableDescriptors.length}<option value="" disabled>Каталог продуктов недоступен</option>{/if}</select></label>
          {#if selectedDescriptor?.needsSecret}<label><span>API-ключ</span><input class="shortcut-input" type="password" aria-label="API-ключ" bind:value={newSecret} placeholder="sk-…" /></label>{/if}
          <div class="button-row"><button class="secondary" on:click={createAccount}>Добавить</button></div>
          {#if blockedDescriptors.length}<p class="muted">Discovery: {blockedDescriptors.map((d) => `${d.providerName} ${d.productName}`).join(', ')} — источник не подтверждён, подключение недоступно.</p>{/if}
          {#if testResult}<p class="muted">{testResult}</p>{/if}
          {#if !accounts.length}
            <p class="muted">Подключённые аккаунты появятся здесь после первого обновления.</p>
          {:else}
            {#each accounts as account}
              <div class="account-line"><span>{formatAccountLabel(account.label, account.identityConfidence)} · {account.productId ?? account.providerId} · {account.enabled ? account.lifecycle : 'выключен'}{account.identityConfidence ? ` · ${account.identityConfidence}` : ''}</span></div>
              <div class="button-row"><button class="secondary" on:click={() => toggleAccount(account)}>{account.enabled ? 'Выключить' : 'Включить'}</button><button class="secondary" on:click={() => probeAccount(account.id)}>Проверить</button><button class="secondary" on:click={() => dropAccount(account.id, false)}>Архив</button><button class="secondary" on:click={() => dropAccount(account.id, true)}>Удалить</button></div>
            {/each}
          {/if}
        </div>
        <div class="setting-group">
          <h2>Диагностика ({diagnostics.length})</h2>
          {#if !diagnostics.length}
            <p class="muted">Нет данных. Обновите панель, затем вернитесь сюда.</p>
          {:else}
            {#each diagnostics as item}
              <details><summary>{item.provider} / {item.product} · {stateLabel(item.connectionState)}</summary><dl><div><dt>Аккаунт</dt><dd>{item.accountAlias}</dd></div><div><dt>Источник</dt><dd>{item.selectedSource ?? '—'}</dd></div><div><dt>Покрытие</dt><dd>{item.coverage}</dd></div><div><dt>Схема</dt><dd>{item.schemaFingerprint ?? '—'}</dd></div><div><dt>Коннектор</dt><dd>{item.connectorVersion} · {item.parserVersion}</dd></div><div><dt>Ошибка</dt><dd>{item.lastSafeErrorCode ?? '—'}</dd></div><div><dt>Кулдаун</dt><dd>{item.cooldownUntil ?? '—'}</dd></div>{#if item.warnings?.length}<div><dt>Предупреждения</dt><dd>{item.warnings.join('; ')}</dd></div>{/if}{#if item.recentAttempts?.length}<div><dt>Попытки</dt><dd>{item.recentAttempts.map((a) => `${a.source}:${a.status}`).join(', ')}</dd></div>{/if}</dl></details>
            {/each}
          {/if}
        </div>
        <div class="setting-group">
          <h2>Обновления</h2>
          <p class="muted">Версия {appInfo.version} · канал не настроен, автоматические обновления отключены до настройки подписанного релизного канала.</p>
        </div>
        <div class="button-row"><button class="back" on:click={() => selected = 'overview'}>← Вернуться к обзору</button><button class="save" on:click={persistSettings}>Сохранить</button></div>
      </section>
    {:else}
      <div class="scroll-content">
        <header id="overview">
          <div><div class="eyebrow">USAGE.AI</div><h1>Добрый вечер</h1></div>
          <button class="refresh" class:spinning={refreshing} on:click={refresh} aria-label="Обновить данные" title="Обновить (⌘R)">↻</button>
        </header>

        {#if snapshot.mode === 'demo'}<div class="demo-banner"><span>ДЕМО</span> Показаны демонстрационные данные — не реальные лимиты</div>{/if}
        {#if loadError}<div class="offline-banner" role="alert">{loadError}</div>{/if}
        {#if offline}<div class="offline-banner">Офлайн · показан последний сохранённый снимок</div>{/if}

        <section class="overview-card" aria-labelledby="overview-title">
          <div class="card-title-row"><h2 id="overview-title">Быстрый обзор</h2><div class="metric-switch"><button class:chosen={metric === 'tokens'} on:click={() => metric = 'tokens'}>Токены</button><button class:chosen={metric === 'cost'} on:click={() => metric = 'cost'}>Расходы</button></div></div>
          <div class="period-tabs" role="tablist"><button class:chosen={period === 'today'} on:click={() => period = 'today'}>Сегодня</button><button class:chosen={period === 'yesterday'} on:click={() => period = 'yesterday'}>Вчера</button><button class:chosen={period === '7days'} on:click={() => period = '7days'}>7 дней</button><button class:chosen={period === '30days'} on:click={() => period = '30days'}>30 дней</button></div>
          {#if metric === 'tokens'}
            <div class="period-tabs" role="tablist"><button class:chosen={breakdown === 'product'} on:click={() => breakdown = 'product'}>Продукты</button><button class:chosen={breakdown === 'model'} on:click={() => breakdown = 'model'}>Модели</button><button class:chosen={breakdown === 'account'} on:click={() => breakdown = 'account'}>Аккаунты</button><button class:chosen={breakdown === 'project'} on:click={() => breakdown = 'project'}>Проекты</button></div>
            {#if !breakdownSegments.length || breakdownTotal <= 0}
              <div class="empty-metric"><strong>Нет данных</strong><span>Источник пока не предоставил подтверждённые токены за период</span></div>
            {:else}
              <div class="total"><strong>{formatCompact(breakdownTotal)}</strong><span>токенов</span></div>
              <div class="stacked" aria-label={`Всего ${breakdownTotal} токенов`}>{#each breakdownSegments as segment}<div style={`width:${breakdownTotal > 0 ? segment.value / breakdownTotal * 100 : 0}%;background:${segment.color}`} title={`${segment.label}: ${segment.value.toLocaleString('ru-RU')}`}></div>{/each}</div>
              <div class="legend">{#each breakdownSegments as segment}<span><i style={`background:${segment.color}`}></i>{modelLabel(segment.label)} <b>{formatCompact(segment.value)}</b></span>{/each}</div>
            {/if}
          {:else if !costRows.length}
            <div class="empty-metric"><strong>Нет данных</strong><span>Ни один источник не предоставил подтверждённые расходы за период. Оценка показывается только с меткой «Оценка».</span></div>
          {:else}
            {#each costRows as row}
              {#each row.reported as money}<div class="cost-row"><span>Фактические · {row.label}</span><strong>{money.currency} {money.amount}</strong></div>{/each}
              {#each row.estimated as money}<div class="cost-row"><span>Оценка · {row.label}</span><strong>{money.currency} {money.amount}</strong></div>{/each}
            {/each}
          {/if}
          {#if (snapshot?.excludedUnverifiedCount?.[period] ?? 0) > 0}
            <div class="unverified-overview-note" role="note">
              <span>⚠ Неподтверждённые источники ({snapshot?.excludedUnverifiedCount?.[period] ?? 0}) исключены из авторитарного итога</span>
            </div>
          {/if}
        </section>

        <div class="section-label">ПРОДУКТЫ</div>
        <input class="filter-input" aria-label="Фильтр продуктов" placeholder="Фильтр: название или аккаунт…" bind:value={providerFilter} />
        {#each visibleProviders as provider (providerKey(provider))}
          <section class="provider-card" id={providerKey(provider)}>
            <div class="provider-head">
              <div class="brand" style={`background:${colorFor(provider)}`}>{glyphFor(provider)}</div>
              <div class="provider-name"><h2>{provider.providerName} <span>/ {provider.productName}</span></h2><p>{formatAccountLabel(provider.alias)} · <i class={`dot ${provider.coverage === 'UnverifiedSemantics' ? 'unverified' : provider.connectionState}`}></i>{provider.coverage === 'UnverifiedSemantics' ? 'Семантика не подтверждена' : stateLabel(provider.connectionState)}</p></div>
              {#if provider.planLabel}<span class="plan">{provider.planLabel}</span>{/if}
            </div>
            {#if provider.coverage === 'UnverifiedSemantics'}
              <div class="unverified-banner" data-testid="unverified-banner">
                <strong>⚠ Неподтверждённые данные</strong>
                <span>Семантика этого источника не подтверждена официальным API/документацией. Данные исключены из сводных итогов.</span>
              </div>
            {/if}
            {#if providerPresentation(provider).stale || providerPresentation(provider).hasCurrentError}
              <div class="stale-state" role="status">
                <strong>{providerPresentation(provider).stale ? 'Данные устарели' : 'Источник требует внимания'}</strong>
                {#if provider.currentError}<span>⚠ {stateLabel(provider.connectionState)}</span>{/if}
                {#if provider.lastSuccessfulRefresh}<span>Последний успех: {formatAge(provider.lastSuccessfulRefresh, now)}</span>{/if}
                {#if provider.lastRefreshAttempt}<span>Последняя попытка: {formatAge(provider.lastRefreshAttempt, now)}</span>{/if}
              </div>
            {/if}
            {#if hasCapability(provider, 'subscriptionQuota') && provider.quotas.length}
              {#each provider.quotas as quota}
                <div class="quota">
                  <div class="quota-top"><span>{quota.name}</span><strong>{quota.remainingPercent === undefined ? 'Нет данных' : `Осталось ${quota.remainingPercent}%`}</strong></div>
                  <div class={`progress ${severity(quota)}`} title={`Источник: ${quota.source} · Покрытие: ${provider.coverage}`}><div style={`width:${quota.remainingPercent ?? 0}%`}></div>{#if quota.windowStart && quota.resetsAt && quota.windowKind === 'fixed' && provider.freshness.kind === 'Fresh'}<i class="pace" style={`left:${Math.min(100, Math.max(0, (new Date(quota.resetsAt).getTime() - now) / (new Date(quota.resetsAt).getTime() - new Date(quota.windowStart).getTime()) * 100))}%`}></i>{/if}</div>
                  <div class="quota-meta"><span>{quota.resetsAt ? formatCountdown(quota.resetsAt, now) : 'Время сброса неизвестно'}</span><span>{formatAge(provider.fetchedAt, now)}{provider.freshness.kind === 'Stale' ? ' · устарело' : ''}</span></div>
                </div>
              {/each}
            {:else if hasCapability(provider, 'historyLocal') || hasCapability(provider, 'historyRemote')}
              <div class="unavailable"><strong>{stateLabel(provider.connectionState)}</strong><span>История доступна, quota этот источник не предоставляет. Нулевое значение не подставлено.</span></div>
            {:else}
              <div class="unavailable"><strong>{stateLabel(provider.connectionState)}</strong><span>Источник не предоставляет подтверждённые quota‑метрики. Нулевое значение не подставлено.</span></div>
            {/if}
            {#if provider.tokensToday}<div class="cost-row"><span>{provider.coverage === 'UnverifiedSemantics' ? 'Токены сегодня (неподтверждённые)' : 'Токены сегодня'}</span><strong>{provider.tokensToday.toLocaleString('ru-RU')}</strong></div>{/if}
            {#if provider.reportedCostToday}<div class="cost-row"><span>Фактические расходы сегодня</span><strong>{provider.reportedCostToday.currency} {provider.reportedCostToday.amount}</strong></div>{/if}
            {#if provider.estimatedCostToday}<div class="cost-row"><span>Оценка расходов сегодня</span><strong>{provider.estimatedCostToday.currency} {provider.estimatedCostToday.amount}</strong></div>{/if}
            {#if provider.balances && provider.balances.length}
              {#each provider.balances as balance}
                <div class="cost-row"><span>Баланс</span><strong>{balance.currency} {balance.amount}</strong></div>
              {/each}
            {/if}
            <details><summary>Источник и диагностика</summary><dl><div><dt>Покрытие</dt><dd>{provider.coverage}</dd></div><div><dt>Состояние</dt><dd>{provider.connectionState}</dd></div><div><dt>Возможности</dt><dd>{provider.capabilities.join(', ') || 'не определены'}</dd></div><div><dt>Наблюдение</dt><dd>{provider.observedAt ?? 'неизвестно'}</dd></div></dl></details>
          </section>
        {/each}
      </div>
    {/if}

    {#if snapshot && selected !== 'settings'}
      <footer><span class="update-state">●</span><span>Версия {appInfo.version}</span><button on:click={refresh} disabled={refreshing}>{refreshing ? 'Обновление…' : snapshot.nextRefreshAt ? `Следующее обновление ${formatCountdown(snapshot.nextRefreshAt, now).replace('Сброс ', '').toLowerCase()}` : 'Обновление вручную'}</button></footer>
    {/if}
  </main>
  {#if notice}<div class="toast" role="status">{notice}</div>{/if}
</div>

<style>
  .muted { font-size: 10px; color: var(--muted); }
  .account-line { min-height: 30px; display: flex; align-items: center; font-size: 10px; border-top: 1px solid #f0eee9; }
  .filter-input { width: 100%; border: 1px solid var(--line); background: var(--card); color: inherit; border-radius: 10px; padding: 7px 10px; font-size: 10px; margin-bottom: 10px; }
</style>
