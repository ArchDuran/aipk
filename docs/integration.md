# Интеграция с существующим ПО

## Стратегия A: Middleware-прокси (рекомендуется для старта)

Самый быстрый путь — прокси который сидит между клиентом и ollama.

```
Клиент (Open WebUI, curl, приложение)
         │
         ▼
  aipk-proxy :11435
         │
         ├── получает chat completion запрос
         ├── embed(user_query) → ищет top_k чанков в KNOW
         ├── инжектирует чанки в messages[]
         ├── prepend system prompt из PERS
         ├── применяет SKIL если совпадает trigger
         │
         ▼
  ollama :11434  (не тронут)
```

**Плюсы:** ollama не патчится, работает с любым OpenAI-совместимым клиентом  
**Минусы:** лишний сетевой хоп, прокси держит .aipk в памяти

## Стратегия B: Open WebUI Pipeline

Open WebUI поддерживает Pipelines — Python-скрипты перехватывающие запросы.

```python
class AIPKPipeline:
    def on_startup(self):
        self.pkg = AIPKPackage.load(os.getenv("AIPK_PATH"))

    def pipe(self, messages, model_id, body):
        query = messages[-1]["content"]
        chunks = self.pkg.knowledge.search(query, top_k=5)
        return inject_context(messages, chunks, self.pkg.persona)
```

**Плюсы:** UI уже есть, не нужен фронтенд  
**Минусы:** привязка к Open WebUI

## Стратегия C: llama.cpp напрямую

llama.cpp умеет embedding (`llama_encode`) и имеет серверный режим.

```python
from llama_cpp import Llama
from aipk import AIPKPackage

llm = Llama(model_path="model.gguf", embedding=True)
pkg = AIPKPackage.load("expert.aipk")

query_vec = llm.embed(user_query)
chunks = pkg.knowledge.search(query_vec, top_k=5)

response = llm.create_chat_completion(
    messages=[
        {"role": "system", "content": pkg.persona + "\n\n" + format_chunks(chunks)},
        {"role": "user",   "content": user_query}
    ]
)
```

**Плюсы:** минимум зависимостей  
**Минусы:** нет UI, нужно писать сервер

## Стратегия D: LlamaIndex/LangChain для сборки

Эти фреймворки используются **только на этапе build**, не в рантайме:

```python
from llama_index.core import SimpleDirectoryReader
from aipk import AIPKBuilder

docs = SimpleDirectoryReader("./docs").load_data()
builder = AIPKBuilder()
builder.set_persona("Ты — эксперт по Rust...")
builder.add_documents(docs, embed_model="nomic-embed-text")
builder.build("rust-expert.aipk")
```

## Совместимость форматов

| Формат / Инструмент    | Отношение к .aipk |
|------------------------|-------------------|
| `.gguf`                | Дополняет — .aipk нужна базовая модель |
| Ollama Modelfile       | .aipk → Modelfile (экспорт PERS + META) |
| AnythingLLM workspace  | Схожая концепция, возможен импорт/экспорт |
| OpenAI Assistants API  | KNOW ≈ Vector Store, TOOL ≈ tools |
| LangChain agents       | SKIL ≈ tool descriptions |
| LM Studio              | Через OpenAI-совместимый API + прокси |
