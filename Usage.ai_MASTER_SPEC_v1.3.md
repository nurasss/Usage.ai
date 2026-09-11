# Usage.ai — Master Technical Specification v1.3

**Версия документа:** 1.3  
**Дата:** 2026-09-10  
**Статус:** Implementation-ready draft  
**Целевая версия продукта:** Usage.ai v1.3  
**Платформа:** macOS  
**Первичная архитектура:** Tauri 2 + Rust + Svelte 5 / TypeScript + SQLite  
**Primary target:** `x86_64-apple-darwin`  
**Backend Usage.ai:** отсутствует  
**Telemetry:** отсутствует  

---

## 1. Назначение v1.3

Usage.ai v1.3 — первый функциональный релиз после стабилизации P0, в котором приложение должно перейти от локального прототипа мониторинга истории к полноценному ежедневному monitor-приложению для AI-подписок.

Главная цель релиза:

> Пользователь открывает Usage.ai и сразу видит реальные subscription quota Codex и Claude по каждому локальному профилю, историю использования, источник каждого значения, актуальность данных и безопасные уведомления — без расходования inference quota, без ручного ввода секретов и без смешивания аккаунтов.

v1.3 должен превратить уже существующую P0-архитектуру в устойчивую продуктовую основу для дальнейшего расширения провайдеров.

---

## 2. Основные принципы

Вся реализация v1.3 обязана сохранять следующие инварианты.

### 2.1. Local-first

Usage.ai не использует собственный облачный backend.

Все данные:

- читаются локально или напрямую у provider-owned endpoint;
- нормализуются локально;
- хранятся локально;
- агрегируются локально;
- не передаются Usage.ai-серверу.

### 2.2. Read-only ownership

Usage.ai не должен модифицировать credential/session/auth store другого приложения.

Запрещено без отдельного будущего решения:

- перезаписывать Codex `auth.json`;
- обновлять OAuth refresh token и записывать его обратно в Claude storage;
- изменять Claude Code credentials;
- изменять локальные session logs;
- менять конфигурацию внешнего CLI/IDE;
- импортировать credential в собственный permanent secret store без явного пользователя.

### 2.3. Monitoring must not consume inference quota

Ни один refresh Usage.ai не должен выполнять обычный model inference request ради проверки лимита.

Разрешены только:

- provider-owned usage/quota endpoints;
- локальные CLI-команды, которые сами являются usage/status-командами;
- чтение локальной истории;
- локальные IPC/RPC usage endpoints;
- credential/profile metadata reads.

### 2.4. Missing is not zero

Если значение невозможно определить, оно отображается как неизвестное/недоступное.

Запрещено:

```text
missing data -> 0%
missing quota -> exhausted
missing history -> 0 tokens
unknown account -> default account
```

### 2.5. Source confidence is first-class data

Каждое значение Usage.ai должно иметь объяснимое происхождение.

Минимально различаются:

- connection/source health;
- freshness;
- semantic coverage;
- identity confidence;
- strategy/source kind;
- last success;
- current error.

`Connected` не означает автоматически `Verified`.

### 2.6. Unverified data never becomes authoritative by omission

`Coverage::UnverifiedSemantics`, `Coverage::Unknown` и аналогичные non-authoritative состояния:

- могут быть видимы пользователю;
- должны иметь явное предупреждение;
- не участвуют в authoritative overview по умолчанию;
- не выигрывают tray metric selection;
- не создают authoritative quota notifications;
- не создают authoritative usage/budget alerts.

### 2.7. Account isolation is mandatory

Нельзя смешивать данные двух локальных профилей или аккаунтов только потому, что:

- provider совпадает;
- display name совпадает;
- путь выглядит похожим;
- один profile является default;
- credential другого профиля доступен.

---

## 3. Entry Gate v1.3

Реализация provider expansion v1.3 начинается только после формального закрытия P0 stabilization baseline.

Перед началом `V13-01` необходимо:

1. Завершить финальный P0-05 Codex identity/evidence вопрос.
2. Зафиксировать P0-06 Claude semantics как PASS.
3. Иметь независимый итоговый P0 verdict без известных code-level integrity/security blockers.
4. Закоммитить накопленные remediation изменения.
5. Получить чистую воспроизводимую baseline revision.
6. Прогнать полный verification suite.

Рекомендуемый baseline tag:

```text
v1.2-p0
```

или другой эквивалентный internal milestone.

До выполнения Entry Gate новые production providers не добавляются.

---

## 4. Scope v1.3

### 4.1. Входит в v1.3

