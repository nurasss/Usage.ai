# Usage.ai — технический аудит текущей реализации перед Master Spec v1.1

Дата аудита: 2026-09-08  
Ревизия проекта: `8fc2e34` (`main`, tag `v1.0`)  
Режим аудита: только чтение и анализ реализации; исходный код приложения не изменялся.

## Ограничения и методика

В доступном рабочем каталоге отсутствует `Usage.ai_MASTER_SPEC_v1.0.md`; во вложении доступен только текст задания на аудит. Поэтому раздел соответствия ниже проверяет требования, явно перечисленные в задании, а не неизвестные формулировки и acceptance criteria полного Master Spec. Архив также не был отдельным вложением: аудит выполнен по Git-репозиторию `/Users/nuras/Desktop/usage.ai` на указанной ревизии.

Проверены все исходные файлы Rust/TypeScript/Svelte, SQL-миграции, конфигурация Tauri, package manifests, документация и CI. Выполнены `npm run check`, `npm test`, `npm run build`, `cargo test --workspace` и строгий `cargo clippy --workspace --all-targets -- -D warnings`. Для сравнения с CodexBar использованы его актуальные документы и исходная архитектура в `steipete/CodexBar`; private API рассматриваются как исследовательские источники, а не как стабильный контракт.

Статусы: ✅ реализовано; 🟡 частично; 🔴 отсутствует; ⚠️ реализовано, но архитектурно проблемно; ❓ невозможно подтвердить.

## 1. Executive Summary

Usage.ai сейчас является работающим локальным macOS menu-bar прототипом на Tauri 2 + Svelte 5 + Rust/SQLite. Реально поддержаны только три источника:

1. локальные Codex JSONL — токены и последний найденный rate-limit snapshot;
2. локальные Claude Code JSONL — токены и модели, но не subscription quota;
3. официальный OpenAI Organization Costs API — reported cost за 30 дней, только с подходящим admin key/scope.

Antigravity, Z.ai, OpenCode Zen, ChatGPT consumer, claude.ai и Gemini API данных не получают. Их карточки — явные `UnsupportedSource`, созданные через `DiscoveryOnlyAdapter`; это честнее fake metrics, но не является реализацией provider integration (`src-tauri/src/lib.rs:1231-1273`). Claude API, Z.ai API, OpenCode Go и локальный OpenCode вообще не заведены как отдельные продукты.

Сильные стороны: удачная provider-neutral модель `Account/Product/Capability/Snapshot/UsageRecord/CostRecord`; SQLite с миграциями, idempotent upsert и byte-offset checkpoints; разделение subscription quota и API cost на уровне `product_id`; decimal money; Keychain для собственных API keys; HTTPS host allowlist и запрет redirects; whitelist diagnostics/export; корректное различение missing и zero; русскоязычный UI; работающий Intel CI job.

Главная проблема — архитектурный каркас заметно опережает рабочую интеграцию. `ProviderAdapter` объединяет discovery, fetch, parsing и mapping; стратегий и fallback pipeline нет; Host APIs отсутствуют; `src-tauri/src/lib.rs` (2183 строки) вручную знает все products, UI labels, refresh policy, storage, notifications, Keychain и platform lifecycle. Это уже создаёт расхождения: capability system существует, но UI в основном не управляется capabilities; retry helper существует, но orchestration его не вызывает; snapshot cache существует, но при ошибке refresh успешный provider snapshot заменяется пустой error DTO; заявленная параллельность отсутствует.

P0 до расширения провайдеров:

- исправить double-fetch OpenAI Costs на каждом refresh (один запрос для snapshot, второй для import);
- сделать refresh transactional/stale-while-revalidate: ошибка источника не должна стирать последний успешный provider snapshot;
- определить корректную семантику Codex/Claude JSONL token events и дедуп streaming/cumulative events; текущий Codex импорт рискует считать каждый `last_token_usage` как независимый запрос, Claude дедуплицирует только UUID и не использует `message.id + requestId`;
- разделить Provider Descriptor / Fetch Strategy / Parser / Mapper и ввести ограниченные Host APIs до добавления private/local sources;
- включить `src-tauri` в clippy CI и добавить интеграционные тесты рабочего refresh/storage flow.

Проект не нужно переписывать целиком. `usage-core` models, большая часть storage schema, HTTP safety wrapper, Tauri/Svelte shell и текущий локальный-first принцип пригодны для v1.1 после декомпозиции orchestration.

## 2. Что сейчас реально реализовано

