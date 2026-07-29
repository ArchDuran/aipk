# AIPK — Trust Layer for Portable AI Specialists

[![CI](https://github.com/ArchDuran/aipk/actions/workflows/ci.yml/badge.svg)](https://github.com/ArchDuran/aipk/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ArchDuran/aipk)](https://github.com/ArchDuran/aipk/releases)
[![License: BSL 1.1](https://img.shields.io/badge/License-BSL%201.1-blue.svg)](LICENSE)
[![Security Policy](https://img.shields.io/badge/security-policy-green)](SECURITY.md)

**[aipk.dev](https://aipk.dev)** — site, docs, demo video

> Internal preview — no public binary releases yet.

**Package trustworthy AI expertise into one portable file.**

A `.aipk` package carries a domain expert's knowledge, policies, skills, and tools as one versioned, signed artifact — decoupled from any specific model. Point it at Ollama, vLLM, llama.cpp, LM Studio, or a cloud OpenAI-compatible endpoint, and every factual sentence it produces can be traced back to a reviewed, canonical claim and the exact source span that backs it.

**Portable** — the expert package outlives any one model; swap Llama for Qwen or a cloud API without losing the knowledge or policies.
**Governed** — every claim has an explicit lifecycle (extracted → reviewed → canonical → deprecated) and an audit trail of who approved it.
**Verifiable** — sentence-level provenance ties every factual claim in an answer back to its source document and exact span.

```
Documents + Policies + Skills + Tools
              ↓
      legal-expert.aipk
              ↓
   Ollama / vLLM / Cloud LLM
              ↓
Answer → Claim → Source → Exact span
```

A 4 MB Rust binary. No vector database. No setup beyond ollama.

**Who it's for** — teams building internal AI assistants on sensitive corporate documents who need local execution, version-controlled knowledge, and an audit trail on every answer. Best fit today: legal, compliance, insurance, fintech, and enterprise operations teams.

---

## Why AIPK?

| Problem | Solution | Details |
|---|---|---|
| Sharing a "smart agent" means sharing a complex setup | One `.aipk` file contains everything | [docs](docs/use-cases/single-file-packaging.md) |
| RAG pipelines need separate vector databases | Vectors are embedded inside the package | [docs](docs/use-cases/embedded-vectors.md) |
| No standard way to audit what an LLM actually said | Sentence-level provenance via `aipk verify` | [docs](docs/use-cases/provenance-verify.md) |
| LLMs hallucinate even with good context | Strict-render mode: model is prompted to cite a verified claim per sentence; every response reports which sentences weren't grounded, and `--enforce block` can withhold ungrounded answers outright (experimental) | [docs](docs/use-cases/strict-render.md) |
| Claims live in a black box | Explicit claim lifecycle: extracted → reviewed → canonical → deprecated | [docs](docs/use-cases/claim-lifecycle.md) |

Full write-ups with worked examples: [docs/use-cases/](docs/use-cases/).

---

## The core loop

Three steps: build a package from documents, review what it knows, serve it and get a verifiable answer. Everything past this point ([Beyond the MVP](#beyond-the-mvp)) is real and shipped, but secondary to getting this loop working first.

## Quick Start

Requires [ollama](https://ollama.ai).

```bash
# Build from source (cargo 1.75+) — no pre-built binaries yet
git clone <repository-url> aipk && cd aipk
cargo build --release   # binary at: target/release/aipk
```

### Create a package

```bash
# 1. Init project
aipk init legal-assistant --dir ./legal-assistant
cd legal-assistant

# 2. Add documents (embeds with ollama)
aipk add-docs contracts.pdf regulations.md \
  --embed-model nomic-embed-text \
  --llm-url http://localhost:11434

# 3. Add a skill (triggered when user mentions "review")
aipk add-skill skills/contract-review.md

# 4. Build
aipk build -o legal-assistant.aipk

# 5. Inspect
aipk info legal-assistant.aipk
```

### Serve

```bash
aipk serve legal-assistant.aipk \
  --model llama3.2 \
  --llm-url http://localhost:11434 \
  --port 8080

# Force response language (overrides model's language choice)
aipk serve legal-assistant.aipk --model llama3.2 --lang ru
```

Point any OpenAI-compatible client at `http://localhost:8080/v1/` — Continue.dev, Open WebUI, Cursor, or curl. Streaming (`stream: true`) is fully supported. A built-in chat UI is served at `http://localhost:8080/`.

### Turnkey start

```bash
aipk up                      # detect ollama/vLLM/LM Studio/llama.cpp, serve hub packages + chat UI
aipk up legal.aipk gdpr.aipk # or explicit packages
```

### Any backend, not just ollama

Generation goes through the standard `/v1/chat/completions`, embeddings speak
both the ollama API (`/api/embed`) and the OpenAI API (`/v1/embeddings`) with
automatic detection — so vLLM, SGLang, llama.cpp and LM Studio work out of the box:

```bash
vllm serve Qwen/Qwen2.5-7B-Instruct --port 8000
aipk serve pkg.aipk --llm-url http://localhost:8000 --model Qwen/Qwen2.5-7B-Instruct \
  --embed-url http://localhost:11434   # embeddings from ollama, generation from vLLM
```

No local GPU? Every command that talks to an LLM — `add-docs`, `add-git`,
`add-web`, `extract-claims`, `pipeline`, `claims embed`,
`claims check-conflicts`, `serve`, `run` — accepts `--llm-url` and
`--llm-api-key`, so the same package-building workflow above works
unchanged against a hosted OpenAI-compatible endpoint:

```bash
aipk pipeline contracts.pdf regulations.md --dir ./legal-assistant \
  --llm-url https://api.openai.com \
  --llm-api-key "$OPENAI_API_KEY" \
  --model gpt-4o-mini --embed-model text-embedding-3-small \
  --auto-promote -o legal-assistant.aipk
```

Any provider that speaks the OpenAI API works the same way — OpenAI, Mistral,
xAI/Grok, DeepSeek, Together, or a company's own internal LLM gateway connect
directly, no adapter needed.

Anthropic Claude and Google Gemini don't expose an OpenAI-compatible endpoint
natively — route through [OpenRouter](https://openrouter.ai) instead, and the
exact same commands work unchanged, one API key for all of them:

```bash
aipk pipeline contracts.pdf regulations.md --dir ./legal-assistant \
  --llm-url https://openrouter.ai/api/v1 \
  --llm-api-key "$OPENROUTER_API_KEY" \
  --model anthropic/claude-3.5-sonnet --embed-model text-embedding-3-small \
  --auto-promote -o legal-assistant.aipk
```

Swap `--model` for `google/gemini-2.0-flash`, `openai/gpt-4o`,
`mistralai/mistral-large`, `x-ai/grok-2`, or any other model OpenRouter
carries — nothing else in the command changes. `aipk serve`/`aipk run` take
the same two flags, so a package built against one provider can be served
against a completely different one without rebuilding it.

---

## Epistemic Mode — Grounded, Auditable Answers

AIPK goes beyond RAG. The `CLMS` section stores atomic facts extracted from your documents, each with a source reference and a lifecycle status (`extracted → reviewed → canonical → deprecated`). This enables two strict operating modes and post-hoc verification — the second and third steps of the core loop.

### Extract claims

```bash
aipk extract-claims regulations.pdf contracts.pdf \
  --model llama3.2 \
  --dir ./legal-assistant
```

Claims are written to `claims.jsonl` with status `extracted`. Review and promote them with the `aipk claims` subcommands (see CLI Reference).

For sources under a license that doesn't permit reproducing excerpts
(proprietary or CC-BY-NC/ND material), `--digest` rewrites each claim as a
new sentence in the model's own words instead of quoting the source
verbatim — a package built this way is closer to cited study notes than to
a redistributed copy. Pair it with `--source-map` to attach a real citation
instead of a bare filename:

```bash
aipk extract-claims handbook.pdf --dir ./ops-assistant --digest \
  --source-map sources.json   # {"handbook.pdf": "Ops Handbook, 3rd ed."}
```

### Verify an answer

```bash
aipk verify legal-assistant.aipk \
  "Data retention must not exceed 5 years under Article 17." \
  --json
```

```json
{
  "package": "legal-assistant",
  "sentences": [
    {
      "text": "Data retention must not exceed 5 years under Article 17.",
      "verdict": "grounded",
      "claim_id": "regs_0012",
      "claim_text": "Article 17 limits data retention to 5 years.",
      "source": "regulations.pdf",
      "span": "retention period shall not exceed five years",
      "score": 0.71
    }
  ],
  "summary": {
    "total": 1,
    "grounded": 1,
    "coverage": 1.0,
    "unsupported_sentences": []
  }
}
```

**Exit codes** (CI/pipeline ready):
- `0` — all sentences grounded
- `1` — unsupported sentences found
- `2` — package or runtime error

### Strict serving modes

```bash
# strict-verify: model is instructed to answer only from canonical claims and say
# "I don't have verified information" if none match — a soft prompt constraint,
# not an enforced refusal (falls back to general knowledge if the package has no claims at all).
# --enforce does not change strict-verify's own behavior — see below.
aipk serve legal-assistant.aipk --strict-verify --model llama3.2

# strict-render: LLM is prompted to cite [claim_id] after every factual sentence;
# by default (--enforce observe) the response is reported on, not blocked — see below
aipk serve legal-assistant.aipk --strict-render --model llama3.2
```

**strict-render** always prompts the model to cite claims and reports grounding as `_aipk` metadata on every response. What happens to an *ungrounded* response depends on `--enforce`:

- `--enforce observe` (default) — report only, same as before this flag existed. The caller decides what to do with `fully_grounded: false`.
- `--enforce warn` — return the answer, but set `_aipk.flagged: true` so it's easy to surface a banner without inspecting `coverage` yourself.
- `--enforce block` — withhold the ungrounded answer entirely; `content` becomes a refusal message instead. `aipk run --strict-render --enforce block` also exits non-zero, so scripts/CI can detect a block.

```json
{
  "_aipk": {
    "mode": "strict-render",
    "coverage": 0.87,
    "canonical_claims_used": 4,
    "uncited_sentences": 1,
    "invalid_claim_ids": [],
    "unsupported_sentences": ["This sentence had no citation."],
    "fully_grounded": false,
    "verdict": "warn",
    "enforce": "warn"
  }
}
```

> **EXPERIMENTAL, read narrowly:** `--enforce block` gates on the same single `fully_grounded` signal shown above — citation validity + sentence coverage. It does not check claim freshness, semantic entailment (does the cited claim actually support the sentence?), or contradictions between canonical claims. Treat it as "ungrounded sentences withheld," not "hallucinations eliminated."

---

## MCP Tools

```bash
# Add a filesystem MCP server to the package
aipk add-tool filesystem \
  --command npx \
  --args "-y @modelcontextprotocol/server-filesystem /workspace" \
  --dir ./legal-assistant
```

Tools are launched automatically on `aipk serve` and exposed to the LLM via OpenAI tool-calling protocol.

> **Security note:** `aipk serve`/`run`/`up`/`mcp`/`hub serve` print every MCP command a package declares, each server's signature status, and ask for confirmation before launching any of it. `--yes` trusts the whole package with no prompt; `--allow <server>` (repeatable) pre-approves specific servers by name without a blanket `--yes` — once every declared server is covered by `--allow`, the prompt is skipped, otherwise the remaining servers still get one combined y/N. `--allow-tool <server>:<tool>` (repeatable) additionally restricts which *tools* on an approved server can actually be called, rejecting everything else. Running non-interactively without `--yes`/full `--allow` coverage refuses to launch.
>
> The signature line shown at the prompt (`integrity OK, unknown publisher (fingerprint SHA256:...)` or `UNSIGNED`) only answers "has this package's bytes changed since some key signed it" — it does **not** mean the package is safe or that you should trust its publisher. AIPK's signing (`aipk sign`/`aipk keygen`) is TOFU (trust-on-first-use): there's no certificate authority, no publisher-identity verification, and no revocation. Read "signed" as *integrity*, not *trust*.
>
> None of the above is a sandbox. There's still no filesystem/network isolation, no per-tool resource limits, and no timeout enforcement — a command or tool call you approve runs with your full user permissions. Treat an untrusted `.aipk` package like you'd treat an untrusted shell script, not like an untrusted document. Real process isolation (seccomp/containers) is on the roadmap, not implemented today.

---

## Beyond the MVP

Everything below is real, tested, and shipped — but secondary to the core loop above (build → review claims → serve a verified answer). Reach for it once that loop works for your use case.

### Multi-package serving

```bash
# Merge knowledge from multiple packages; RAG results are globally re-ranked
aipk serve legal.aipk gdpr.aipk contracts.aipk \
  --model llama3.2 --port 8080

# Graph mode: each query is routed to the best-matching package
# (requires thkg.md in each package describing its domain)
aipk serve legal.aipk medical.aipk finance.aipk \
  --model llama3.2 --graph --port 8080
```

**Package-as-model dispatch**: with multiple packages loaded, `/v1/models`
lists every package as a "model". A client that sets `"model": "legal"`
talks to that package only (its persona, knowledge, and policies); any other
model name is forwarded to the backend unchanged.

### Datasets — reusable knowledge sources

A dataset is a directory of raw materials, independent from any package:
one dataset can feed many packages. Files dropped into the directory are
picked up automatically (Markdown, PDF, DOCX, CSV/TSV, HTML, JSON, …);
`sources.toml` links external sources.

```bash
aipk dataset init second_me_developer     # creates ~/Datasets/second_me_developer/
# drop files into the directory, and/or declare links:
```

```toml
# ~/Datasets/second_me_developer/sources.toml
[dataset]
name = "second_me_developer"

[[link]]                       # local dir (e.g. an Obsidian vault) — read live
type = "dir"
path = "~/Documents/Obsidian/SecondMe"
ext = ["md"]

[[link]]                       # web pages / sitemap — cached by `dataset sync`
type = "web"
url = "https://docs.example.com/sitemap.xml"
sitemap = true

[[link]]                       # git repo — cached by `dataset sync`
type = "git"
url = "https://github.com/org/second-me"
```

```bash
aipk dataset sync second_me_developer          # mirror web/git links into .cache/
aipk add-dataset second_me_developer --dir ./my-project   # chunk + embed into the project
aipk build --dir ./my-project -o second_me.aipk
```

Datasets store raw text only — embeddings belong to the package and are
computed at ingest time, so the same dataset works with any embedding model.
Secrets never go into `sources.toml`; reference environment variables instead.

### Sealed packages — weights-grade content protection

```bash
aipk keygen --name author
aipk seal pkg.aipk --key ~/.aipk/keys/author.pem
```

A sealed package behaves like model weights: **`serve`/`run`/`mcp` work
normally, but the file is opaque** — contents are AES-256-GCM encrypted, and
`aipk extract` / `aipk export` are blocked. Integrity is real cryptography:
the package is Ed25519-signed and the runtime **refuses to load a sealed
package that fails verification**, so modifying it requires the author's
private key (`aipk unseal`). License and copyright terms live in the `LICN`
section (`license.toml`), which stays readable even when sealed — recipients
always see the terms, `aipk info` shows them.

Honest scope note: sealing protects against casual reading and against *any*
undetected modification. It is not DRM — a determined attacker with the
open-source runtime can recover the plaintext.

### GDPR & data subject rights

If a package is built from people's own contributions — onboarding notes,
wiki pages, Slack exports — it needs to answer both sides of a data subject
request: "give me a copy of what you have on me" (Art. 15/20) and "delete
everything about me" (Art. 17). Both operate on the same underlying unit —
a `source` — and both can act on a single file or on every file belonging
to one person at once, via a `--subject-map` (`{"filename": "subject id"}`):

```bash
aipk gdpr list-sources acme-kb.aipk

# Right of access — export everything tied to one person, without changing the package
aipk gdpr snapshot acme-kb.aipk \
  --subject alice@acme.com --subject-map subjects.json \
  -o alice-snapshot.json

# Right to erasure — remove every source belonging to that person in one call
aipk gdpr erase acme-kb.aipk \
  --subject alice@acme.com --subject-map subjects.json

# Single-file shortcut for either command, no map needed
aipk gdpr erase acme-kb.aipk --source alice-onboarding.md --dry-run
```

`snapshot` writes a self-contained JSON export (source metadata + every
matching KNOW chunk + every matching CLMS claim) — a portable answer to a
subject access request. `erase` rebuilds the package with those sections
removed. `aipk gdpr report` summarizes what personal data a package holds
and flags packages that should be encrypted before distribution.

### Export to agent harnesses

`.aipk` sections map 1:1 onto harness concepts, so a package can be unpacked
into the native layout of your agent tool:

```bash
aipk export pkg.aipk --to claude-code   # CLAUDE.md + .claude/skills/ + .mcp.json + knowledge/
aipk export pkg.aipk --to openclaw      # SOUL.md + skills/ + mcp-servers.json + knowledge/
aipk export pkg.aipk --to generic       # persona.md + skills/ + tools.json + knowledge/
```

Claims and strict verification have no harness equivalent — keep serving via
`aipk serve` (or `aipk mcp`) when you need grounded, auditable answers.

---

## CLI Reference

| Command | Description |
|---|---|
| `aipk doctor` | Check the environment: inference backend reachable, models available, current directory writable |
| `aipk info <file>` | Inspect package sections and metadata |
| `aipk init <name>` | Create a new project directory |
| `aipk init-base` | Create a ready-to-use behavioral base package (small-model friendly) |
| `aipk add-docs <files>` | Chunk documents and embed (Markdown/PDF/DOCX/CSV/HTML/…) |
| `aipk dataset init/list/sync` | Manage reusable dataset directories (`~/Datasets`, `$AIPK_DATASETS`) |
| `aipk add-dataset <name>` | Ingest a dataset (own files + dir/web/git links) into a project |
| `aipk add-git <url\|path>` | Import a git repo as knowledge base (shallow clone, filters by extension) |
| `aipk add-web <url>` | Import web pages or a sitemap as knowledge base (HTML stripped, `--sitemap`) |
| `aipk add-skill <file>` | Add a markdown skill with frontmatter trigger |
| `aipk add-tool <name>` | Add an MCP stdio server |
| `aipk extract-claims <files>` | Extract atomic claims with LLM |
| `aipk claims list` | List claims filtered by status |
| `aipk claims stats` | Claim count distribution per status |
| `aipk claims promote <id>` | Promote claim to canonical with audit trail |
| `aipk claims reject <id>` | Mark claim deprecated with audit trail |
| `aipk claims review` | Interactive y/n/s/q review of pending claims |
| `aipk claims promote-all` | Bulk-promote all claims of a given status |
| `aipk claims check-conflicts` | NLI-based conflict detection between canonical claims via LLM |
| `aipk pipeline <files>` | Full pipeline: embed + extract + review + build in one command |
| `aipk lint [path]` | Static analysis: injection patterns, homoglyphs, language directive check |
| `aipk build` | Assemble `.aipk` from project directory |
| `aipk extract <file>` | Unpack `.aipk` back into a project directory |
| `aipk run <pkg> <query>` | One-shot query (`--strict-verify` / `--strict-render`, `--embed-url`, `--enforce observe\|warn\|block`, `--allow <server>`, `--allow-tool <server:tool>`) |
| `aipk serve <pkgs...>` | OpenAI-compatible server (multi-package, streaming, chat UI at `/`, package-as-model dispatch, `--embed-url`, `--enforce observe\|warn\|block`, `--allow <server>`, `--allow-tool <server:tool>`) |
| `aipk up [pkgs...]` | Turnkey start: autodetect backend, serve hub packages + chat UI |
| `aipk export <pkg> --to <t>` | Unpack into a harness layout: claude-code / openclaw / generic |
| `aipk seal <pkg> --key <pem>` | Seal: encrypt contents + Ed25519 sign (opaque, author-locked) |
| `aipk unseal <pkg> --key <pem>` | Unseal back to an editable package (author's key only) |
| `aipk verify <pkg> <answer>` | Sentence-level provenance check (`--min-coverage` for CI) |
| `aipk bench-scale --sizes <n,...>` | Synthetic-vector benchmark: KNOW section build/load time, size, retrieval latency at scale |
| `aipk mcp <pkg>` | Expose package as MCP stdio server (`--allow <server>`, `--allow-tool <server:tool>`) |
| `aipk hub install <path\|url>` | Install a package into the local hub |
| `aipk hub list` | List installed hub packages |
| `aipk hub search <query>` | Search hub by name, description, or tag |
| `aipk hub publish <pkg>` | Publish a local package to the hub |
| `aipk hub serve [names...]` | Serve all (or selected) hub packages via OpenAI-compatible API |
| `aipk hub info <name>` | Show hub package details |
| `aipk hub remove <name>` | Remove a package from the hub |
| `aipk hub update [name]` | Re-download URL-sourced packages |
| `aipk hub status` | Show hub directory and disk usage |
| `aipk v2-init [dir]` | Create Epistemic v2 template files (IDTY/ANSP/PLCY/NKNW) |
| `aipk keygen [--name <n>]` | Generate Ed25519 keypair → `~/.aipk/keys/` |
| `aipk sign <pkg> --key <key.pem>` | Sign package (appends SIGN section, sets SIGNED flag) |
| `aipk verify-sig <pkg> [--key <pub>]` | Verify Ed25519 signature (exit 0=valid, 1=invalid) |
| `aipk test <pkg> [--model <m>]` | Run static checks + live LLM tests from TEST section |
| `aipk encrypt <pkg> --key <passphrase>` | Encrypt content sections with AES-256-GCM |
| `aipk decrypt <pkg> --key <passphrase>` | Decrypt a previously encrypted package |
| `aipk gdpr list-sources <pkg>` | List all source documents embedded in the package |
| `aipk gdpr snapshot <pkg> --source\|--subject` | Export everything held about a source/person (KNOW + CLMS + SRCS), unmodified |
| `aipk gdpr erase <pkg> --source\|--subject` | Remove all data from a source, or every source of one person (KNOW + CLMS + SRCS) |
| `aipk gdpr report <pkg>` | Generate a GDPR compliance report |

---

## Package Format

`.aipk` is a binary format: 96-byte header followed by typed sections, with an `INDX` section at the end listing every section's tag, offset, and size.

The `INDX` directory exists for external tools to inspect the file layout without a full parse — `aipk` itself still reads section headers sequentially rather than jumping via `INDX`, which costs little given how few sections a package has, and sidesteps a real gap: `aipk sign` appends the `SIGN` section after `INDX` is built without updating the directory, so `INDX` isn't guaranteed complete for signed packages. None of this is about semantic search speed, which is a separate question. Retrieval is a brute-force cosine scan by default: O(n) per query, comfortable through ~100k chunks (~123ms p50). For packages built with more than 20,000 KNOW chunks, `aipk build`/`pipeline` also build an HNSW index (via `instant-distance`) and embed it as an `ANNX` section — retrieval then drops to low-single-digit milliseconds regardless of package size (~1.3ms p50 at 100k chunks, ~95x faster than brute force there), at the cost of roughly doubling package size and taking noticeably longer to build (the index build itself is O(n log n)-ish and only happens once, at build time — never on `aipk run`/`serve`/`test` load, since those can reload a package once per query). Packages below the threshold, or built before ANNX existed, fall back to brute force. See [bench/scale-results.md](bench/scale-results.md), reproducible with `aipk bench-scale`.

```
[MAGIC "AIPK" | version u32 | name u8[64] | created_at u64 | section_count u32 | flags u32 | indx_offset u64]
[Section: tag u8[4] | flags u32 | size u64 | data]  ×  N
[INDX: section directory]
```

| Section | Contents |
|---|---|
| `META` | TOML: name, version, author, description |
| `PERS` | Plain text persona / system prompt |
| `KNOW` | 24-byte header + gzip(JSONL chunks) + raw float32 vectors |
| `SKIL` | JSON manifest + markdown skill files |
| `TOOL` | JSON MCP server configuration |
| `CLMS` | JSONL: atomic claims with id, text, source, span, status |
| `CLMV` | float32 vectors for claims (parallel to CLMS; enables semantic matching) |
| `SRCS` | JSONL: source document registry |
| `IDTY` | JSON: identity contract (name, role, owner — injected into system prompt) |
| `ANSP` | JSON: answerability gate (domain keywords, confidence threshold, messages) |
| `PLCY` | JSON: answer policy (citation_required, forbidden behaviors) |
| `NKNW` | JSONL: negative knowledge (forbidden topics, acknowledged unknowns) |
| `THKG` | Plain text domain description for graph routing (`aipk serve --graph`) |
| `LICN` | TOML: license / copyright terms — always plaintext, readable even when sealed |
| `SEAL` | Seal salt (18 bytes) for author-locked packages |
| `TEST` | JSON: CI test suite (static checks + live LLM assertions) |
| `SIGN` | Ed25519 signature (98 bytes: 32B pubkey + 64B sig + 2B reserved) |
| `INDX` | Section directory |
| `PKDF` | Argon2id salt + params (30 bytes) for passphrase-encrypted packages; present only while encrypted, dropped on `aipk decrypt` |

Content sections (`PERS`, `KNOW`, `SKIL`, `CLMS`, `CLMV`, `SRCS`, `IDTY`, `ANSP`, `PLCY`, `NKNW`, `TOOL`, `THKG`, `TEST`) can be encrypted with AES-256-GCM using `aipk encrypt`, which derives the key from your passphrase with Argon2id (a fresh random salt per encryption, stored in `PKDF`). `META`, `INDX`, `SIGN`, and `PKDF` are always stored in plaintext.

Full specification: [SPEC.md](SPEC.md)

---

## Skill Format

Skills are markdown files with YAML frontmatter:

```markdown
---
name: Contract Review
trigger: review
---

Review the following contract for:
1. Missing termination clauses
2. Liability caps
3. GDPR compliance

Flag any issues with severity (HIGH / MEDIUM / LOW).
```

When the user message contains the trigger word, the skill is injected into the system prompt.

---

## Integrations

Works with any OpenAI-compatible client — no changes required:

| Tool | How |
|---|---|
| **Continue.dev** | `apiBase: "http://localhost:8080/v1"` in `config.yaml` |
| **Open WebUI** | Settings → Connections → add URL |
| **Cursor** | Settings → Models → OpenAI base URL |
| **curl / any HTTP client** | `POST /v1/chat/completions` |

---

## Requirements

- Any LLM backend with `/v1/chat/completions`: [ollama](https://ollama.ai), vLLM, SGLang, llama.cpp, LM Studio
- Embeddings: ollama `/api/embed` or any OpenAI-compatible `/v1/embeddings` (auto-detected; `--embed-url` to split from generation)
- Recommended models: `llama3.2` (generation), `nomic-embed-text` (embeddings)
- Rust 1.75+ to build from source

---

## Roadmap

- [x] `aipk claims list / promote / reject / review / promote-all` — explicit claim lifecycle CLI with audit trail
- [x] `aipk extract` — unpack `.aipk` contents to directory
- [x] Streaming responses in `aipk serve` (`stream: true`)
- [x] `aipk lint` — static persona analysis (injection patterns, homoglyphs)
- [x] `aipk init-base` — ready-to-use behavioral base package for small models
- [x] `--lang` flag in `aipk serve` — force response language
- [x] Semantic claim matching via `CLMV` (cosine similarity, cross-language)
- [x] `aipk hub` — local package registry (install, search, publish, serve, update)
- [x] Ed25519 signing — `aipk keygen` / `aipk sign` / `aipk verify-sig`, SIGN section, tamper detection
- [x] `aipk test` — TEST section CI harness (static checks + live LLM queries with contains/grounded assertions)
- [x] Epistemic v2 — IDTY/ANSP/PLCY/NKNW sections, answerability gate, identity contract, answer policy
- [x] `aipk v2-init` — scaffold Epistemic v2 template files
- [x] `aipk claims check-conflicts` — NLI conflict detection between canonical claims
- [x] Graph mode — multi-package routing via THKG embeddings (`aipk serve --graph`)
- [x] `aipk add-git` — import a git repository as knowledge base
- [x] `aipk add-web` — import web pages or a sitemap as knowledge base
- [x] AES-256-GCM encryption — `aipk encrypt` / `aipk decrypt`, transparent `--key` in serve/run
- [x] GDPR tooling — `aipk gdpr list-sources` / `snapshot` / `erase` / `report`, right of access + right-to-erasure at source or subject (person) level
- [x] Backend abstraction — dual-API embeddings (ollama + OpenAI `/v1/embeddings`), `--embed-url`
- [x] Package-as-model dispatch — select a package per request via the `model` field
- [x] Datasets — `aipk dataset init/list/sync`, `aipk add-dataset`, sources.toml (dir/web/git links)
- [x] Document parsers — PDF, DOCX, CSV/TSV, HTML in `add-docs` and datasets
- [x] Sealed packages — `aipk seal/unseal`, opaque content + Ed25519-enforced immutability
- [x] `LICN` section — license/copyright terms (license.toml), readable even when sealed
- [x] `aipk export` — unpack into Claude Code / OpenClaw / generic harness layouts
- [x] `aipk up` — autodetect backend, serve hub packages, built-in chat UI at `/`
- [x] `aipk bench-scale` — synthetic-vector benchmark for KNOW section I/O and both retrieval strategies at scale
- [x] ANN index for retrieval — `aipk build`/`pipeline` embed a pre-built HNSW index (`ANNX` section) for packages above 20,000 KNOW chunks; built once at package-build time, not on load (see [bench/scale-results.md](bench/scale-results.md))
- [x] Hard verification gate — `--enforce observe|warn|block` on `run`/`serve`/`hub serve`, withholds ungrounded strict-render answers instead of only reporting them (experimental — see [Strict serving modes](#strict-serving-modes))
- [x] Per-server/per-tool MCP allowlisting — `--allow <server>`, `--allow-tool <server:tool>`, plus signature status shown at the trust prompt (integrity only, not a publisher-trust chain — see [MCP Tools](#mcp-tools))
- [x] `aipk doctor` — checks backend reachability, model availability, and directory write access before a new user hits a confusing error
- [x] Narrower default `--help` — 22 secondary subcommands hidden from the top-level listing (`#[command(hide = true)]`); all remain fully functional via `aipk <command> --help`, listed in full in this CLI Reference

---

## License

[Business Source License 1.1](LICENSE) — free for evaluation, and for individuals or organizations with fewer than 10 employees and under $1M in annual revenue. Converts to Apache License 2.0 on 2030-07-29. See [LICENSING.md](LICENSING.md) for what needs a commercial license. Contact: support@aipk.dev.
