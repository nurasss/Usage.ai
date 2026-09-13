# 📊 Usage.ai — Менюбар-утилита учета лимитов и расходов на AI

**Usage.ai** — локальная утилита для строки меню macOS (Menu Bar), которая показывает поддерживаемые сигналы использования, квот и затрат. Production scope v1.3 ограничен OpenAI Codex, Anthropic Claude Code и OpenAI API, когда соответствующий источник настроен. Google Gemini и Ollama находятся только в discovery/backlog scope и не являются production-драйверами.

---

## 📌 Архитектурные принципы

- **Local-first**: Usage.ai не использует собственный backend и telemetry; данные читаются из локальных источников и provider-owned интерфейсов.
- **Zero-inference monitoring**: приложение не выполняет скрытые генеративные запросы ради проверки квоты или баланса. Оно использует provider-owned usage/status interfaces и локальную структурированную историю, без вывода usage из текста ответа модели.
- **Разделение источника и доверия**: documented/provider-owned API, поддерживаемый client/App Server интерфейс и локальная structured history различаются в данных. Для provider-facing totals, budget, tray и notifications authoritative являются только канонические Complete и Partial; Unknown, UnverifiedSemantics и discovery-данные остаются видимыми для локального анализа, но не считаются authoritative.
- **Границы хранения**: доступ к source-owned auth/session stores других приложений read-only. Секреты, принадлежащие Usage.ai, хранятся в Keychain where applicable; usage/history сохраняются в обычной локальной SQLite. Для SQLite database-level encryption не заявляется.
- **Release target**: текущие v1.3 release artifacts собираются только для x86_64-apple-darwin (Intel). Apple Silicon и universal artifact не входят в текущий release gate.

---

## 🧭 Источники по продуктам

| Provider / product | Статус | Usage / quota source | Local history | Примечание |
|---|---|---|---|---|
| OpenAI Codex | Monitored in v1.3 | provider-owned supported-client/App Server strategy; fallback помечается своим source/coverage | Codex local structured history | profile- и trust-aware данные |
| Anthropic Claude Code | Monitored in v1.3 | Claude Code PTY/provider-owned client strategy; read-only OAuth usage strategy where available | Claude local structured history | source и coverage показываются отдельно |
| OpenAI API | Monitored when configured | provider API strategy для применимых usage/cost данных | Not applicable | только для настроенной API account model |
| Google Gemini / Gemini API | Discovery only | None | None | descriptor/discovery metadata; production driver отсутствует |
| Ollama | Discovery/backlog only | None | None | production driver отсутствует |

Другие обнаруженные продукты могут присутствовать как discovery metadata, но это не означает production monitoring или authoritative usage.

---

## 🏗 Архитектура крейтов (Rust Cargo Workspace)

~~~text
usage.ai/
├── src-tauri/              # Точка входа Tauri-приложения
├── crates/
│   ├── usage-core/         # Базовые структуры данных
│   ├── usage-providers/    # Production strategies для Codex, Claude Code и OpenAI API
│   ├── usage-runtime/      # Фоновый сборщик метрик и планировщик опроса
│   ├── usage-storage/      # Локальная SQLite usage/history storage
│   └── usage-host/         # Нативные системные вызовы macOS
├── src/                    # Svelte 5, TypeScript, Vite и lightweight custom CSS
└── Cargo.toml              # Манифест рабочего пространства Rust
~~~

---

## 🛠 Стек технологий

- **Системный бэкенд**: Rust 2021, Tauri Framework
- **Пользовательский интерфейс**: Svelte 5, TypeScript, Vite, CSS variables и lightweight custom UI
- **Локальные данные**: SQLite и rust_decimal; Keychain используется для принадлежащих Usage.ai секретов where applicable
- **Тестирование**: cargo test, vitest и fixtures локальной истории Codex/Claude

---

## 🚀 Сборка и разработка

~~~bash
# Установка зависимостей интерфейса
npm ci

# Проверки интерфейса
npm run check
npm test -- --run
npm run build

# Запуск в режиме разработки Tauri
npm run tauri dev

# Сборка .app / .dmg для текущего release target
npm run tauri build
~~~

## Release qualification

RC-версии являются pre-release candidates. Точные gates и provenance каждого кандидата фиксируются в versioned reports. Native notification, launch-at-login, clean-install GUI и Claude true-subscription positive требуют отдельной environment qualification и не обозначаются здесь как универсальный PASS.
