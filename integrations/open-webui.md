# AIPK + Open WebUI

Serve any `.aipk` package as a model inside Open WebUI. The package appears in the
model dropdown under its own name; retrieval, claims and strict modes run inside
`aipk serve`, so Open WebUI needs zero extra configuration beyond one connection.

```
Open WebUI ──/v1──> aipk serve (RAG + claims + policy) ──> ollama / vLLM / any OpenAI backend
```

## 1. Direct connection (recommended)

Start the package server:

```bash
aipk serve mypkg.aipk --model llama3.2:3b --port 8080
# add --strict-render to force claim-cited answers
```

> Note: with ollama, use the full model tag (`llama3.2:3b`). A bare name only
> resolves if the model is tagged `:latest`.

Point Open WebUI at it — either in the UI
(**Admin → Settings → Connections → OpenAI API**, URL `http://<host>:8080/v1`,
any non-empty API key), or via environment:

```bash
docker run -d --name open-webui --network=host \
  -e PORT=3001 \
  -e OPENAI_API_BASE_URL=http://127.0.0.1:8080/v1 \
  -e OPENAI_API_KEY=aipk \
  -e ENABLE_OLLAMA_API=False \
  -v open-webui:/app/backend/data \
  ghcr.io/open-webui/open-webui:main
```

Open `http://localhost:3001` — every loaded package is listed as a model
(`aipk serve` implements `/v1/models`). Streaming works out of the box.

Multi-package: pass several packages to `aipk serve` (or use `--graph` for
embedding-based routing) — each one shows up as a separate model in the dropdown.

## 2. Pipeline (optional)

`integrations/open-webui-pipeline.py` is a [Pipelines](https://github.com/open-webui/pipelines)
wrapper that additionally appends retrieved source previews to answers
(`SHOW_SOURCES`, backed by `POST /v1/retrieve`). Use it only if you already run a
Pipelines server; for most setups the direct connection above is simpler.

## 3. Useful endpoints

| Endpoint | Purpose |
|---|---|
| `GET /v1/models` | packages as models (feeds the dropdown) |
| `POST /v1/chat/completions` | OpenAI-compatible chat, `stream: true` supported |
| `POST /v1/retrieve` `{query, top_k}` | raw RAG chunks with cosine scores — debugging, source display |
| `GET /health` | package stats: chunks, claims, strict flags, MCP tools |

## 4. Verified setup

Tested end-to-end with Open WebUI (ghcr.io `main`, July 2026), `aipk serve` and
ollama `llama3.2:3b`: model listing, streaming and non-streaming chat, strict-render
refusals on out-of-corpus questions.