| Область | Фактическое состояние | Статус |
|---|---|---|
| macOS menu bar | tray icon, click-to-toggle panel, accessory activation, hide-on-blur | ✅ |
| Codex subscription quota | последний `rate_limits` из newest `rollout-*.jsonl`; primary/secondary dynamic pools | 🟡 |
| Codex local history | incremental import всех `~/.codex/sessions/**/rollout-*.jsonl` | 🟡 |
| ChatGPT subscription | только disabled/unsupported card | 🔴 |
| OpenAI API billing | `/v1/organization/costs`, reported cost | 🟡 |
| OpenAI API usage/tokens | records не создаются (`usage_records` локально объявлен пустым) | 🔴 |
| Claude Code history | incremental `~/.claude/projects/**/*.jsonl` | 🟡 |
| Claude quota/auth | credentials, OAuth, Keychain и quota source не реализованы | 🔴 |
| Claude API | отдельного adapter/product нет | 🔴 |
| Antigravity | discovery-only card | 🔴 |
| Z.ai | discovery-only card | 🔴 |
| OpenCode / Go / Zen | только одна discovery-only Zen card | 🔴 |
| Multi-account | несколько OpenAI API account rows + individual Keychain items | 🟡 |
| History | SQLite, 4 ranges, checkpoints, retention, export | 🟡 |
| Notifications | quota/auth/reset/cost/budget rules, persistent dedup | 🟡 |
| Update channel | явно выключен | 🔴 |

Mock/demo data находятся в `src/lib/demo.ts`: вымышленные Codex/Claude/OpenAI API providers, quotas, costs и overview; `src/lib/api.ts:24-27` возвращает этот snapshot не только в browser preview, но и при любой ошибке `get_snapshot`. Это опасная UX-семантика: backend failure в packaged app может выглядеть как правдоподобная demo-статистика. Fixtures `crates/usage-providers/fixtures/*.jsonl` синтетические и используются только тестами; они не попадают в runtime.

## 3. Архитектура

### 3.1 Модули и границы

- `crates/usage-core`: domain models, capability enum, adapter trait, aggregation helpers, retry-decision helper, notification helper, safe diagnostics.
- `crates/usage-providers`: Codex/Claude local parsers, OpenAI costs HTTP adapter, shared HTTP helper, discovery stub.
- `crates/usage-storage`: SQLite connection, migrations, repositories/queries/export в одном файле.
- `src-tauri`: composition root, IPC, orchestration, scheduler, cache, Keychain, notification delivery, filesystem scanning, tray/window integration.
- `src`: Svelte UI, DTO types, IPC wrapper, demo data, formatting.

Физическое разбиение разумное, но логические границы неполны. `ProviderAdapter` (`crates/usage-core/src/provider.rs:23-49`) одновременно декларирует provider/product/capabilities/hosts, делает remote refresh и local import. Нет `ProviderDescriptor`, `Connector`, `Parser`, `Mapper`, `FetchStrategy`, `FetchOutcome` или source diagnostics. Codex/Claude parsers являются private methods/inline code внутри adapters. OpenAI parser выделен pure function, что является лучшим текущим образцом.

`src-tauri/src/lib.rs` — чрезмерно связанный god-module. Он содержит DTO, settings, cache, all refresh flows, provider registry в виде массивов/if-chain, Keychain calls, database calls, notifications, history import, overview queries и native lifecycle. Добавление provider требует правок минимум в `supported_products`, `build_live_snapshot`, managed-account filtering, UI `productOptions`, icon/name mappings и diagnostics versions.

### 3.2 Масштабирование

Domain schema масштабируется умеренно хорошо: `Account.id`, `provider_id`, `Product`, `product_id`, `billing_scope_id`, source identity и unique constraints заложены правильно (`crates/usage-core/src/models.rs`; `crates/usage-storage/migrations/001_initial.sql`). Runtime — нет: local Codex/Claude используют по одному детерминированному UUID (`stable_local_account`) и игнорируют managed local accounts/custom paths. Несколько OpenAI API accounts поддерживаются последовательно. Для других products UI позволяет создать account row, но refresh его игнорирует.

### 3.3 Технологический стек

Фактически: Tauri 2, Rust 2021, Tokio, Reqwest/Rustls, Rusqlite bundled SQLite, keyring/apple-native, Svelte 5, TypeScript 5.9, Vite 7, Vitest/jsdom. Native features: autostart, global shortcut, notification, tray. Это компактный и подходящий стек для local-first utility. Замена Tauri/Svelte или SQLite не обоснована.

Технический долг зависимостей/решений:

- `glob` синхронно сканирует filesystem на Tokio task;
- единый `Mutex<rusqlite::Connection>` и per-row upserts без batch transaction;
- `rustls` комментарий обещает bundled WebPKI, но Cargo features Reqwest явно не фиксируют backend — это надо подтверждать lock/build configuration, а не документацией;
- `macos-private-api` и transparent window повышают platform risk;
- version `0.1.0` расходится с Git tag `v1.0` и UI label expectations;
- updater/signing/notarization отсутствуют.