- Codex local history — сохранение и финализация evidence-validated semantics.
- Claude local history — сохранение текущей evidence-validated реализации.
- OpenAI API usage/cost — сохранение и regression protection.
- Codex subscription quota — новый production source.
- Claude subscription quota — новый production source.
- Multi-profile Codex.
- Multi-profile Claude.
- Per-profile quota/history identity separation.
- Source Fidelity / confidence UI.
- Diagnostics v2.
- Adaptive refresh policy.
- Per-profile tray metric selection.
- Per-profile notifications.
- Release packaging/smoke validation для Intel macOS.

### 4.2. Не входит в v1.3

Следующие интеграции остаются замороженными:

- Antigravity production integration;
- Z.ai / GLM;
- ZCode-specific provider expansion;
- OpenCode Local;
- OpenCode Go;
- OpenCode Zen;
- ChatGPT consumer как отдельный provider;
- `claude.ai` browser session/cookies;
- Gemini consumer/API;
- Cursor;
- GitHub Copilot;
- Grok;
- Ollama;
- Windsurf;
- cloud sync;
- Usage.ai backend;
- Windows/Linux production builds;
- браузерный cookie scraping;
- provider auto-login;
- credential mutation.

Эти источники могут исследоваться только как Discovery backlog без production registry activation.

---

## 5. Технологический стек

Стек v1.3 не меняется.

### 5.1. Desktop shell

- Tauri 2
- Rust
- macOS menu-bar application
- NSPanel/AppKit bridge при необходимости

### 5.2. Frontend

- Svelte 5
- TypeScript
- Vite
- CSS variables / lightweight custom UI

### 5.3. Runtime/storage

- Tokio
- SQLite
- `rust_decimal` для денежных значений
- `serde` / arbitrary precision JSON для exact money paths
- scoped host abstractions для файлов, HTTP, Keychain, process/PTY

### 5.4. Целевая платформа

Основной релизный target:

```text
x86_64-apple-darwin
```

Apple Silicon не должен быть архитектурно заблокирован, но не является главным release gate v1.3.

---

## 6. Нормативная архитектура

v1.3 обязана сохранять текущую архитектурную модель:

```text
ProviderDescriptor
        ↓
ordered FetchStrategy[]
        ↓
Connector / source access
        ↓
Parser
        ↓
Mapper / normalized domain model
        ↓
FetchOutcome
        ↓
usage-runtime
        ↓
usage-storage
        ↓
Tauri DTO / Svelte UI
```

Provider не получает прямой доступ ко всей системе.

### 6.1. Host isolation

Provider работает только через scoped `ProviderHostFacade`.

Разрешённые capability:

```text
FilesHost
HTTPHost
KeychainHost
ProcessHost / PTYHost
ClockHost
NetworkState
Logger / safe diagnostics
```

Каждый descriptor определяет необходимые capability.

Запрещено provider production code напрямую использовать:

```text
std::fs
tokio::fs
std::env
reqwest::Client
Command::new
shell execution
arbitrary Keychain service
arbitrary hostname
```

### 6.2. Runtime ownership

`usage-runtime` владеет:

- refresh orchestration;
- source strategy ordering;
- account/profile routing;
- coalescing;
- semaphore/concurrency;
- cancellation;
- cooldown/backoff;
- SWR/LKG;
- aggregation;
- notification planning;
- connection testing;
- trust/coverage policies.

### 6.3. Tauri boundary

`src-tauri` отвечает за:

- IPC;
- platform composition;
- tray/window lifecycle;
- native notification delivery;
- macOS-specific integration;
- DTO adaptation.

`src-tauri` не должен выбирать provider strategy и не должен вычислять domain totals.

---

## 7. Нормализованная модель данных

Минимальные идентификаторы:

```text
ProviderId
ProductId
AccountId
ConnectionId
SourceId
ProfileId / RootIdentity
BillingScopeId
```

Они не должны заменяться display name.

### 7.1. Account

```text
Account {
  id
  provider_id
  product_id
  display_name
  lifecycle
  enabled
  identity_confidence
  external_identity?
  credential_fingerprint?
}
```

### 7.2. SourceConnection

```text
SourceConnection {
  id
  account_id
  provider_id
  product_id
  source_kind
  root_identity?
  identity_confidence
  enabled
  created_at
  last_seen_at?
}
```

### 7.3. Coverage / confidence

Конкретные enum-значения определяются существующей domain model, но semantic contract сохраняется:

```text
Complete
Partial
UnverifiedSemantics
LocalClientOnly
Unknown
```

Authoritative по умолчанию могут быть только состояния, явно признанные trust policy.

### 7.4. Quota

```text
QuotaWindow {
  id
  label
  used_percent?
  remaining_percent?
  resets_at?
  window_seconds?
  window_kind
  source_strategy
  coverage
  freshness
}
```

