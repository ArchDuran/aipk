# Ship one file, not a folder

> Sharing a "smart agent" normally means sharing a complex setup. AIPK bundles persona, RAG knowledge, skills, and MCP tools into one `.aipk` file.

## The problem

A working RAG agent is usually not one thing — it's a folder: a system prompt file, a dump from whatever vector database you picked, an embedding-model config that has to match exactly, and a handful of tool scripts wired together by hand. Handing that off to someone else means handing off your entire environment and hoping they can reproduce it. Versioning it means versioning five different artifacts that drift out of sync with each other.

## How AIPK solves it

`.aipk` is a single binary file: a 96-byte header followed by typed sections, with an `INDX` directory at the end listing every section's tag, offset, and size. Everything a specialist agent needs lives in one place:

| Section | Contents |
|---|---|
| `PERS` | Plain text persona / system prompt |
| `KNOW` | RAG chunks + their embedded vectors |
| `SKIL` | Skill manifest + markdown skill files |
| `TOOL` | MCP server configuration |
| `CLMS` / `CLMV` | Atomic claims + their vectors (see [provenance-verify.md](provenance-verify.md)) |

Building the file is one command; running it is another. Nothing else needs to travel with it.

## Walkthrough

```bash
# 1. scaffold a project
aipk init legal-assistant --dir ./legal-assistant
cd legal-assistant

# 2. add knowledge (embeds with ollama at build time)
aipk add-docs contracts.pdf regulations.md \
  --embed-model nomic-embed-text \
  --llm-url http://localhost:11434

# 3. add a skill, triggered when the user mentions "review"
aipk add-skill skills/contract-review.md

# 4. build the package
aipk build -o legal-assistant.aipk

# 5. inspect what's inside
aipk info legal-assistant.aipk
```

The result, `legal-assistant.aipk`, is the entire deliverable. Whoever receives it runs exactly one command to use it:

```bash
aipk serve legal-assistant.aipk --model llama3.2 --llm-url http://localhost:11434 --port 8080
```

That single `serve` invocation *is* the setup step on the receiving end — no vector database to provision, no embedding model to match by hand, no separate tool scripts to wire in.

## See also

- [README.md — Quick Start](../../README.md#quick-start)
- [SPEC.md — Package Format](../../SPEC.md)
- [embedded-vectors.md](embedded-vectors.md)