## 4. Provider-by-provider audit

### 4.1 Codex / ChatGPT / OpenAI

**Codex local.** `CodexLocalAdapter` ищет newest session через полный glob и читает файл с offset 0 для quota refresh (`crates/usage-providers/src/codex.rs:58-75, 164-211`). Источник — undocumented local JSONL Codex, не официальный API contract. Парсер принимает только `payload.type == token_count`, читает `info.last_token_usage` и `rate_limits`. Quota pool IDs динамические, primary/secondary не названы жёстко по моделям — это хорошо. Но refresh видит только newest file; rate limits в другом свежем/архивном файле будут пропущены. `window_kind` ошибочно маркируется `Fixed`, если можно вычислить start; наличие duration/reset само по себе не доказывает fixed versus rolling.

Local history импортирует все active session files инкрементально (`src-tauri/src/lib.rs:1650-1735`). `CODEX_HOME` учитывается внутри adapter refresh, но importer жёстко использует `$HOME/.codex`, поэтому custom home и refresh/history могут расходиться. `archived_sessions` не импортируются. Model markers не разбираются; Codex records всегда `model=None`. Account identity/auth/credits отсутствуют.

Особый риск double count: каждое `last_token_usage` сохраняется как отдельная запись по byte offset. Нужно fixture-based доказательство, что поле является delta для каждого request, а не повторяемым/cumulative snapshot. Сейчас такого теста нет.

**Codex App Server.** Не используется. Нет ProcessHost/PTY, JSON-RPC lifecycle, `account/read`, `account/rateLimits/read`, cancellation/kill on timeout. Это полезный кандидат на отдельную strategy, поскольку даёт актуальные rate limits, account identity и credits; не следует запускать его без cadence/coalescing из-за стоимости startup и возможных side effects.

**ChatGPT consumer.** Auth, cookies/OAuth, usage dashboard и quotas отсутствуют. Card в blocked array не является реализацией.

**OpenAI API.** `OpenAiApiAdapter` вызывает официальный admin endpoint `GET https://api.openai.com/v1/organization/costs` с 30-day range, daily buckets и limit 180 (`crates/usage-providers/src/openai_api.rs:11-12, 131-151`). Endpoint требует organization-level admin privileges; обычный inference key не гарантированно подходит. Pagination не реализована: `has_more` лишь превращается в `Coverage::Partial`, next page не запрашивается. Usage endpoint не вызывается, tokens/requests отсутствуют. Balance правильно остаётся unknown.

На один successful refresh выполняются два одинаковых network request: `refresh_with_key`, затем `import_costs_with_key` (`src-tauri/src/lib.rs:1368-1404`). Это удваивает latency/rate-limit pressure. `refresh_with_key` создаёт пустой `usage_records` vector без применения. Cost record ID не включает currency и amount; project+line item within same bucket конфликтуют, если provider вернёт несколько строк одинакового line item/scope.

Codex subscription и OpenAI API billing разделены корректно через `product_id` (`codex` vs `openai-api`) и разные accounts/sources; UI всё же группирует обе под provider `openai`.

### 4.2 Claude

`ClaudeLocalAdapter` не читает credentials, Keychain или OAuth. Его `refresh` всегда возвращает Connected с empty quotas и unknown freshness независимо от наличия файлов (`crates/usage-providers/src/claude.rs:50-70`), поэтому connection state вводит в заблуждение. Фактический источник — undocumented local JSONL `~/.claude/projects/**/*.jsonl`.

Importer учитывает `message.usage.input_tokens`, `output_tokens`, `cache_read_input_tokens`, model и envelope UUID (`claude.rs:72-147`). Cache read хранится отдельно и не добавляется в total — это верно. Cache creation field не моделируется. Нет дедуп streaming chunks по `message.id + requestId`; UUID может быть достаточно только для некоторых schemas, что не доказано fixtures. Нет session/weekly/model limits. Claude API usage/cost как отдельный product отсутствует.

### 4.3 Antigravity

Реализация отсутствует (`DiscoveryOnlyAdapter`). Нет process/language-server detection, localhost RPC, `agy`, quota group parser, OAuth, account matching или IDE-closed behavior. Следовательно, никаких Gemini Pro/Flash/Claude pools в runtime не зашито; demo для Antigravity отсутствует. Integration spec в `docs/integrations/Antigravity-Integration-Spec.md` — шестистрочная placeholder-форма, не код.

### 4.4 Z.ai

Реализация отсутствует. Нет token source, quota endpoint, global/CN region, Personal/Team headers/scopes, Coding Plan/API separation или usage mapping. Единственный product id `glm-coding`; `zai-api` отсутствует. UI позволяет сохранить такой account без secret, но refresh игнорирует его.

