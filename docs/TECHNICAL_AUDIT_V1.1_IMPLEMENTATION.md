# Usage.ai — Technical Audit of Master Spec v1.1 Implementation

> Historical baseline audit. Remediation work and current verification are recorded in [`P0_REMEDIATION_REPORT.md`](../P0_REMEDIATION_REPORT.md); this document is intentionally retained as the pre-remediation finding set.

Дата аудита: 2026-09-08  
Проверенный документ: `/Users/nuras/Downloads/Usage.ai_MASTER_SPEC_v1.1.md` (полностью, разделы 0–54)  
Reference only: [CodexBar](https://github.com/steipete/CodexBar)  

## 1. Audit metadata

| Поле | Значение |
|---|---|
| Git branch | `main` |
| Commit | `b4c04de3717fd81a81ad0576f0fc5a5e36fdf8e4` |
| Tag at HEAD | отсутствует; ближайший baseline tag `v1.0` у `8fc2e34` |
| Working tree перед аудитом | clean |
| Rust / Cargo | `rustc 1.98.1`, `cargo 1.98.1` |
| Node / npm / pnpm | `v25.9.0` / `11.12.1` / `11.23.0` |
| Target | macOS 13+, Tauri `.app` + `.dmg`; CI first target `x86_64-apple-darwin` на `macos-13` |
| Версии приложения | Cargo workspace `1.0.0`; `package.json` и `tauri.conf.json` — `0.1.0` |
| Изменения во время аудита | исходный код не изменялся; добавлен только этот отчёт |

Несовпадение версий является отдельным release defect: backend/UI, package manifest и Tauri bundle не описывают одну версию.

## 2. Executive Summary

**Итог: реализация не соответствует Master Spec v1.1, обязательный P0 Stabilization Gate не пройден, расширять provider integrations сейчас нельзя.**

Сильные стороны текущей ревизии:

- workspace разделён на `usage-core`, `usage-host`, `usage-providers`, `usage-runtime`, `usage-storage` и тоньше организованный Tauri слой;
- portable CI проверяет весь workspace, Intel job собирает x86_64 `.app`/`.dmg`;
- clippy, Rust tests, frontend tests, Svelte check и production build проходят;
- packaged app не подменяет backend error демо-метриками;
- OpenAI production strategy получает snapshot и cost records из одного logical fetch;
- last-known-good snapshot сохраняется и при ошибке возвращается как stale;
- SQLite использует WAL, migrations, unique upsert keys и транзакционный provider refresh bundle.

Но correctness gate нарушен в местах, где цена ошибки — ложные usage/cost/account history:

1. Все default и custom roots одного локального продукта импортируются в один stable default account (`src-tauri/src/refresh_flow.rs:615-681`). Комментарий обещает отдельную атрибуцию custom accounts, но отдельного цикла импорта нет.
2. Переключение реального Codex/Claude аккаунта не определяется: identity check получает наблюдаемую identity только из fingerprint OpenAI API secret (`crates/usage-runtime/src/refresh.rs:495-535`). Локальная история продолжает записываться в один deterministic account.
3. При replacement/rotation JSONL старые строки не удаляются. Тест прямо закрепляет неправильное поведение: после замены файла обе версии coexist (`crates/usage-runtime/src/history_import.rs:123-167`). Это может завышать usage.
4. OpenAI Costs pagination реализована по `next_cursor`/`starting_after`, тогда как официальный контракт использует response `next_page` и query `page` (`crates/usage-providers/src/openai_api.rs:54-66,159,194`). Многостраничная история будет неполной.
5. Production refresh calls не coalesce: каждый `refresh_many` сначала увеличивает generation, а inflight key включает generation (`crates/usage-runtime/src/refresh.rs:170-218`). Тест coalescing вручную даёт двум scope один generation и не моделирует два реальных trigger.

Поэтому успешные сборки не являются доказательством корректности данных.

## 3. P0 Stabilization Gate

| P0 | Статус | Результат проверки |
|---|---|---|
| P0-1 OpenAI one-fetch | 🟡 | Production `OpenAiCostsStrategy` переиспользует один payload для snapshot/history. Однако pagination несовместима с официальным API, а legacy adapter API всё ещё допускает два отдельных fetch path. One-fetch формально достигнут, корректность полного результата — нет. |
| P0-2 SWR | 🟡 | LKG хранится и возвращается stale при refresh error. Но `ProviderDto` не несёт отдельную текущую source error; `outcome_to_dto` при snapshot восстанавливает старый connected state. Required состояние «old data + stale + current error» в UI не выполнено полностью. Retention также может удалить единственный LKG до fallback. |
| P0-3 Demo integrity | ✅ | В Tauri IPC/backend error пробрасывается как error; demo возвращается только вне Tauri. UI показывает явный баннер `ДЕМО` (`src/lib/api.ts:27-35`, `src/App.svelte:312`). |
| P0-4 Codex semantics | 🔴 | Нет real/redacted fixture corpus и DB-level доказательства delta/cumulative/dedup. Единственный fixture синтетический; тест repeated import лишь сравнивает два parsed struct. `archived_sessions` не сканируется; account switch не определяется; replacement оставляет старые записи. |
| P0-5 Claude semantics | 🔴 | Parser читает `message.id`, `requestId`, UUID, model, input/output/cache fields и даёт streaming chunks одинаковый ID, но fixtures/test не доказывают cumulative semantics в storage. Alternate roots отсутствуют; replacement сохраняет старые rows; custom roots смешиваются. |
| P0-6 Runtime decomposition | 🟡 | Crates, descriptor, strategy trait и runtime существуют. Но нет обязательных Process/PTY hosts; FilesHost/KeychainHost не capability-scoped; локальные adapters обходят hosts через `std::fs`, `glob`, env; Connector/Parser/Mapper остаются сцеплены; `refresh_flow.rs` всё ещё содержит orchestration и provider branching. |
| P0-7 CI | ✅ | `cargo clippy --workspace --all-targets -- -D warnings` проходит локально и есть в portable CI; основной app crate не исключён. |
| P0-8 Runtime integration tests | 🟡 | Есть in-memory flow tests для strategy→parse→transactional commit→DTO-like outcome и SWR. Нет production-trigger coalescing test, restart cooldown test, real local fixtures through FilesHost, account switching, atomic import+checkpoint, replacement rebuild и current-error UI contract. |

**P0 Gate: FAIL.** Пять требований из восьми не имеют статуса ✅; два из них (`P0-4`, `P0-5`) не выполнены.

## 4. Architecture compliance

Цепочка `ProviderDescriptor → ordered FetchStrategy[] → Connector → Parser → Mapper → domain` существует только частично.

- `ProductDescriptor` централизует identity, branding, capabilities, account model, allowed hosts и local discovery (`crates/usage-providers/src/descriptor.rs:18-35`).
- Strategy order остаётся отдельным hard-coded match (`crates/usage-runtime/src/registry.rs:15-21`), а construction — ещё в runtime refresh. Descriptor не содержит strategy IDs/order, settings schema, source-specific capabilities и полноценной diagnostics policy.
- UI получает descriptor DTO, но содержит fallback product mapping и icon switches (`src/App.svelte:118,181-183`). Backend overview также жёстко знает `codex`/`claude-code` (`src-tauri/src/refresh_flow.rs:480-491`).
- `FetchStrategy` хорошо отделяет availability/execute/classification/fallback, но transport/parser/mapper для Codex/Claude остаются внутри adapters.
- `src-tauri/src/lib.rs` стал composition-only, однако `src-tauri/src/refresh_flow.rs` (около 700 строк) по-прежнему выполняет import, account creation, request construction, aggregation, notification/tray preparation и product-specific branching, что нарушает запрет раздела 7.1.

Добавление нового работающего provider по-прежнему требует синхронных правок descriptor, registry match, strategy factory/runtime, refresh assembly и местами UI/backend labels. Single source of truth не достигнут.

## 5. HostServices audit

| Host | Статус | Наблюдение |
|---|---|---|
| ClockHost | ✅ | Есть и используется runtime. |
| NetworkHost | ✅ | Наблюдаемое frontend online/offline state поступает в runtime. |
| LoggerHost | 🟡 | Structured redaction есть, но coverage diagnostics ограничено. |
| HTTPHost | 🟡 | HTTPS, exact host allowlist, no redirects, response cap и UA есть. Но legacy `providers/http.rs` делает direct reqwest; `Retry-After` поддерживает только seconds. |
| KeychainHost | 🟡 | Generic frontend secret API удалён, typed commands есть. Но trait разрешает любой service/account, то есть capability boundary не enforced самим host. Secret дополнительно копируется в обычный `Vec<u8>`/request context. |
| LocalFileHost | ⚠️ | Discovery защищает root/canonicalization/symlink escape. Но `read_range`/`read_tail` принимают raw `Path`, а не выданный `ScopedPath`, и могут читать произвольный файл. Production Codex/Claude adapters вообще обходят host. |
| ProcessHost | 🔴 | Отсутствует. |
| PTYHost | 🔴 | Отсутствует. |
| BrowserHost | N/A | Не нужен текущим официальным/local sources; browser cookie integrations не реализованы. |

Вывод: Host API skeleton существует, но не выполняет главный security-инвариант «provider может делать только разрешённые host operations».

## 6. Refresh/runtime audit

Положительно:

- bounded parallelism реализован semaphore + `JoinSet`;
- strategy fallback различает terminal/non-terminal errors;
- timeout, retry/backoff, offline suppression и LKG fallback присутствуют;
- transactional provider result commit есть;
- fixed refresh and manual trigger прокладываются в production path.

Нарушения:

- **Coalescing не работает между реальными triggers.** `refresh_many` всегда вызывает `bump_generation`; inflight key — `(generation, scope)`. Два overlapping manual/scheduled/tray вызова имеют разные keys и могут оба сходить к source.
- **Cancellation является generation poll**, проверяемым лишь перед очередной strategy. Нет cancellation token для in-flight HTTP/file/process operation, shutdown и disabled source.
- **Persistent cooldown не восстанавливается после restart.** `Coordinator::new` вызывает restore до создания scope states; restore перебирает пустой `states`. Позднее создание scope DB cooldown не подхватывает (`crates/usage-runtime/src/refresh.rs:103-143`).
- Scheduler — обычный sleep loop; native sleep/wake event отсутствует. Frontend `visibilitychange` помогает только при ожившем webview (`src-tauri/src/platform/lifecycle.rs:11-28`, `src/App.svelte:65-75`).
- History import выполняется перед каждым full refresh и не участвует в coordinator coalescing.
- Attempt timings неверны: фактические timestamps execute не передаются в `record_attempt`; `started_at == finished_at == now` (`crates/usage-runtime/src/refresh.rs:626-655`).

## 7. Storage/history audit

Реализовано:

- schema v4, WAL, migrations, pre-migration backup;
- usage/cost unique upsert keys;
- snapshot/source-attempt/cooldown/checkpoint tables;
- provider refresh bundle commit транзакционен;
- device/inode + shrink detection сбрасывает offset.

Критические проблемы:

- `import_usage_batch` и `set_checkpoint` — две отдельные транзакции/операции (`crates/usage-runtime/src/history_import.rs:68-95`). Crash boundary после rows до checkpoint нарушает требование atomic batch+checkpoint.
- Replacement сбрасывает offset, но не удаляет records, полученные из предыдущего file identity. Тест ожидает count `2` после замены одного-line файла (`history_import.rs:123-167`). Требуется file provenance и transactional rebuild rows для этого source file.
- Codex IDs вида path-hash/offset оставляют старый tail при замене на более короткий/иной файл; Claude IDs могут collide across roots или сохранять уже удалённые turns.
- Retention удаляет все snapshots старше cutoff, включая последний LKG (`usage-storage/src/lib.rs:438-480`). Поскольку prune идёт после history import и перед provider refresh, длительно недоступный provider может потерять fallback именно перед ошибкой.
- DB file и WAL sidecars не получают явный user-only chmod, хотя settings/export получают `0600`.
- Usage/cost referential integrity к account/product недостаточна; repository separation по spec не завершена.

## 8. Account/source identity audit

Typed routing helper сам по себе аккуратно различает confidence и mismatch. Production wiring не обеспечивает наблюдаемую identity:

- OpenAI API использует fingerprint secret как weak identity, а не organization/project identity; rotation ключа того же org выглядит как account mismatch.
- Codex и Claude передают `observed = None`, route получается `Unattributed`, после чего refresh всё равно commit'ит данные в stable account.
- Default local accounts создаются детерминированно по product (`refresh_flow.rs:57-80`), поэтому смена логина незаметно смешивает историю.
- В `import_all_history` default и все managed custom roots объединяются, после чего один вызов `import_product` получает stable default account (`refresh_flow.rs:624-681`). Обещанная в комментарии separate attribution отсутствует.
- `Account` не содержит полноценной пары product/source/billing scope identity; значительная часть domain по-прежнему использует свободные `String`/UUID.

Это два независимых BLOCKER-сценария account mixing.

## 9. Provider-by-provider audit

| Product | Статус | Фактически работает | Основные gaps |
|---|---|---|---|
| Codex | ⚠️ unsafe partial | Local sessions JSONL, tail quota extraction, token history/model when present | Нет App Server strategy, archived sessions, доказанной delta/cumulative семантики, account identity, real fixtures; raw filesystem bypass; replacement inflation. |
| Claude Code | ⚠️ unsafe partial | Local `~/.claude/projects/**/*.jsonl`, token/cache fields, streaming stable ID heuristic | Нет alternate roots (`CLAUDE_CONFIG_DIR`, `.config/claude`, Desktop), quota sources, identity; DB streaming test отсутствует; replacement/custom-root mixing. |
| OpenAI API | ⚠️ unsafe partial | Official Costs endpoint, one logical fetch, reported cost/project records, Keychain setup/test | Pagination contract неверен; Organization Usage endpoints отсутствуют; secret fingerprint ≠ org identity; f64 money parsing; possible record-key collision for same bucket/scope/line item. |
| ChatGPT consumer | Discovery only | Descriptor/UI placeholder | Источник отсутствует — метрик нет, что честно. |
| claude.ai consumer | Discovery only | Descriptor/UI placeholder | Источник отсутствует — метрик нет. |
| Antigravity | Discovery only | Descriptor/UI placeholder | Ни LSP/local probe, ни kill-switch strategy нет. |
| Gemini API | Discovery only | Descriptor/UI placeholder | OAuth/official quota source отсутствует. |
| Z.ai GLM Coding | Discovery only | Descriptor/UI placeholder | Нет разделения coding plan/API/team context и working source. |
| OpenCode/Zen | Discovery only | Только descriptor `zen` | Нет обязательного разделения OpenCode local / Go / Zen и double-count protection. |

CodexBar использовался только для cross-check архитектурных вариантов. Его текущая документация подтверждает полезные patterns: ordered source strategies, Codex App Server RPC, sibling `archived_sessions`, Claude alternate roots и rebuild rows при replacement. Это не превращает private endpoints в контракт Usage.ai и не меняет оценки источников: [provider overview](https://github.com/steipete/CodexBar/blob/main/docs/providers.md), [Claude history notes](https://github.com/steipete/CodexBar/blob/main/docs/claude.md), [provider architecture](https://github.com/steipete/CodexBar/blob/main/docs/provider.md).

## 10. Security audit

Положительно: no shell plugin, no arbitrary browser cookie import, narrow typed Tauri commands, Keychain storage, HTTPS/host allowlist/no redirects, redacted exports/log fields, CSP и explicit export destination.

Оставшиеся риски:

- FilesHost не выдаёт non-forgeable scoped capability; provider может вызвать raw read path.
- Local adapters обходят HostServices и читают filesystem/env напрямую.
- KeychainHost policy не ограничивает app service/account namespace на trait boundary.
- Custom path принимается через IPC как произвольный absolute path; нет explicit native folder grant/security-scoped bookmark и нет enforcement descriptor `allow_custom_path` в account update path.
- SQLite DB/WAL permissions явно не затягиваются до user-only.
- Secret удаляется async до/рядом с DB account deletion без durable recovery; при сбое возможен orphaned Keychain item.
- Нет process/PTY host, следовательно будущие CLI integrations пока нельзя безопасно добавлять.

## 11. UI/macOS audit

- Capability-driven rendering в основном выполнен; descriptor list поступает с backend.
- Live backend failure показывает error/retry, а не fake metrics; demo имеет явный баннер.
- Stable UI selection key включает provider/product/account.
- Theme, menu-bar metric и cost period работают.
- SWR card не показывает current source error отдельно от старого connected snapshot.
- Hard-coded product fallback, icons, labels/colors означают неполный descriptor-driven UI.
- Panel position clamp использует `window.current_monitor()` до переноса к click point (`src-tauri/src/platform/window.rs:24-50`); при клике на другом monitor может выбрать bounds прежнего экрана.
- Shortcut opening без click position не гарантирует cursor/current-monitor-aware placement.
- Native sleep/wake observer отсутствует; visibility event не эквивалентен lifecycle integration.
- Intel CI проверяет architecture artifact, но нет smoke launch/menu-bar/tray/window test.

## 12. Performance audit

- Coordinator ограничивает параллелизм; UI timer дешёвый; SQLite aggregations приемлемы для малой базы.
- Каждый full refresh сначала glob/сканирует local history; quota adapters дополнительно делают собственный raw glob/tail scan.
- Синхронный filesystem I/O находится внутри async provider adapters.
- History roots обрабатываются последовательно и custom roots повторно входят в общий scan.
- Новый HTTP client создаётся на fetch/page path вместо shared pool.
- Snapshot/history retention не является bounded cache strategy для всех tables.
- Adaptive refresh (P2) отсутствует, что допустимо до закрытия correctness, но fixed refresh не должен дублировать текущую работу.

## 13. Tests audit

Фактически выполнено:

| Проверка | Результат |
|---|---|
| `npm run check` | PASS, 0 errors / 0 warnings |
| Vitest | PASS, 3 files / 9 tests |
| Vite production build | PASS, 119 modules |
| Rust unit tests | PASS, 87 tests (`app 0`, core 16, host 11, providers 28, runtime 20, storage 12) |
| Rust doc tests | PASS, 0 failures |
| Strict clippy workspace | PASS |

Критические пробелы test evidence:

- нет redacted real-world Codex/Claude fixture corpus;
- нет DB-level cumulative/streaming/dedup invariants;
- нет replacement rebuild expectation (наоборот, тест закрепляет coexistence);
- нет atomic import+checkpoint interruption test;
- нет two-production-trigger coalescing test;
- нет coordinator restart cooldown restore test;
- нет local account switch/custom-root isolation test;
- нет official OpenAI `next_page`/`page` pagination fixture;
- нет migration interruption/partial schema drift/restore test;
- нет UI test «stale values + visible current error»;
- нет security test, запрещающего raw path/keychain namespace access.

## 14. CI/build audit

Portable CI корректно запускает frontend checks/build, strict workspace clippy и workspace tests. Intel job ставит target, запускает workspace tests, собирает Tauri x86_64, проверяет Mach-O architecture и наличие `.app`/`.dmg` (`.github/workflows/ci.yml:20-41`).

Не закрыто:

- запуск собранного app/smoke lifecycle;
- arm64 и universal artifacts;
- signing, notarization, Sparkle/update feed или эквивалент;
- reproducible release version из одного manifest;
- packaged-app integration tests и artifact install/open verification.

Это не мешает локальному P0 development, но не удовлетворяет release/distribution DoD.

## 15. Master Spec compliance matrix

| Группа требований v1.1 | Статус | Краткое заключение |
|---|---|---|
| Product principles / read-only / no-source-no-metric | 🟡 | Read-only соблюдён, discovery placeholders честны; локальные недоказанные семантики всё же могут показывать ложные суммы. |
| Platform/stack | ✅ | Tauri 2 + Rust + Svelte + SQLite, macOS target. |
| Crate/project structure | 🟡 | Crates выделены, но app `refresh_flow` всё ещё business layer. |
| ProviderDescriptor single source | 🟡 | Полезный registry есть, но strategies/settings/diagnostics и UI/backend switches раздвоены. |
| FetchStrategy pipeline | 🟡 | Trait/fallback/attempts есть; production coalescing/cancellation/diagnostics некорректны. |
| Connector/Parser/Mapper | 🟡 | OpenAI parser частично pure; local implementations сцеплены. |
| HostServices | 🔴 | Process/PTY отсутствуют, file/keychain capability boundaries не enforced, production bypass. |
| Domain model / capabilities | 🟡 | Основные structs/capabilities есть; identity/source/billing typing неполно. |
| Account/product/source identity | ⚠️ | Реальные local account switches и custom-root attribution смешивают данные. |
| RefreshCoordinator | ⚠️ | Parallelism/retry/offline есть; coalescing, cancellation и restored cooldown нарушены. |
| SWR / LKG / atomic refresh | 🟡 | Data fallback есть; current error UI и retention safety отсутствуют. |
| History/import/dedup | ⚠️ | Checkpoints/upserts есть; checkpoint не atomic, replacement даёт stale rows. |
| Codex integration | 🔴 | Local prototype, correctness acceptance не доказан. |
| OpenAI API integration | ⚠️ | One-fetch Costs работает частично; pagination wrong, Usage API отсутствует. |
| Claude integration | 🔴 | Local prototype, fixtures/roots/identity/quota incomplete. |
| Antigravity | 🔴 | Не реализован. |
| Z.ai | 🔴 | Не реализован. |
| OpenCode local/Go/Zen | 🔴 | Не реализован и логически не разделён. |
| Consumer browser products | ✅ scoped-out | Нет silent cookie use; placeholders без метрик. |
| Storage/repositories/integrity/recovery | 🟡 | SQLite foundation хорош; replacement, FK, retention, permissions/recovery gaps. |
| Diagnostics | 🟡 | Attempts/import counters есть; timings, selected source, schema fingerprint/warnings unreliable. |
| UI/UX | 🟡 | Основные states хорошие; SWR error и descriptor purity incomplete. |
| macOS lifecycle | 🟡 | Tray/panel/shortcut есть; multi-monitor and sleep/wake incomplete. |
| Security | 🟡 | Narrow IPC/Keychain/HTTP policy есть; host capability enforcement и DB permissions incomplete. |
| Financial semantics | ⚠️ | Reported vs estimated различается, но incomplete pagination и stale replacement rows делают totals ненадёжными. |
| Notifications | 🟡 | Планирование/threshold foundation есть; полнота source semantics зависит от unsafe data. |
| Refresh modes | 🟡 | Manual/fixed есть; adaptive P2 отсутствует; fixed trigger duplication possible. |
| Performance | 🟡 | Bounded network concurrency; redundant scans/blocking I/O остаются. |
| Testing strategy | 🟡 | 96 automated tests проходят, но mandatory semantics/integration fixtures отсутствуют. |
| CI | 🟡 | P0 clippy/Intel build закрыты; smoke/release chain нет. |
| Release/update | 🔴 | Version mismatch; signing/notarization/updater/release validation отсутствуют. |
| P0 Stabilization Gate | 🔴 | FAIL. |
| Full Master Spec v1.1 DoD | 🔴 | FAIL. |

## 16. New issues found beyond explicit Master Spec checks

1. **OpenAI pagination schema mismatch.** Официальная документация Costs определяет query `page` и response `next_page`; implementation/tests используют `starting_after`/`next_cursor`. См. [OpenAI Usage/Costs API reference](https://platform.openai.com/docs/api-reference/usage/audio_transcriptions_object).
2. **Cooldown restore constructor-order bug.** Persistence есть на уровне таблицы, но restart restoration фактически no-op из-за пустого state map.
3. **Attempt duration fabricated as zero.** Diagnostics записывает одинаковые timestamps вместо измеренного interval.
4. **Comment/implementation divergence.** `import_all_history` утверждает separate custom attribution, но объединяет roots в default account. Это повышает риск ложной уверенности при review.
5. **Version split-brain.** UI/backend и installer могут показывать разные версии (`1.0.0` против `0.1.0`).
6. **Descriptor advertises allowed hosts without enforcing them.** Metadata выглядит capability-safe, но runtime не строит per-provider host facade.
7. **Retention can erase the only recovery state.** Последний LKG не защищён от age-based snapshot deletion.

## 17. Findings by severity

### BLOCKER

- **B-01 — Custom/default local histories are attributed to one account.** Impact: cross-account mixing and false totals. Evidence: `refresh_flow.rs:624-681`.
- **B-02 — Local client account switch is not observed.** Impact: history from sequential Codex/Claude logins merges into deterministic account. Evidence: `refresh.rs:495-535`, `refresh_flow.rs:57-80`.

### P0

- **P0-01 — Replacement preserves deleted old JSONL records.** Impact: inflated history/cost. Evidence: `history_import.rs:123-167`.
- **P0-02 — OpenAI Costs pagination uses the wrong cursor contract.** Impact: silent incomplete reported cost. Evidence: `openai_api.rs:54-66,159,194` and official API reference.
- **P0-03 — Real triggers do not coalesce.** Impact: duplicate requests/rate limiting/races. Evidence: `refresh.rs:170-218,846-897`.
- **P0-04 — History rows and checkpoint are not atomic.** Impact: inconsistent restart/import state. Evidence: `history_import.rs:68-95`.
- **P0-05 — Codex token semantics are not proven.** Impact: delta/cumulative double count possible.
- **P0-06 — Claude streaming semantics are not proven end-to-end.** Impact: duplicate/cumulative totals possible.
- **P0-07 — SWR current error is not represented in provider UI DTO.** Impact: stale connected-looking data hides an active failure.
- **P0-08 — Mandatory capability hosts are absent/bypassed.** Impact: new CLI/private providers cannot be added within the specified security model.
- **P0-09 — Cooldown restoration is ineffective after restart.** Impact: rate-limited source is retried too soon.

### P1

- Preserve newest LKG per account/product during retention.
- Make diagnostics timings/source/schema fingerprint/warnings truthful.
- Add alternate Claude roots and Codex `archived_sessions` only after fixture proof.
- Enforce `allow_custom_path`, native user grant and scoped read token.
- Apply `0600`/user-only policy to DB and sidecars.
- Move remaining orchestration/provider switches out of `src-tauri`.
- Make descriptor own strategy/settings/diagnostic metadata and remove UI/backend switch duplication.
- Add FK/integrity and repository boundaries where they improve migration/recovery safety.
- Correct multi-monitor positioning and add native sleep/wake handling.

### P2

- Shared HTTP client/pooling, indexed incremental discovery and removal of redundant scans.
- Adaptive refresh only after correctness and coalescing are proven.
- arm64/universal build matrix and packaged-app smoke automation.

### P3

- Finish branding/icon descriptor purity and eliminate presentation fallbacks.
- Consolidate version display and release metadata ergonomics.

## 18. Final verdict

**D — UNSAFE / DATA INTEGRITY RISK**

Причина категории D — не количество незавершённых providers. Есть воспроизводимые архитектурные paths, способные показать пользователю неверные данные:

- account histories объединяются;
- заменённые transcript rows продолжают учитываться;
- OpenAI multi-page reported costs могут тихо обрезаться;
- stale snapshot скрывает текущую ошибку;
- overlapping triggers не гарантируют один source fetch.

Следовательно:

- соответствие Master Spec v1.1: **нет**;
- P0 Stabilization Gate: **не выполнен**;
- переход к новым provider integrations: **запрещён до remediation и повторного gate-аудита**;
- release/shared distribution: **не готов**.

## 19. Exact next actions

Выполнять именно в таком порядке:

1. **Заморозить provider expansion.** Не добавлять Antigravity/Z.ai/OpenCode/browser sources.
2. **Исправить account attribution.** Импортировать каждый default/custom root в явно выбранный account; ввести observed local identity. При unknown/mismatch не commit'ить в прежний account: создать unverified candidate/запросить подтверждение согласно spec.
3. **Исправить replacement transaction.** Добавить source-file provenance; при identity change в одной транзакции удалить старые rows этого файла, импортировать новый content и обновить checkpoint.
4. **Объединить batch+checkpoint.** Один storage API/transaction; добавить fault-injection test между parse и commit.
5. **Исправить OpenAI pagination.** Читать `next_page`, отправлять `page`, добавить official-shape multi-page fixture и assertion одного logical refresh. Парсить decimal без промежуточного `f64`.
6. **Исправить production coalescing.** Scope-level shared task не должен зависеть от generation конкретного trigger. Добавить тест двух concurrent `refresh_many` и assertion одного adapter call.
7. **Сделать cancellation/cooled restart реальными.** Cancellation token в transport/file/process boundary; восстановление cooldown при lazy scope creation; restart integration test.
8. **Закрыть SWR contract.** DTO должен одновременно нести LKG values, `stale=true` и current error/source attempts. Retention обязан сохранять newest LKG per scope.
9. **Построить доказательства Codex.** Redacted fixtures: delta/cumulative, multiple files, archived sessions, truncate/replace, CODEX_HOME, model variants, duplicates; end-to-end storage totals.
10. **Построить доказательства Claude.** Redacted streaming chunks, repeated message/request/UUID, cache create/read, out-of-order/duplicate, alternate roots, replace; end-to-end storage totals.
11. **Закрыть HostServices security.** `ScopedPath`/opaque grant для reads, per-provider Keychain/HTTP facades, запрет direct filesystem/reqwest/env в adapters; добавить ProcessHost и PTYHost до App Server/CLI strategies.
12. **Очистить architecture boundary.** Перенести import/request/aggregation orchestration из `src-tauri`; descriptor должен определять strategy order/settings/diagnostics; удалить Rust/Svelte provider switches.
13. **Повторить P0 Gate.** Требовать ✅ по всем P0-1…P0-8 и новые regression tests B-01/B-02/P0-01…P0-09.
14. **Только затем продолжить integrations** в порядке Codex → Claude Code → OpenAI Usage → Antigravity → Z.ai → OpenCode families.
15. **После correctness — release work:** единая версия, app smoke, arm64/universal, signing, notarization, update/release mechanism.

До выполнения пунктов 2–10 любые новые provider данные увеличивают blast radius существующих ошибок идентичности и истории.
