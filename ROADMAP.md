# AIPK — Roadmap

## Phase 1: Specification ✓

- [x] Format and section concept
- [x] Byte-level structure (SPEC.md v0.2)
- [x] RFC: edge cases, compatibility
- [x] Epistemic architecture (v2 design)

---

## Phase 2: Rust runtime v0.1 ✓

Full migration from Python to Rust. A single binary (4.4 MB), zero dependencies besides ollama.

- [x] Binary parser and builder (META, PERS, KNOW, SKIL, TOOL, CLMS, SRCS, INDX)
- [x] `aipk init / info / build / extract`
- [x] `aipk add-docs` — chunking + ollama /api/embed
- [x] `aipk add-skill / add-tool`
- [x] `aipk run` — one-shot RAG query
- [x] `aipk serve` — OpenAI-compatible HTTP server
  - [x] Multi-package: ComposedRuntime, cross-package RAG merge
  - [x] `--strict-verify` — refuse if no canonical grounding
  - [x] `--strict-render` — LLM must cite [claim_id], _aipk coverage metrics
- [x] MCP tool calling (stdio JSON-RPC 2.0, tool loop)
- [x] `aipk mcp` — expose package as MCP stdio server

---

## Phase 3: Epistemic pipeline ✓

Claim-grounded answers with a full audit trail.

- [x] `aipk extract-claims` — LLM → atomic facts → claims.jsonl
- [x] Claim lifecycle: extracted → reviewed → canonical → deprecated
- [x] Audit trail in claims.jsonl (action, reviewer, reason, timestamp)
- [x] `aipk claims list / stats / promote / reject`
- [x] `aipk verify` — sentence-level provenance report
  - [x] JSON output: verdict, claim_id, source, span, score
  - [x] Exit codes: 0=grounded, 1=unsupported, 2=error

---

## Phase 4: Polish ✓

- [x] README — rewritten for the real Rust CLI
- [x] SPEC.md — added the current CLMS format, lifecycle, audit trail
- [x] Dead code removed, zero compiler warnings
- [x] Streaming responses in `aipk serve` (`stream: true`, SSE proxy + synthetic SSE)
- [x] `aipk lint` — static PERS analysis (injection patterns, homoglyphs, language directive)
- [x] `aipk claims promote-all` — batch promote with audit trail
- [x] `aipk serve --lang <code>` — force response language
- [x] `aipk init-base --test` — self-test with 5 queries after building
- [x] `CLMV` section — float32 claim vectors, cosine semantic matching (cross-language)
- [x] `aipk pipeline` — manifest auto-update + claims embedding step
- [x] `aipk hub` — local package registry (install, search, publish, serve, update, status)
- [x] 46 unit tests (lint, CLMV cosine, streaming, init, verify, hub)
- [x] Ed25519 signatures — `aipk keygen / sign / verify-sig`, SIGN section (98 bytes), tamper detection
- [x] `aipk test` — TEST section (tests.json → TEST section in the package), static checks + live LLM tests

---

## Phase 5: Epistemic v2 ✓

Full implementation of the policy machine from SPEC.md v0.2.

- [x] `IDTY` section — immutable identity contract (inject_prefix → system prompt)
- [x] `PLCY` section — answer policy (citation_required, forbidden behaviors)
- [x] `ANSP` section — answerability gate (allow/partial/refuse/escalate, domain keywords + coverage threshold)
- [x] `NKNW` section — negative knowledge registry (forbidden topics → early refusal, unknown notes → hints)
- [x] `aipk v2-init` — creates identity.json / answerability.json / policy.json / negative.jsonl templates
- [x] `aipk claims check-conflicts` — NLI conflict detection via LLM (comparing canonical claim pairs)
- [x] v2 runtimes integrated into `aipk serve` and `aipk run`
- [x] `aipk build` reads IDTY/ANSP/PLCY/NKNW from project files
- [x] `aipk info` shows v2 section details

---

## Phase 6: Ecosystem