### 4.5 OpenCode / Go / Zen

Реализация отсутствует. Код моделирует только `opencode/zen`; локальный OpenCode и OpenCode Go не имеют отдельных product IDs. Нет local SQLite, browser session/API, workspace ID, rolling/weekly quota, balance/cost или history parser. Защита от gateway-vs-direct double count существует только как unit test агрегации разных `product_id` (`usage-core/src/aggregation.rs:124-130`), но применить её не к чему и source attribution для OpenCode requests отсутствует.

### 4.6 Прочие заявленные продукты

`claude.ai`, `gemini-api` и ChatGPT существуют только в blocked list. Claude API и Z.ai API упомянуты в docs, но отсутствуют даже в runtime registry. Наличие `docs/integrations/*` не следует учитывать как реализацию: большинство файлов содержат только status/source/acceptance placeholder.

## 5. Storage / History

SQLite schema v2 содержит accounts, snapshots, usage_records, cost_records, import_checkpoints, notification_deliveries, app_cache, budgets и balances. `snapshots` table фактически нигде не записывается/читается; вместо неё весь `AppSnapshot` хранится JSON blob в `app_cache` (`get_snapshot`, `cache_snapshot`). Это мешает per-provider stale recovery и provenance.

Положительное:

- migrations transaction + `user_version`; pre-migration backup создаётся для version 1;
- unique keys делают повторный import idempotent;
- checkpoints сохраняют byte offsets и partial line не продвигает cursor;
- size shrink сбрасывает checkpoint;
- local timezone корректно превращается в UTC boundaries для Today/Yesterday/7/30 days;
- unknown model сохраняется отдельно;
- money хранится decimal text и currency не конвертируется.

Проблемы:

- discovery всё равно выполняет полный glob всех JSONL при каждом refresh; содержимое читается только с offsets, то есть полного reparse обычно нет, но filesystem scan и metadata каждого файла повторяются;
- checkpoint keyed только path hash; rotation/replacement одинакового или большего размера не распознаётся. `mtime_ms` сохраняется, но reset проверяет фактически только shrink/schema (проверить `reset_checkpoint_on_rotation`);
- importer игнорирует warnings и import errors; пользователь их не видит;
- per-record SQLite writes вне одной transaction;
- retention удаляет только `usage_records`, но не old cost, snapshots, notifications, checkpoints или cache (`usage-storage/src/lib.rs:216-224`);
- settings retention минимум 30 дней; произвольной политики/size cap нет;
- history export исключает account ID и identity, но экспортирует source/model and exact timestamps, что всё равно является sensitive activity metadata;
- `snapshots` FK существует, а usage/cost tables не имеют declared FK;
- migration SQL uses `ALTER TABLE ADD COLUMN` without per-statement idempotence; transaction/user_version делает normal path приемлемым, но recovery после ручного/частичного schema drift слабый.

## 6. Security

### Сильные стороны

- Usage.ai-owned OpenAI keys находятся в macOS Keychain service `com.nurasss.usageai`, account = UUID (`src-tauri/src/lib.rs:742-749, 817-823`). SQLite хранит fingerprint/connection ref, не raw secret.
- Secret-bearing HTTP принимает только HTTPS exact-host allowlist, не follows redirects и не логирует body/header (`crates/usage-providers/src/http.rs:9-79`).
- CSP запрещает remote scripts/content; frontend не имеет shell/process API (`src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`).
- diagnostics/export строятся whitelist DTO; path hash вместо raw path; provider errors сводятся к safe codes.
- Foreign CLI credentials сейчас вообще не читаются, что уменьшает attack surface.

### Риски

1. **Arbitrary file metadata/read scope.** `custom_path` принимается как произвольная строка без canonicalization/allowlist/bookmark, хотя сейчас не используется. После подключения это станет arbitrary file read из frontend-controlled IPC. Нужен `LocalFileHost` с product-scoped roots, symlink policy и security-scoped bookmark/user selection.
2. **Generic secret IPC.** `store_secret(account_id, secret)` позволяет frontend записать произвольный Keychain account до 128 chars, не проверяя существующий account/product. CSP снижает риск, но API шире необходимого. Secret проходит через WebView IPC и JS memory/input state (`newSecret`), хотя не persistence.
3. **Demo fallback.** `loadSnapshot` ловит любой IPC error и показывает realistic fake data без обязательного persistent demo banner semantics. Это integrity risk.
4. **At-rest metadata.** SQLite/settings file permissions явно не устанавливаются; полагаются на app-data defaults/umask. API-key fingerprint, aliases, exact usage timestamps and models remain sensitive metadata.
5. **Redaction helper incomplete and unused in runtime logging.** `redact_message` tokenizes by whitespace and может пропускать `key=value` structures with short secrets; orchestration fortunately logs only controlled errors. Policy should live in LoggerHost.
6. **Diagnostics `schema_fingerprint` is not a fingerprint.** В него кладётся quota `source` или product-local label (`src-tauri/src/lib.rs:649-655`), что может нести timestamps and confuses diagnostic contract.
7. **Export destination.** History/diagnostics silently write into Documents/Downloads without save dialog; this is user-visible but may violate least-surprise/privacy.
8. **No signing/notarization/update chain.** signing identity/entitlements absent; updater disabled. Для public distribution это supply-chain/reputation risk, не remote-code risk внутри текущей сборки.

