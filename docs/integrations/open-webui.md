# AIPK + Open WebUI

Два способа подключить AIPK к Open WebUI — прямое подключение (проще) и pipeline-плагин (расширенные возможности).

## Способ 1: Прямое подключение (рекомендуется)

AIPK сервер полностью OpenAI-совместим — Open WebUI подключается без настройки.

**1. Запусти AIPK сервер**
```bash
aipk serve expert.aipk --model llama3.2 --port 8080
```

**2. В Open WebUI**

`Settings` → `Connections` → `OpenAI API` → добавь:
```
URL:     http://localhost:8080/v1
API Key: none
```

После сохранения модель появится в списке как `llama3.2` и `expert` (имя пакета).

---

## Способ 2: Pipeline-плагин

Используй когда нужно видеть какие чанки были найдены, фильтровать контент или добавить свою логику.

Файл плагина: [`integrations/open-webui-pipeline.py`](../../integrations/open-webui-pipeline.py)

**Установка:**

1. В Open WebUI: `Settings` → `Pipelines` → загрузи файл плагина
2. Настрой `AIPK_SERVER_URL` в Valves (по умолчанию `http://localhost:8080`)
3. Выбери pipeline как модель в чате

---

## Способ 3: Docker Compose (рекомендуется для продакшена)

```yaml
# docker-compose.yml
services:
  ollama:
    image: ollama/ollama
    volumes:
      - ollama_data:/root/.ollama
    ports:
      - "11434:11434"

  aipk:
    image: python:3.11-slim
    command: >
      sh -c "pip install aipk[serve] && aipk serve /packages/expert.aipk
             --model llama3.2 --ollama http://ollama:11434 --port 8080"
    volumes:
      - ./packages:/packages
    ports:
      - "8080:8080"
    depends_on:
      - ollama

  open-webui:
    image: ghcr.io/open-webui/open-webui:main
    environment:
      - OPENAI_API_BASE_URL=http://aipk:8080/v1
      - OPENAI_API_KEY=none
    ports:
      - "3000:8080"
    depends_on:
      - aipk

volumes:
  ollama_data:
```

```bash
docker compose up
# → Open WebUI на http://localhost:3000
# → уже подключён к AIPK + ollama
```