### 7.5. History UsageRecord

History record обязан сохранять:

- account;
- product;
- source connection;
- source file/generation при local history;
- timestamp;
- model;
- token buckets;
- coverage;
- source record identity;
- provenance.

---

## 8. Source separation

В v1.3 subscription quota и local history считаются разными источниками.

### 8.1. Правило

```text
Quota source != History source
```

Их нельзя объединять в один trust state автоматически.

Пример:

```text
Codex · Personal

Quota
  Codex account usage source
  Fresh / Complete

History
  ~/.codex/sessions/**/rollout-*.jsonl
  Fresh / evidence-validated local semantics
```

Ошибка quota endpoint не должна отключать local history.

Ошибка history importer не должна превращать quota source в stale.

---

# 9. Codex v1.3

## 9.1. Product decomposition

Codex v1.3 состоит из трёх независимых логических частей:

```text
Codex
├── Subscription quota
├── Local history
└── Account/profile identity
```

## 9.2. Codex local history

Сохраняется существующий источник:

```text
$CODEX_HOME/sessions/**/rollout-*.jsonl
$CODEX_HOME/archived_sessions/**/rollout-*.jsonl
```

Default:

```text
~/.codex
```

Обязательные свойства:

- incremental scanning;
- generation/replacement detection;
- partial-tail tolerance;
- malformed-line tolerance;
- no cross-import inflation;
- no cross-profile contamination;
- model parsing;
- token buckets;
- rate-limit observations как local observations;
- coverage/provenance.

Финальная semantics identity должна соответствовать P0 evidence baseline.

## 9.3. Codex subscription quota

Главная новая функция v1.3.

Целевой strategy:

```text
CodexQuotaStrategy
    ↓
read-only local Codex credential/profile source
    ↓
ProviderHostFacade
    ↓
HTTPHost
    ↓
provider-owned account usage endpoint
    ↓
normalized QuotaWindow[]
```

Private/undocumented endpoint не считается стабильным публичным контрактом и должен быть изолирован strategy layer.

### 9.3.1. Требования

Quota source должен уметь получить, если это реально доступно в source response:

- primary/session window;
- weekly/secondary window;
- remaining/used percent;
- reset timestamp;
- window duration;
- additional quota groups;
- plan type;
- account/profile evidence.

### 9.3.2. Credential policy

Usage.ai:

- только читает credential;
- не обновляет credential;
- не перезаписывает auth store;
- не логирует token;
- не сохраняет raw token в SQLite/settings;
- перечитывает credential при последующем refresh;
- классифицирует stale/expired auth отдельно.

### 9.3.3. Fallback

Если online quota source недоступен, допустим fallback на последнюю локальную Codex rate-limit observation.

Fallback обязан иметь отдельную source strategy и coverage.

UI должен явно показывать:

```text
Online quota unavailable
Showing locally observed quota
```

Fallback не повышается до `Complete` автоматически.

### 9.3.4. Codex quota acceptance criteria

Обязательно доказать fixtures/tests для:

- primary window;
- secondary window;
- resets;
- additional windows;
- no-credits response;
- optional credits;
- plan absent;
- malformed response;
- 401/403;
- 429 + Retry-After;
- 5xx;
- timeout;
- cancelled request;
- partial/fallback behavior;
- expired credential;
- wrong account/profile binding;
- response body limits;
- no token leakage in logs/errors.

---

# 10. Claude v1.3

## 10.1. Product decomposition

```text
Claude Code
├── Subscription quota
├── Local history
└── Account/profile identity
```

## 10.2. Claude local history

Источник:

```text
$CLAUDE_CONFIG_DIR/projects/**/*.jsonl
```

Default:

```text
~/.claude/projects/**/*.jsonl
```

Сохраняется текущая evidence-validated модель:

- logical response identity;
- streaming/replay reducer;
- cache creation/read buckets;
- input/output;
- model;
- partial tails;
- profile isolation;
- provenance.

## 10.3. Claude subscription quota

Целевой ordered strategy:

```text
ClaudeQuota
│
├── Strategy 1: installed Claude Code usage command via PTYHost
│
└── Strategy 2: read-only OAuth usage source fallback
```

Порядок может быть пересмотрен только на основании discovery/real evidence, но каждый source остаётся отдельной strategy.

### 10.3.1. PTY strategy

Требования:

- использовать только allowlisted executable;
- не использовать shell;
- structured args;
- ограниченный timeout;
- cancellation;
- bounded output;
- deterministic parser;
- locale-independent parsing либо controlled locale;
- отсутствие arbitrary command injection;
- завершать только собственный child process;
- не расходовать inference quota.

### 10.3.2. OAuth fallback