Cookies, OAuth access/refresh tokens, credentials других apps, arbitrary shell execution и localhost server/TLS в текущей версии отсутствуют. Поэтому соответствующие утечки не найдены; это не означает готовность будущих integrations.

## 7. UI / macOS integration

Svelte UI реализует sidebar, overview, provider cards, quota bars/reset countdown, settings, accounts, budgets, diagnostics/export, loading skeleton, offline banner, stale/error labels, Russian strings and responsive narrow panel (`src/App.svelte`, `src/styles.css`). Capability data передаётся, но provider cards рендерят fields по наличию, а settings/options/icon/name mappings hard-coded. Capability-based UI реализован лишь частично.

Проблемы UI/UX:

- theme setting сохраняется, но не применяется к DOM/CSS; `prefers-color-scheme` может работать независимо, однако explicit light/dark не работает;
- menu-bar `iconMetric` setting сохраняется, но tray title/icon не меняется;
- duplicate products produce duplicate sidebar `id` and `document.getElementById(productId)`, multi-account navigation ambiguous;
- local Codex/Claude default accounts всегда отображаются, даже если managed accounts disabled;
- account editor создаёт unsupported accounts, после чего они нигде не появляются в snapshot;
- `test_connection` поддерживает только OpenAI API и Codex, не Claude local;
- cost period selector не влияет на backend cost display: provider DTO содержит только `reportedCostToday`; overview metric `cost` имеет мало/нет period data;
- errors often collapse to generic notices; source warnings/schema changes unavailable to user;
- cached snapshot may indefinitely show old `nextRefreshAt` without recomputing freshness;
- no explicit demo/error differentiation on backend failure.

macOS: Accessory policy hides Dock, tray click positions panel near status item, always-on-top borderless transparent window, hide on focus loss, global shortcut, launch at login and notifications exist. Multi-monitor positioning is naive (no visible-frame clamping, left/right edge handling, notch/menu-bar geometry). `toggle_panel(None)` from shortcut reuses prior/default position. Sleep/wake is approximated by WebView visibility refresh and timer behavior; no native power/network observer. Intel target is built on macos-13 CI. Apple Silicon/universal artifact is not built. No updater; DMG configured; signing/notarization absent.

## 8. Performance / Refresh / stale data

Scheduler is one infinite Tokio loop using fixed setting interval (`src-tauri/src/lib.rs:2061-2087`). It does one immediate refresh at startup, then sleeps. Manual refresh and tray refresh can overlap scheduler refresh: there is no global coalescing/cancellation guard. UI only prevents duplicate button clicks within one WebView.

Provider refresh is sequential, despite comments claiming per-source timeout prevents one source blocking the rest. Worst-case latency is sum of timeouts/accounts, plus duplicated OpenAI call. No `join_all`, semaphore or concurrency budget. Cancellation tokens absent.

`Retry-After` is honored for 429 via in-memory `cooldown_until`; it is lost on restart. The exponential backoff helper in `usage-core/src/scheduler.rs` is not called anywhere. Network errors are retried every configured interval, offline backend state is never detected, and `offline=false` is hardcoded. No stale-while-revalidate per provider: failed refresh emits an empty DTO, overwrites cached aggregate snapshot, and discards last successful quota/card values. The SQLite cache only helps initial `get_snapshot`, and even then no background refresh is triggered when cache exists.

`next_refresh_at` is always now+5 minutes regardless of configured interval/manual mode (`src-tauri/src/lib.rs:1293`). UI maintains a 1-second timer for countdowns; that is acceptable. Major costs are repeated recursive globs, synchronous metadata/file IO in async tasks, sequential HTTP, duplicated requests, unbatched SQLite upserts and repeated aggregate queries (4 ranges × 4 breakdown queries).

Adaptive Refresh is worthwhile after correctness fixes. Recommended pure policy inputs: recent panel interaction, newest local JSONL activity, power/thermal state, last source outcome, rate-limit reset proximity and manual mode. It must not increase private endpoint polling; it should recompute only after a coalesced refresh completes and remain bounded (e.g. 2–30 min). Current fixed 5 min is simpler but unnecessarily active when idle and too slow immediately after active usage.

