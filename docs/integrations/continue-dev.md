# AIPK + Continue.dev

Continue.dev — open-source AI-ассистент для VS Code и JetBrains.
Подключается к AIPK серверу через OpenAI-совместимый API.

## Быстрый старт

**1. Запусти AIPK сервер**

```bash
aipk serve my-expert.aipk --model llama3.2 --port 8080
```

**2. Добавь модель в Continue**

Создай или отредактируй `.continue/config.yaml` в корне проекта:

```yaml
models:
  - name: "My Expert (AIPK)"
    provider: openai
    baseUrl: http://localhost:8080/v1
    model: llama3.2
    apiKey: "none"

  # Оставь другие модели если нужны
  - name: "Claude 3.5 Sonnet"
    provider: anthropic
    model: claude-sonnet-4-6
```

**3. Выбери модель в Continue**

В VS Code: `Ctrl+Shift+P` → `Continue: Select Model` → выбери `My Expert (AIPK)`

---

## Конфигурация под задачи

### Автодополнение кода (Tab)

```yaml
tabAutocompleteModel:
  name: "AIPK Autocomplete"
  provider: openai
  baseUrl: http://localhost:8080/v1
  model: llama3.2
  apiKey: "none"
```

### Несколько пакетов под разные контексты

```yaml
models:
  - name: "Rust Expert"
    provider: openai
    baseUrl: http://localhost:8081/v1   # отдельный aipk serve на другом порту
    model: llama3.2
    apiKey: "none"

  - name: "Python Expert"
    provider: openai
    baseUrl: http://localhost:8082/v1
    model: llama3.2
    apiKey: "none"
```

### Запуск нескольких серверов

```bash
aipk serve rust-expert.aipk   --model llama3.2 --port 8081 &
aipk serve python-expert.aipk --model llama3.2 --port 8082 &
```

---

## Полный шаблон `.continue/config.yaml`

```yaml
models:
  - name: "Project Expert (AIPK)"
    provider: openai
    baseUrl: http://localhost:8080/v1
    model: llama3.2
    apiKey: "none"
    description: "Domain-specific assistant powered by AIPK"

tabAutocompleteModel:
  name: "AIPK Autocomplete"
  provider: openai
  baseUrl: http://localhost:8080/v1
  model: llama3.2
  apiKey: "none"

slashCommands:
  - name: "review"
    description: "Review current file"

contextProviders:
  - name: code
  - name: docs
  - name: diff
  - name: terminal
```

---

## Проверка подключения

```bash
curl http://localhost:8080/v1/models
# → {"data": [{"id": "llama3.2"}, {"id": "my-expert"}]}

curl http://localhost:8080/health
# → {"status": "ok", "stats": {"chunks": 42, "rag_active": true}}
```