Если используется provider-owned usage endpoint:

- credential только read-only;
- credential source scoped;
- no token mutation;
- no refresh-token writeback;
- HTTP hostname allowlisted descriptor'ом;
- response bounded;
- 401/403/429/5xx классифицируются отдельно.

### 10.3.3. Claude quota output

При наличии source evidence нормализуются:

- short/session window;
- weekly window;
- model-scoped weekly windows;
- reset times;
- additional usage;
- subscription plan/tier;
- coverage/fidelity.

Нельзя синтезировать denominator из local JSONL history.

### 10.3.4. Claude acceptance criteria

Обязательные tests:

- PTY command success;
- PTY missing executable;
- PTY timeout;
- PTY cancellation;
- malformed output;
- locale variance;
- OAuth fallback success;
- OAuth 401/403;
- OAuth 429 cooldown;
- strategy fallback ordering;
- one logical refresh coalescing;
- profile-specific credential;
- no cross-profile credential fallback;
- LKG on current failure;
- coverage/source visible to UI.

---

# 11. Multi-profile architecture

Multi-profile — обязательная функция v1.3.

## 11.1. Codex profiles

Минимально поддерживаются:

```text
~/.codex
~/.codex-<slug>
explicit custom CODEX_HOME roots
```

## 11.2. Claude profiles

Минимально поддерживаются:

```text
~/.claude
~/.claude-<slug>
explicit custom CLAUDE_CONFIG_DIR roots
```

## 11.3. Discovery policy

Запрещён бесконтрольный scan домашней директории.

Разрешённые candidate roots:

- default root;
- known provider naming convention;
- explicit user-added root;
- previously registered root.

Discovered root становится `candidate`, а не автоматически trusted account.

## 11.4. Canonical root identity

Перед созданием connection:

- expand `~`;
- canonicalize;
- reject traversal/escape;
- resolve symlink safely;
- derive stable root hash;
- reject silent rebinding одного physical root другому account.

## 11.5. No cross-profile fallback

Пример:

```text
Codex Work quota auth failed
```

Запрещено использовать:

```text
Codex Personal credential
```

даже если он валиден.

Fallback возможен только между strategy одного и того же logical account/profile, если identity доказана.

## 11.6. Profile data isolation

На profile scope должны быть независимы:

- account;
- connection;
- credential fingerprint;
- quota snapshots;
- local history;
- LKG;
- cooldown;
- refresh state;
- notification state;
- last success/error;
- diagnostics.

---

# 12. Source Fidelity / Trust UI

v1.3 должен сделать provenance частью основного UX.

## 12.1. Независимые состояния

Для каждой provider/profile card отображаются или доступны:

```text
ConnectionState
Freshness
Coverage
IdentityConfidence
SourceStrategy
CurrentError
LastSuccess
```

## 12.2. Пример verified source

```text
Codex · Personal

5-hour        73% left
Weekly        46% left

Source        Codex account usage
Updated       18 sec ago
History       Local JSONL
```

## 12.3. Пример fallback

```text
⚠ Online quota unavailable
Showing locally observed quota
```

## 12.4. Unverified source

Если `Coverage::UnverifiedSemantics`:

- amber/non-normal indicator;
- explicit warning;
- точное значение может отображаться только как non-authoritative;
- authoritative overview исключает его;
- tray не использует;
- notification planner suppress'ит authoritative alerts.

---

# 13. Diagnostics v2

Для каждого account/source должен быть diagnostic view.

Минимальные поля:

```text
Provider
Product
Account alias
Strategy
Availability
Connection state
Coverage
Identity confidence
Freshness
Last attempt
Last success
Latency
Classified error
HTTP status if safe
Cooldown until
Fallback reason
Credential source kind
Credential age if derivable safely
History root alias
Files discovered
Files imported
Records accepted/rejected
Current generation/checkpoint summary
```

## 13.1. Privacy restrictions

Diagnostics никогда не содержат:

- access token;
- refresh token;
- API key;
- raw JWT;
- cookie;
- Authorization header;
- raw request/response payload;
- prompt;
- assistant response;
- tool-call content;
- personal path, если можно показать безопасный alias;
- arbitrary environment variables.

## 13.2. Copy Diagnostics

Добавить действие:

```text
Copy diagnostics
```

Выход должен проходить safe redaction/allowlist.

Copy output обязан иметь tests на отсутствие secrets.

---

# 14. Refresh orchestration v1.3

Существующий refresh coordinator сохраняется.

## 14.1. Базовые свойства

- bounded parallelism;
- coalescing до semaphore acquisition;
- follower waiters;
- cancellation;
- shutdown cancellation;
- persisted cooldown;
- retry/backoff;
- transactional commit;
- SWR/LKG;
- account lifecycle validation inside commit transaction.