## 9. Tests / CI / Build

Observed results:

- Svelte check: pass, 0 errors/warnings.
- Vitest: 5 tests pass.
- Vite production build: pass.
- Rust workspace tests: 32 tests pass (including 2 app tests, 13 core, 10 provider, 7 storage; doc tests pass).
- Strict whole-workspace clippy: fail with 6 errors in `src-tauri/src/lib.rs` (items after test module, two clone-on-copy, needless question mark, type complexity, while-let-loop).

CI's `portable` job runs clippy and tests only for `usage-core`, `usage-providers`, `usage-storage`; it excludes `usage-ai-app`, hiding the failures in the most coupled code. Intel job runs full tests and x86_64 app/DMG build but no signing/notarization/smoke launch. No arm64 or universal job.

Critical missing tests:

- end-to-end `build_live_snapshot` with temp filesystem/DB/Keychain/HTTP;
- stale cache preservation on parse/network/auth error;
- concurrent/manual/scheduler coalescing and cancellation;
- OpenAI pagination, Retry-After/date formats, redirects via local test server, one-request behavior;
- Codex cumulative-vs-delta semantics, multiple files, archived sessions, `CODEX_HOME`, truncation/replacement;
- Claude streaming dedup, cache creation, duplicate roots, malformed/rotated files;
- migrations from real v1 DB, corrupted/partial schema, rollback/backup;
- DST/timezone boundaries and retention across timezone change;
- multi-account identity switching/credential isolation;
- Tauri command authorization and Keychain failure modes;
- UI card states, capability-driven visibility, demo/error banner, multi-account IDs;
- tray position/multi-monitor, launch login, notification permission, sleep/wake;
- packaged Intel runtime smoke test.

## 10. Сравнение с Master Spec v1.0

Полный spec отсутствует, поэтому ниже — честная матрица по крупным требованиям из задания.

| Требование | Статус | Основание |
|---|---:|---|
| Разделение UI/core/providers/storage/platform | 🟡 | crates существуют, orchestration god-file |
| Provider/Connector/Parser/Mapper | 🔴 | один `ProviderAdapter`; parser inline |
| Capability system | 🟡 | enum/DTO есть, UI почти не driven by it |
| Несколько fetch methods/fallback | 🔴 | одна strategy на реализованный product |
| Реальные источники без fabrication | 🟡 | runtime честный; demo fallback может маскировать error |
| Codex quota + history | 🟡 | local JSONL only, incomplete semantics |
| ChatGPT subscription | 🔴 | blocked card |
| OpenAI API cost/usage | 🟡 | costs yes; usage/tokens/pagination no |
| Claude Code history/quota | 🟡 | history yes; auth/quota no |
| Claude API | 🔴 | нет adapter |
| Antigravity | 🔴 | discovery-only |
| Z.ai Personal/Team/Coding/API | 🔴 | discovery-only |
| OpenCode local / Go / Zen separation | 🔴 | только Zen placeholder |
| Multi-account | 🟡 | OpenAI API only; local identity isolation absent |
| SQLite history/migrations | ✅ | schema v2 + tests |
| Incremental JSONL import | 🟡 | offsets yes; repeated glob; replacement edge cases |
| Today/Yesterday/30 days/timezone | ✅ | local boundaries converted to UTC |
| Reported/estimated/balance/quota separation | ✅ | domain/storage fields distinct |
| Double-count protection | 🟡 | DB unique + some cost rules; token/source overlaps unresolved |
| Refresh timeout/cooldown | 🟡 | timeout and 429 cooldown; no connected backoff/cancel/parallel |
| Stale-while-revalidate | 🔴 | cache exists, but failures replace live provider values |
| Secrets in Keychain | 🟡 | own OpenAI keys yes; host abstraction/IPC scope incomplete |
| Host APIs | 🔴 | direct OS access from provider/orchestrator |
| Menu bar/panel/settings/history | 🟡 | broad UI exists; settings partly inert |
| Dark/light/Russian | 🟡 | Russian yes; explicit theme inert |
| Launch login/notifications/shortcut | 🟡 | implemented, little integration coverage |
| Intel support | 🟡 | CI build; no runtime smoke/signing |
| Tests/CI | 🟡 | unit baseline good; integration/UI/security gaps |
| Signed updater/release | 🔴 | updater false, signing identity null |

## 11. Сравнение с CodexBar

