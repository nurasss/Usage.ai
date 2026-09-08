# Usage.ai

> [English version](README.en.md)

Локальная утилита для menu bar macOS: квоты, использование, расходы и история AI-сервисов в одной панели. Первая цель — Intel (`x86_64-apple-darwin`), macOS 13+.

Приложение никогда не отправляет inference-запросы ради мониторинга. Показываются только те данные, источник, аккаунт, охват и свежесть которых приложение может объяснить.

## Разработка

Требования: Node.js, стабильный Rust, Xcode Command Line Tools, macOS 13 или новее.

```bash
npm install
npm run check
npm test
cargo test --workspace
npm run tauri dev
```

Сборка Intel-приложения и DMG на Intel Mac:

```bash
rustup target add x86_64-apple-darwin
npm run tauri build -- --target x86_64-apple-darwin
file target/x86_64-apple-darwin/release/bundle/macos/Usage.ai.app/Contents/MacOS/usage-ai-app
```

## Текущий статус интеграций (v1.1)

- Codex: стратегия `codex-local-jsonl` — квоты из всех session-хвостов и история токенов как request-delta.
- Claude Code: стратегия `claude-local-jsonl` — история с дедупом `message.id + requestId`; квоты подписки заблокированы (нет подтверждённого источника).
- OpenAI API: один fetch на refresh (snapshot + costs в одной транзакции), bounded-пагинация; активируется типизированной Admin-конфигурацией, иначе честный `NotConfigured`.
- Остальные продукты: `DiscoveryOnly`, метрики не выдумываются (Gate A: новых private-источников до закрытия P0 нет).
- Архитектура: `usage-runtime` (коалесцинг, bounded-параллелизм, отмена, SWR last-known-good, персистентные кулдауны), `usage-host` (scoped доступ к ОС), дескрипторы как единый источник UI.
- Безопасность: типизированный Keychain IPC, Save-диалоги экспорта, файлы 0600, allow-list HTTPS без редиректов с секретом.

См. [Phase 0 discovery](docs/PHASE0_DISCOVERY.md) и [матрицу интеграций](docs/INTEGRATION_MATRIX.md).

## Приватность

Собственные API-секреты — только в macOS Keychain. В SQLite, логах, фикстурах, диагностике и экспорте нет секретов, промптов, ответов, email, браузерных cookie и сырых ответов провайдеров. Локальные файлы сессий читаются инкрементально и никогда не изменяются.