## 14.2. Adaptive refresh

Целевая policy:

```text
Active/recent provider activity   ~2 min
Normal                            ~5 min
Recently rate-limited             Retry-After / persisted cooldown
Offline                           exponential backoff
App wake                          refresh eligible sources
Manual refresh                    immediate + coalesced
```

Точные интервалы должны быть конфигурируемы policy layer, а не размазаны по UI/provider code.

## 14.3. Activity detection

Допустимые признаки активности:

- recently modified local history file;
- recent successful usage event;
- recently active provider process;
- explicit recent manual refresh.

Activity detection не должна запускать inference.

---

# 15. SWR / Last Known Good

При текущей ошибке provider:

- последний успешный snapshot сохраняется;
- freshness становится stale;
- current error отображается одновременно;
- данные не заменяются нулями;
- successful timestamp не обновляется;
- stale source не становится authoritative fresh.

Пример:

```text
Codex · Work
73% left
Данные устарели · обновлено 12 мин назад
Ошибка авторизации текущего обновления
```

---

# 16. Aggregation policy

## 16.1. Authoritative overview

По умолчанию overview использует только trusted coverage согласно central trust policy.

Нельзя суммировать `UnverifiedSemantics` с authoritative totals без явного отдельного режима.

## 16.2. Separate unverified view

Допускается отдельная секция:

```text
Неподтверждённые данные
```

с собственными totals.

## 16.3. Coverage preservation

Aggregate result должен сохранять как минимум:

- included source/account count;
- excluded unverified count;
- coverage/result confidence;
- period;
- provenance summary.

## 16.4. Money

Все monetary values проходят exact Decimal path.

Запрещено использовать `f64` для стоимости.

---

# 17. UI v1.3

UI остаётся компактным menu-bar panel и не требует полного redesign.

## 17.1. Provider/profile cards

Каждый профиль отображается отдельной карточкой:

```text
Codex · Personal
Codex · Work
Claude · Personal
Claude · Work
```

Не использовать глобальный account switch как единственный способ просмотра.

## 17.2. Карточка

Пример:

```text
Codex · Work                    Plus

5-hour        ███████░░ 73%
              Сброс через 2 ч 18 мин

Weekly        █████░░░░ 46%
              Сброс в пятницу 18:00

Source: Codex account usage
History: Local JSONL
Updated: 18 sec ago
```

## 17.3. Details

Provider details минимум:

```text
Usage limits
History
Models
Cost
Source
Diagnostics
```

Показывать только релевантные вкладки.

## 17.4. Discovery UI

Для candidate profile:

```text
Обнаружен Codex profile
Codex · Work

[Подключить] [Игнорировать]
```

Auto-connect может быть отдельным setting, default — conservative/manual connect.

---

# 18. Tray / Menu Bar

## 18.1. Default selection

По умолчанию menu-bar metric показывает наиболее constrained authoritative quota среди eligible profiles.

Пример:

```text
Codex Personal  74%
Codex Work      41%
Claude          63%

Tray -> 41%
```

## 18.2. Settings

```text
Menu bar metric:
- Most constrained authoritative quota
- Codex · Personal
- Codex · Work
- Claude · Personal
- No percentage
```

## 18.3. Trust policy

Non-authoritative coverage:

- не может быть выбрана автоматически;
- не показывает ordinary numeric tray metric;
- manual selection либо запрещается, либо получает явный experimental state.

Для v1.3 предпочтительно запрещать ordinary numeric tray display для unverified sources.

---

# 19. Notifications v1.3

Notifications привязаны к account/profile/window.

Пример:

```text
Codex · Work
Осталось 20% недельного лимита
Сброс через 1 д 8 ч
```

## 19.1. Thresholds

Default configurable thresholds:

```text
50%
20%
10%
5%
```

Также:

- pace warning;
- reset soon;
- reset completed;
- quota exhausted.

## 19.2. Per-profile configuration

Пользователь может включать/выключать notification по:

- provider;
- profile;
- window type;
- threshold type.

## 19.3. Authoritative-only policy

Quota notification engine по умолчанию принимает только authoritative coverage.

`UnverifiedSemantics` suppress'ится до semantic validation.

## 19.4. Budget notifications

Usage/budget alerts используют только trusted usage records.

Cost alerts не должны смешивать subscription usage и API cost.

---

# 20. Settings v1.3

Минимум:

### General

- launch at login;
- theme;
- refresh policy;
- manual refresh;
- language.

### Providers

- enable/disable provider profile;
- reorder profiles;
- add custom root;
- reconnect/retest source;
- ignore discovered profile.

### Menu Bar