CodexBar применяет descriptor registry, ordered fetch strategies, explicit Host APIs, source-attempt outcomes, account-scoped routing, incremental local cost caches и adaptive refresh. Его зрелость полезна как набор архитектурных паттернов, но значительная часть sources — browser sessions/private endpoints/local RPC — несёт schema, ToS, privacy и maintenance risks.

### Стоит перенести идею

- **ProviderDescriptor** как single source of truth для identity, labels, icon/color, supported products, capabilities, settings schema, allowed hosts and ordered source modes. Это устранит hard-coded registries в Rust и Svelte.
- **FetchStrategy pipeline** с `is_available`, `fetch`, `should_fallback`, terminal/non-terminal errors и diagnostics attempts. Для Codex: App Server/local snapshot; для Antigravity: app/agy/IDE/OAuth; при этом каждая risky strategy opt-in/feature-gated.
- **Host APIs**: `CredentialHost`, `KeychainHost`, `HTTPHost`, `LocalFileHost`, `ProcessHost`, `PTYHost`, optional `BrowserSessionHost`, `LoggerHost`. Providers должны получать scoped handles, а не `std::fs`, env, keyring/reqwest напрямую.
- **Codex App Server** как primary live quota strategy с bounded lifecycle/kill-on-timeout; JSONL оставить history/fallback, не auth source.
- **Claude incremental cache semantics**: file identity, replacement rebuild, append offset, dedup streaming chunks by stable message/request identity, cache creation tokens.
- **Adaptive refresh** как pure policy + coalesced loop.
- **Per-source diagnostics**: attempts, versions, safe schema fingerprint, last success/error, selected source.
- **Local cost cache** отдельно от provider quota snapshot; Usage.ai SQLite может выполнять эту роль лучше JSON cache.

### Уже сделано у нас лучше или безопаснее

- SQLite normalized records и migrations лучше подходят для Usage.ai history/reporting, чем provider-specific JSON caches.
- Decimal money + explicit `CostKind`/`Coverage` и account/product/source unique keys — хорошая domain basis.
- Собственные API keys находятся в Keychain; CodexBar docs допускают raw config-backed keys. Это не следует ухудшать.
- HTTP exact-host allowlist и no-secret-redirect policy уже выражены явно.
- Blocked providers не фабрикуют значения.

### Не нужно переносить сейчас

- SwiftUI/AppKit-specific implementation, widgets, CLI server, confetti/status-page polling;
- browser cookie importer/WebView scraping до появления явного product requirement and consent/security model;
- огромный compile-time provider enum со всеми CodexBar providers — Usage.ai нужен небольшой data-driven registry для согласованного scope;
- local localhost dashboard/server.

### Опасно или слишком private

- ChatGPT `backend-api/wham/usage`, web dashboard scraping and browser cookies;
- OpenCode `/_server` function hashes and JavaScript-response regex;
- Antigravity internal RPC/LSP and `v1internal:retrieveUserQuotaSummary` OAuth behavior;
- undocumented Z.ai quota variations/headers without captured Personal/Team fixtures and official contract;
- reading/refreshing credentials owned by other apps. Any such integration needs source classification, opt-in, strict read-only credential policy, fixtures, schema fingerprint and kill switch.

## 12. Technical Debt P0–P3

### P0 — исправить до дальнейшего provider development

1. Разделить descriptor/strategy/parser/mapper и Host APIs; прекратить рост `src-tauri/src/lib.rs`.
2. Сделать per-provider last-known-good snapshots и stale-while-revalidate; не заменять данные пустой error card.
3. Устранить двойной OpenAI costs request и добавить pagination/idempotent page fetch.
4. Доказать и исправить Codex/Claude token dedup/cumulative semantics на реальных redacted fixtures; добавить file replacement identity.
5. Убрать silent demo fallback на IPC error; показывать explicit fatal/error state.
6. Включить `usage-ai-app` в clippy CI и добавить refresh integration tests.

### P1 — начало v1.1

1. Account/product/source identity model для local clients; account-switch detection and isolation.
2. Coalesced parallel refresh с per-strategy timeout, cancellation, persistent backoff/cooldown.
3. Scoped `LocalFileHost` + user-selected paths; `CODEX_HOME`, archived sessions, Claude alternate roots.
4. Codex App Server strategy and credits/identity, отдельно от OpenAI API billing.
5. Capability-driven UI/settings registry; исправить multi-account anchors.
6. Применить theme/menu-bar metric settings или удалить их до реализации.
7. Provider diagnostics attempts + actionable warnings/import errors.
8. Security permissions/file modes/export save dialog.

### P2 — желательно

1. Adaptive refresh/power/network observers/reset proximity.
2. Batch SQLite transactions, query consolidation, indexed retention cleanup.
3. Arm64/universal CI, signed/notarized release and update strategy.
4. Broader UI/integration/migration/DST tests.
5. Per-provider/source enablement and feature flags for private APIs.

