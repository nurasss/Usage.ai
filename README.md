# 📊 Usage.ai — Менюбар-утилита учета лимитов и расходов на AI

**Usage.ai** — локальная утилита для строки меню macOS (Menu Bar), обеспечивающая централизованный мониторинг квот, расхода токенов, баланса подписок и затрат по всем ключевым провайдерам искусственного интеллекта: OpenAI Codex, Anthropic Claude, Google Gemini, Ollama и другим.

---

## 📌 Архитектурные принципы

- **Zero-Inference Monitoring**: Приложение никогда не отправляет скрытых генеративных запросов к нейросетям ради проверки баланса. Используются исключительно официальные API квот и парсинг локальных логов сессий.
- **Поддержка Intel Mac и Apple Silicon**: Скомпилировано под архитектуры `x86_64-apple-darwin` и `aarch64-apple-darwin` на macOS 13+.
- **Безопасность (P0 Remediation)**: Успешно пройдены 4 раунда технического аудита безопасности и предотвращения утечек учетных данных.

---

## 🏗 Архитектура крейтов (Rust Cargo Workspace)

```text
usage.ai/
├── src-tauri/              # Точка входа Tauri приложения (управление треем и окном меню)
├── crates/
│   ├── usage-core/         # Базовые структуры данных (Account, TokenQuota, SpendRecord)
│   ├── usage-providers/    # Драйверы провайдеров (OpenAI, Anthropic, Gemini, Ollama)
│   ├── usage-runtime/      # Фоновый сборщик метрик и планировщик опроса
│   ├── usage-storage/      # Локальное зашифрованное хранилище токенов и кэша
│   └── usage-host/         # Нативные системные вызовы macOS
├── src/                    # Фронтенд на Svelte 5, TypeScript и Tailwind CSS
├── docs/                   # Отчеты аудита безопасности P0
└── Cargo.toml              # Манифест рабочего пространства Rust
```

---

## 🛠 Стек технологий

- **Системный бэкенд**: Rust 2021, Tauri Framework
- **Пользовательский интерфейс**: Svelte 5, TypeScript, Vite, Tailwind CSS
- **Тестирование**: `cargo test`, `vitest`, фикстуры реальных логов Codex и Claude

---

## 🚀 Сборка и разработка

```bash
cd usage.ai

# Установка зависимостей интерфейса
npm install

# Запуск в режиме разработки Tauri
npm run tauri dev

# Сборка готового .app / .dmg бандла
npm run tauri build
```