- icon only;
- most constrained authoritative quota;
- fixed provider/profile;
- no percentage.

### Notifications

- global enable;
- per-profile enable;
- thresholds;
- reset notifications;
- quiet hours.

### Diagnostics

- copy diagnostics;
- clear local cache/LKG for selected source;
- show data directory;
- no secret reveal.

---

# 21. Storage changes v1.3

Schema evolution должна быть backward-compatible.

Ожидаемые сущности:

- accounts;
- source connections;
- profile/root identity;
- source files/generations;
- usage records;
- cost records;
- quota snapshots;
- attempts;
- cooldown state;
- notification state;
- optional strategy diagnostics.

## 21.1. Migration rules

Каждая migration:

- transactionally applied;
- backup before risky migration;
- idempotent/reopen-safe;
- не повышает identity confidence без evidence;
- не меняет modern API ownership;
- не смешивает profiles;
- имеет real old-schema tests.

---

# 22. Security requirements

## 22.1. Files

- opaque/scoped file handles;
- canonical root verification;
- traversal rejection;
- symlink escape rejection;
- bounded reads;
- no arbitrary filesystem path from provider.

## 22.2. HTTP

- hostname allowlist;
- HTTPS in production;
- redirects disabled unless explicitly justified;
- bounded body;
- streaming body read;
- absolute deadline;
- cancellation;
- Retry-After support;
- no unrestricted provider-owned `reqwest::Client`.

## 22.3. Keychain/credentials

- scoped service/account access;
- no arbitrary keychain lookup;
- no raw secret in logs;
- no raw secret in diagnostics;
- no permanent copy unless explicitly designed later.

## 22.4. Process/PTY

- executable allowlist;
- structured args;
- no shell;
- bounded stdout/stderr;
- timeout;
- cancellation;
- kill only owned child;
- no inherited dangerous env unless allowlisted.

## 22.5. SQLite permissions

On Unix/macOS:

- app data directory user-only;
- database user-only;
- sidecars protected;
- permission hardening fail-closed;
- permissions verified after set.

---

# 23. Error taxonomy

Минимальные classified states:

```text
NotInstalled
SourceUnavailable
Unauthenticated
PermissionDenied
UnsupportedSource
UnsupportedSchema
ParseError
RateLimited
Timeout
Cancelled
Offline
NetworkError
ServerError
CredentialExpired
AccountMismatch
StaleWithCurrentError
Unknown
```

UI не должен показывать raw provider error, если он может содержать sensitive payload.

---

# 24. ProviderDescriptor v1.3

Descriptor остаётся single source of static provider capabilities.

Минимально descriptor содержит:

```text
provider_id
product_id
display_name
icon
supported source strategies
strategy order
required host capabilities
allowed hostnames
known roots/profile conventions
quota capability
history capability
cost capability
identity capability
discovery state
```

UI eligibility должна выводиться из descriptor/capabilities, а не из hardcoded provider-name branches.

---

# 25. Testing strategy

## 25.1. Unit

- parsers;
- normalization;
- trust policy;
- aggregation;
- identity mapping;
- error classification;
- notification planning.

## 25.2. Integration

- real scoped FilesHost;
- real HTTPHost against local fake server;
- PTY fake executable/session;
- SQLite migration/import;
- quota → snapshot → DTO;
- history → storage → aggregate.

## 25.3. Concurrency

- scheduled + manual coalescing;
- follower waiters;
- leader cancellation;
- queued cancellation;
- shutdown;
- disable-before-commit;
- delete-before-commit;
- profile-specific cancellation.

## 25.4. Account isolation

Минимум сценарии:

```text
Profile A -> refresh/import
Profile B -> refresh/import
Profile A -> refresh/import again
```

Проверить:

- no row mixing;
- no credential fallback across profiles;
- no LKG mixing;
- no cooldown mixing;
- no notification-state mixing.

## 25.5. UI

Component tests должны покрывать:

- verified card;
- unverified card;
- stale + current error;
- multi-profile cards;
- discovery candidate;
- fallback source warning;
- tray selection policy logic;
- profile-specific notification labels.

---

# 26. Mandatory verification gates

Каждый milestone, который меняет production behavior, завершается минимум:

