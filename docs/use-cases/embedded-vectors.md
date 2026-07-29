# No external vector database

> RAG pipelines usually need a separate vector database. AIPK embeds the vectors inside the package itself.

## The problem

A typical RAG setup means standing up pgvector, Qdrant, or Chroma, keeping it running, keeping it in sync with the source documents, and keeping its embedding model consistent with whatever generates queries at serve time. None of that travels with the "agent" you built — it's infrastructure that has to be reprovisioned wherever the agent goes.

## How AIPK solves it

The `KNOW` section of a `.aipk` file stores both halves of RAG together: a gzip-compressed JSONL of source chunks *and* their raw float32 embedding vectors, side by side in the same binary. There is no external store to provision, and no network hop between "generate an answer" and "look up relevant context" — retrieval reads straight out of the file that's already loaded.

```
KNOW section layout:
  24-byte header
  + gzip(JSONL chunks)
  + raw float32 vectors
```

The embedding model used is recorded in `META`, so the runtime can detect a mismatch if you ever swap embedding models rather than silently returning nonsense similarity scores.

## Walkthrough

```bash
# embeddings are computed once, at build time, and stored in the package
aipk add-docs contracts.pdf regulations.md \
  --embed-model nomic-embed-text \
  --llm-url http://localhost:11434

aipk build -o legal-assistant.aipk
```

From here, `legal-assistant.aipk` carries its own retrieval index. Serving it against a *different* generation model still works, because generation and embeddings are decoupled — only the embedding model has to stay consistent with the one recorded at build time:

```bash
# generation via vLLM, embeddings pulled from the package (already computed)
aipk serve legal-assistant.aipk \
  --llm-url http://localhost:8000 --model Qwen/Qwen2.5-7B-Instruct
```

Multiple packages can even be served together, with results globally re-ranked across all of their embedded knowledge:

```bash
aipk serve legal.aipk gdpr.aipk contracts.aipk --model llama3.2 --port 8080
```

No pgvector instance, no Qdrant container, no sync job — the vectors shipped with the file that needed them.

## See also

- [README.md — Any backend, not just ollama](../../README.md#any-backend-not-just-ollama)
- [SPEC.md — Package Format](../../SPEC.md)
- [single-file-packaging.md](single-file-packaging.md)