- [x] `aipk hub` — local registry (publish, search, install, serve) ← implemented in Phase 4
- [x] Graph mode — multi-package routing (THKG + LINK sections, `aipk serve --graph`)
  - THKG section: plain-text description of the package's domain for the router (thkg.md)
  - LINK section: graph edges between packages (links.json)
  - Embedding-based routing: cosine similarity of query vs THKG → selects the best package
  - `aipk init` creates a thkg.md template, `aipk build` packages it
  - Per-package runtimes: routing uses only the persona/RAG/claims of the selected package
  - 6 unit tests (best_route, threshold, empty, ThkgRuntime)
- [x] `aipk add-git` — import from a git repository (shallow clone, extension filter, `--include-code`)
- [x] `aipk add-web` — import from URL / sitemap (HTML stripping, sitemap XML parsing, rate limiting)
- [ ] Hub — remote registry (web UI, public registry)
- [x] Backend abstraction — dual-API embeddings (ollama `/api/embed` + OpenAI `/v1/embeddings`, autodetect with per-URL cache), `--embed-url` in serve/run/hub serve
- [x] Package-as-model dispatch — the `model` field in the request selects the package; the actual backend model comes from `--model`
- [x] Datasets — `aipk dataset init/list/sync` + `aipk add-dataset`
  - `~/Datasets/<name>/` directory ($AIPK_DATASETS), files picked up automatically
  - sources.toml: `[[link]]` type = dir (live read, Obsidian) / web (sitemap, mirrored into .cache) / git (shallow clone, mirrored)
  - Parsers: PDF (pdf-extract), DOCX (zip+XML), CSV/TSV (row = paragraph), HTML — also available in add-docs
  - A dataset holds only raw text; embeddings are computed when ingested into a project
- [x] Sealed packages — `aipk seal/unseal`
  - Content is AES-256-GCM encrypted with a key derived from an embedded salt (SEAL section, 18 bytes) — the file is opaque without the runtime
  - Mandatory Ed25519 signature; the runtime refuses to load a sealed package with an invalid signature
  - extract/export/gdpr erase are blocked; unseal only with the author's key
  - Fixed a header flag collision: SIGNED=0x04, ENCRYPTED=0x08, SEALED=0x10
- [x] LICN section — license.toml (author, license, terms), always plaintext, visible in `aipk info` even for sealed packages
- [x] `aipk export --to claude-code|openclaw|generic` — PERS→CLAUDE.md/SOUL.md, SKIL→skills/, TOOL→.mcp.json, KNOW→knowledge/
- [x] `aipk up` — backend autodetection (11434/8000/1234/8081), model selection, hub serve
- [x] Built-in chat UI at `/` in `aipk serve`
- [x] Section encryption (AES-256-GCM)
  - `aipk encrypt <pkg> --key <passphrase>` — encrypts content sections (PERS/KNOW/SKIL/CLMS/CLMV/SRCS/IDTY/ANSP/PLCY/NKNW/TOOL/THKG/TEST)
  - `aipk decrypt <pkg> --key <passphrase>` — decrypts
  - `aipk serve/run --key <passphrase>` — transparent decryption at runtime (in memory)
  - Format: `[nonce 12B][ciphertext + GCM tag 16B]`, AAD = section tag (protects against section reordering)
  - Key: Argon2id(passphrase, random per-package salt) → 32-byte AES-256 key
  - `SECTION_FLAG_ENCRYPTED = 0x08` flag in each section header
  - 6 unit tests (roundtrip, wrong key, wrong AAD, nonce randomness)
- [x] GDPR tooling
  - `aipk gdpr list-sources <pkg>` — list of sources with chunk counts
  - `aipk gdpr erase <pkg> --source <name>` — removes all data for a source from KNOW/CLMS/SRCS
  - `aipk gdpr erase --dry-run` — preview without writing
  - `aipk gdpr report <pkg>` — compliance report (sections, sources, signature, encryption)