```bash
npm run check
npm test -- --run
npm run build
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Все команды должны завершаться PASS.

Warnings в strict Clippy не допускаются.

---

# 27. Milestones v1.3

## V13-00 — P0 Seal

**Priority:** P0  
**Цель:** зафиксировать доказанный безопасный baseline до provider expansion.

### Tasks

- закрыть последний P0-05 evidence ambiguity;
- подтвердить P0-06;
- провести final independent P0 gate audit;
- commit remediation/evidence reports;
- clean working tree;
- tag baseline;
- сохранить hashes ключевых audit/spec artifacts.

### Acceptance

- нет известных P0 integrity/security blockers;
- все mandatory gates green;
- clean reproducible revision;
- provider expansion formally unlocked.

---

## V13-01 — Multi-profile Foundation

**Priority:** P0/P1

### Tasks

- profile discovery abstraction;
- Codex known root convention;
- Claude known root convention;
- candidate roots;
- explicit user connect/ignore;
- canonical root identity;
- account/source binding;
- per-profile refresh/LKG/cooldown;
- UI separate cards;
- settings management.

### P0 acceptance

- same physical root cannot silently bind to two accounts;
- two roots cannot merge into default account;
- credential from profile A cannot be used for profile B;
- A/B/A regression passes;
- deleting/disabling B does not affect A;
- migration from single-profile baseline preserves rows.

---

## V13-02 — Codex Verified Quota Source

**Priority:** P0/P1

### Tasks

- discovery of actual local Codex credential source;
- read-only credential snapshot;
- account/profile binding;
- quota HTTP strategy;
- normalized primary/secondary/additional windows;
- plan mapping;
- classified errors;
- persistent cooldown;
- LKG/SWR;
- local JSONL quota observation fallback;
- source fidelity UI.

### P0 acceptance

- no inference call;
- no auth mutation;
- no secret persistence/logging;
- no cross-profile fallback;
- one logical refresh → one physical quota fetch;
- coverage reflects strategy;
- fallback visibly marked;
- tray/notifications use only authoritative source.

---

## V13-03 — Claude Verified Quota Source

**Priority:** P0/P1

### Tasks

- Claude PTY `/usage` strategy;
- bounded PTY host execution;
- deterministic parser;
- OAuth usage fallback;
- profile credential isolation;
- quota normalization;
- session/weekly/model-scoped windows;
- plan/tier if source provides it;
- classified errors;
- LKG/cooldown;
- source fidelity UI.

### P0 acceptance

- no inference quota consumed;
- executable allowlisted;
- shell not used;
- OAuth secret read-only;
- no credential mutation;
- PTY failure cleanly falls back if eligible;
- account/profile identity remains correct;
- unverified source cannot trigger authoritative side effects.

---

## V13-04 — Fidelity + Diagnostics v2

**Priority:** P1

### Tasks

- source badge/state;
- coverage display;
- identity confidence display where useful;
- fallback reason;
- source diagnostics;
- safe copy diagnostics;
- latency/last success/cooldown;
- history import statistics.

### Acceptance

- every visible quota can answer “откуда это значение?”;
- no secret/path leakage in copied diagnostics;
- fallback cannot look identical to primary source;
- stale + current error simultaneously visible.

---

## V13-05 — Adaptive Runtime

**Priority:** P1

### Tasks

- app-wake refresh;
- activity-aware schedule;
- offline backoff;
- persisted provider cooldown;
- source-specific cadence policy;
- manual refresh coalescing;
- profile-level refresh cancellation.

### Acceptance

- no polling storm;
- no duplicate concurrent fetch for same scope;
- inactive provider cadence reduces;
- wake/manual refresh works without violating cooldown semantics;
- app restart restores cooldown behavior.

---

## V13-06 — Product UX / Notifications

**Priority:** P1/P2

### Tasks

- multi-profile cards polish;
- menu-bar fixed-profile selection;
- most constrained authoritative mode;
- per-profile notification settings;
- quota thresholds;
- reset alerts;
- quiet hours;
- discovery UX;
- settings cleanup.

### Acceptance

- profile name present in native notification;
- unverified profile does not emit authoritative alert;
- mixed verified/unverified profiles behave predictably;
- selected tray profile persists across restart.

---

## V13-07 — Release Gate

**Priority:** P0

### Required validation

- clean install on supported Intel macOS;
- upgrade from previous local DB;
- existing Codex history preserved;
- existing Claude history preserved;
- OpenAI API costs preserved;
- multiple profiles discovered safely;
- auth-expired scenario;
- offline scenario;
- rate-limited scenario;
- stale/LKG scenario;
- disable/delete account during refresh;
- launch-at-login smoke;
- tray/window smoke;
- notifications smoke;
- packaged app opens without modifying official clients.

### Build artifacts

Минимально:

- `.app`
- local installer/DMG if currently supported by project
- reproducible version `1.3.0`

### Final audit

Перед release tag проводится независимый read-only audit по Master Spec v1.3.

Release запрещён при P0/P1 data-integrity/security blocker.

---

# 28. Priority model

## P0 — release blockers

- account/profile contamination;
- secret exposure;
- credential mutation;
- inference-consuming monitoring;
- incorrect authoritative quota;
- unverified data driving tray/notifications;
- money precision loss;
- stale commit after account disable/delete;
- insecure DB/auth access;
- HostFacade bypass;
- data-destructive migration;
- provider refresh duplication that affects quota/rate limits.

## P1 — required for v1.3 product quality

- clear fidelity UI;
- diagnostics;
- robust fallback;
- adaptive refresh;
- profile discovery UX;
- per-profile notifications;
- good classified error states.

## P2 — polish/non-blocking

- animations;
- advanced visual customization;
- richer charts;
- optional convenience shortcuts;
- extended export formatting;
- minor layout refinement.

---

# 29. Release acceptance matrix

| Area | v1.3 requirement |
|---|---|
| Codex quota | Production-ready verified primary strategy |
| Claude quota | Production-ready primary/fallback strategy |
| Codex history | Evidence-validated and isolated per profile |
| Claude history | Evidence-validated and isolated per profile |
| OpenAI API cost | Existing exact Decimal path preserved |
| Multi-profile | No cross-account contamination |
| Credentials | Read-only and scoped |
| Monitoring | No inference consumption |
| Fallback | Explicitly identified and confidence-aware |
| SWR | LKG + current error |
| Tray | Authoritative sources only |
| Notifications | Authoritative sources only |
| Diagnostics | Safe/redacted |
| Refresh | Coalesced, cancellable, cooldown-aware |
| SQLite | Transactional + user-only permissions |
| UI | Separate provider/profile cards |
| Intel package | Smoke-tested |
| CI/gates | All green |
| Independent audit | No release-blocking integrity/security defects |

---

# 30. Definition of Done v1.3

Usage.ai v1.3.0 считается завершённым только если одновременно выполняется:

```text
Codex quota       production-ready
Claude quota      production-ready
Codex history     evidence-validated
Claude history    evidence-validated
OpenAI API cost   regression-protected
Multi-profile     isolated
Fallbacks         explainable
Source fidelity   visible
SWR               verified
Tray              authoritative only
Notifications     authoritative only
Credentials       read-only + scoped
Monitoring        zero inference
Money             exact Decimal
Migrations        safe
Intel package     smoke-tested
Mandatory gates   green
Final audit       no P0/P1 integrity/security blocker
```

---

# 31. Non-normative research references

Для discovery и comparison допускается использовать существующие open-source проекты как reference implementations, но не как нормативный security contract.

Основные роли references:

```text
Codenotch
  -> provider-specific discovery
  -> current quota-source patterns
  -> multi-profile operational patterns