### P3 — позже

1. Widget/CLI/status pages and richer charts.
2. Optional browser-based sources after threat model and consent UX.
3. Advanced budgets/currency conversion (with explicit FX provenance).

## 13. Что необходимо изменить в архитектуре перед v1.1

Минимальная обязательная реорганизация:

```text
ProviderDescriptor
  identity + products + branding + capabilities + settings schema
  fetch plan -> [FetchStrategy]

FetchStrategy
  availability(context)
  fetch(context) -> FetchOutcome(snapshot/records, source diagnostics)
  fallback policy(error)

Provider package
  descriptor.rs
  strategies/*.rs
  connector.rs       # transport/process/file protocol only
  parser.rs          # pure, fixture-tested
  mapper.rs          # source DTO -> domain model

HostServices
  credentials/keychain/http/files/process/pty/browser/logger/clock/network

RefreshCoordinator
  coalescing + parallel limit + cancellation + retry/backoff
  per-account last-known-good cache + persistent metadata
  transactional storage commit
```

`src-tauri` после этого остаётся composition/platform layer: создаёт HostServices, registry, coordinator, exposes narrow IPC and renders tray/window. Svelte получает descriptors + snapshots and не содержит product switch chains.

Account identity следует сделать составной и явной: provider → product → account → source connection. Local CLI active account нельзя автоматически считать тем же account после login switch. При каждом source result identity fingerprint сравнивается с stored account; mismatch либо создаёт/выбирает другой account, либо блокирует merge. History records сохраняют account observed at import; неизвестная identity маркируется отдельным local-unattributed account, а не `Аккаунт 1`.

## 14. Что можно оставить без изменений

- Rust/Tauri/Svelte/SQLite стек.
- Основные domain structs и принцип missing ≠ zero.
- `CostKind`, `Coverage`, `Freshness`, decimal money and currency separation.
- SQLite unique upserts/checkpoint concept and migration transaction/backup.
- OpenAI exact-host HTTPS/no-redirect HTTP safety policy.
- Keychain-only policy для Usage.ai-owned secrets.
- Russian-first compact panel, accessory/tray shell and local-first/no-telemetry direction.
- Honest `UnsupportedSource` state до появления verified connector.

Оставить можно именно концепции и большую часть кода этих областей; отдельные edge cases, перечисленные выше, всё равно требуют тестов/малых исправлений.

## 15. Рекомендуемая архитектура v1.1

Рекомендуется эволюционная, не wholesale rewrite архитектура:

1. `usage-core`: сохранить models; добавить typed provider/product/source IDs, descriptors, strategy outcomes, error taxonomy, clock/network abstractions and refresh policy.
2. `usage-host`: новый crate с narrow host traits и Tauri/macOS implementations. Secret/file/process permissions scope by descriptor.
3. `usage-providers/<provider>`: один folder per provider, отдельно pure parsers/mappers and strategies. Shared transports only through hosts.
4. `usage-storage`: repositories split by account/history/snapshot/checkpoint; per-provider last-known-good snapshots; source attempts; transactional batches; complete retention.
5. `usage-runtime`: coordinator/registry outside Tauri, fully testable with fake hosts; bounded parallelism and SWR.
6. `src-tauri`: IPC/native lifecycle only.
7. `src`: descriptor/capability-driven UI; explicit loading/empty/stale/error/demo states; account-aware stable element IDs.

Порядок развития providers после foundation: (1) укрепить Codex JSONL + App Server; (2) укрепить Claude JSONL; (3) завершить official OpenAI API usage/cost; затем выбирать Antigravity/Z.ai/OpenCode по наличию проверяемых fixtures и допустимого auth model. Не следует одновременно добавлять все private sources до появления Host APIs, per-source diagnostics и kill switches.

## Проверенные первоисточники CodexBar

- Provider authoring guide: `docs/provider.md` — descriptors, strategies, registries, Host APIs.
- Architecture: `docs/architecture.md`.
- Refresh loop: `docs/refresh-loop.md` — adaptive policy/coalescing.
- Codex: `docs/codex.md`, `docs/providers.md`, `docs/codex-oauth.md` — OAuth, App Server RPC, local cost scan.
- Claude: `docs/claude.md` — incremental caches and streaming dedup.
- Antigravity: `docs/antigravity.md` — app/agy/IDE/OAuth and dynamic quota groups.
- z.ai: `docs/zai.md` — global/CN endpoints and Personal/Team context.
- OpenCode: `docs/opencode.md` — Zen web source and OpenCode Go SQLite.

Эти материалы подтверждают техническую осуществимость подходов, но не превращают undocumented/private endpoints в стабильные или автоматически допустимые источники для Usage.ai.
