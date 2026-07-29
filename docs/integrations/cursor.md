# AIPK + Cursor / VS Code

Cursor и VS Code поддерживают кастомные OpenAI-совместимые провайдеры.

## Cursor

`Settings` → `Models` → `+ Add Model`:
```
Model Name:  llama3.2
Base URL:    http://localhost:8080/v1
API Key:     none
```

Или через `.cursor/settings.json`:
```json
{
  "aiProvider": "openai",
  "openAI": {
    "apiKey": "none",
    "baseURL": "http://localhost:8080/v1",
    "model": "llama3.2"
  }
}
```

## VS Code + GitHub Copilot Chat (custom backend)

Пока не поддерживает кастомный endpoint напрямую — используй Continue.dev вместо него.

## LM Studio

`Settings` → `Local Server` → включи, укажи порт 1234.

Затем настрой AIPK на форвард к LM Studio:
```bash
aipk serve expert.aipk --model llama3.2 \
  --ollama http://localhost:1234 \
  --port 8080
```

## Проверка любого клиента

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama3.2",
    "messages": [{"role": "user", "content": "Привет!"}]
  }'
```