CodexBar
  -> mature provider strategy patterns
  -> source/fallback behavior
  -> usage monitor operational patterns

Usage.ai
  -> stricter HostFacade/capability isolation
  -> local history + analytics
  -> transactional provenance
  -> account/profile isolation
  -> trust-aware side effects
```

Любой private/undocumented endpoint, найденный в reference project, сначала проходит Discovery и controlled validation.

Он не считается стабильным контрактом только потому, что используется другим проектом.

---

# 32. Архитектурный итог v1.3

Целевая модель релиза:

```text
                         Usage.ai v1.3

     Codex · Personal                       Claude · Work
     ───────────────                       ──────────────
     Quota Strategy                         Quota Strategy
          │                                      │
     scoped HTTP/auth                         PTY / OAuth
          │                                      │
          └──────────────┐        ┌──────────────┘
                         ▼        ▼
                    ProviderHostFacade
                         │
                    usage-runtime
               ┌─────────┴─────────┐
               │                   │
            Quota              Local History
               │                   │
               │            scoped JSONL import
               │                   │
               └─────────┬─────────┘
                         ▼
                      SQLite
                         │
             trust-aware aggregation
                         │
           ┌─────────────┼─────────────┐
           ▼             ▼             ▼
         Panel          Tray      Notifications
       provenance    authoritative  authoritative
        visible          only          only
```

Ключевой принцип:

> JSONL отвечает за историю. Provider-owned quota source отвечает за лимит. Account identity связывает их только тогда, когда это можно доказать.

И второй ключевой принцип:

> Provider не получает доверие ко всей системе просто потому, что ему нужно прочитать usage. Usage.ai показывает только данные, источник, аккаунт, coverage и freshness которых он способен объяснить.

---

# 33. Следующий релиз после v1.3

При успешном v1.3 рекомендуемый scope v1.4:

1. Antigravity Discovery → production integration.
2. Z.ai Coding Plan.
3. OpenCode Go / Zen / local separation.
4. Дополнительные provider-specific quota groups.

Эти работы не должны начинаться внутри v1.3, кроме read-only research и fixtures вне production registry.

