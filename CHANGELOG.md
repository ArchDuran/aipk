# Changelog

All notable changes to this project will be documented in this file.

## [0.2.2] — 2026-08-10

### Security & honesty fixes

- **`claims promote-all` / `--auto-promote`: close the digest-mode bypass.** The span-grounding check added in 0.2.1 only applied to verbatim-mode claims — digest mode (deliberate paraphrases) had no substring to check against, so it skipped verification entirely and still went straight to `canonical`. Digest-mode claims are now always held back as `extracted`, same as a failed span check, requiring an explicit `aipk claims promote <id>` or `reject <id>`.
- **Renamed `verdict: "grounded"` → `"lexically_supported"`, `fully_grounded` → `fully_lexically_supported`, across `verify`, `strict-render`, and `aipk run`.** The old naming implied more than word-overlap + numeric-mismatch matching actually checks. A negated sentence ("X is **not** true") or a number spelled out as words ("five hundred" instead of "500") still passes this check today — confirmed reproducible, documented as a known open gap in README rather than left implicit.
- Added a `"method": "lexical"` field to `verify` and `strict-render` JSON output — names the current check explicitly so a future pluggable verifier layer (local NLI, LLM judge, human approval) can report a different `method` without another silent terminology drift.

### 158 unit tests, 0 warnings

## [0.2.1] — 2026-08-09

### Security fixes

- **`verify` / `strict-render`: reject a claim whose numbers don't match the sentence.** Word-overlap matching alone let a fabricated number through — a sentence claiming "+500 °C" used to score `grounded`/`coverage=1.0` against a claim stating "-30 °C to +45 °C", because the surrounding words outweighed the wrong number, and short numeric tokens were being filtered out of the overlap entirely. `strict-render` used to only check that a cited `[claim_id]` existed and was canonical, never that the claim actually supported the specific sentence it was pasted onto — it now runs a real per-citation check, in both the HTTP serve path and `aipk run`.
- **`claims promote-all` / `--auto-promote`: hold back a claim whose span isn't in the source document.** `--auto-promote` skipped human review entirely, so extraction hallucinating a claim with no basis in the document went straight to `canonical` with no check at all. `promote_all` now checks each verbatim-mode claim's span against its source document before promoting, and holds back anything that doesn't match — left as `extracted`, still requiring an explicit `aipk claims promote <id>` or `reject <id>`.

### 158 unit tests, 0 warnings

## [0.2.0] — 2026-07-02

### Datasets
- `aipk dataset init / list / sync` — reusable dataset directories under `~/Datasets` (`$AIPK_DATASETS`); files in the directory are ingested automatically, `sources.toml` declares external links (`dir` = live local folders such as Obsidian vaults, `web` = pages/sitemaps, `git` = repositories; web/git are mirrored into `.cache/` by `sync`)
- `aipk add-dataset <name> --dir <project>` — chunk + embed a whole dataset into a project
- Document parsers: PDF, DOCX, CSV/TSV, HTML — in datasets **and** `aipk add-docs` (which previously only read plain text)

### Backends
- Embeddings speak both ollama `/api/embed` and OpenAI `/v1/embeddings` with per-URL auto-detection — vLLM / SGLang / llama.cpp / LM Studio work without ollama
- `--embed-url` on `serve` / `run` / `hub serve` / `add-dataset` — separate embedding endpoint from the generation backend
- Package-as-model dispatch: in multi-package `serve`, a request whose `model` equals a package name is answered by that package alone; the real backend model comes from `--model`

### Turnkey & harness integration
- `aipk up` — probe ollama/vLLM/LM Studio/llama.cpp ports, pick a model, serve hub packages
- Built-in chat UI at `/` on every `aipk serve`
- `aipk export <pkg> --to claude-code|openclaw|generic` — unpack PERS/SKIL/TOOL/KNOW into harness-native layouts (CLAUDE.md / SOUL.md, skills/, .mcp.json, knowledge/)

### Protection
- `aipk seal / unseal` — author-locked packages: content sections AES-256-GCM encrypted with a package-embedded salt key (opaque at rest), whole file Ed25519-signed; the runtime refuses sealed packages that fail verification, `extract`/`export`/`gdpr erase` are blocked, unsealing requires the author's private key
- `LICN` section — license/copyright terms from `license.toml`, always plaintext (readable even in sealed packages), shown by `aipk info`; template created by `aipk init`
- Fixed package header flag collision: SIGNED stays `0x04`, ENCRYPTED moved to `0x08`, SEALED is `0x10`

### 110 unit tests, 0 warnings

## [0.1.0] — 2026-05-09

Initial public release.

### Core format
- Binary `.aipk` format: 96-byte header + typed sections with O(1) INDX access
- Sections: META, PERS, KNOW, SKIL, TOOL, CLMS, CLMV, SRCS, IDTY, ANSP, PLCY, NKNW, THKG, TEST, SIGN, INDX

### CLI commands
- `aipk init / build / extract / info / lint`
- `aipk add-docs / add-skill / add-tool / add-git / add-web`
- `aipk run / serve` — OpenAI-compatible server (streaming, multi-package, `--graph` routing)
- `aipk extract-claims / claims / pipeline / verify` — epistemic pipeline
- `aipk hub` — local package registry (install, publish, search, serve, update)
- `aipk keygen / sign / verify-sig` — Ed25519 package signing
- `aipk test` — CI test harness
- `aipk v2-init` — Epistemic v2 scaffolding (IDTY/ANSP/PLCY/NKNW)
- `aipk encrypt / decrypt` — AES-256-GCM section encryption
- `aipk gdpr list-sources / erase / report` — GDPR right-to-erasure tooling
- `aipk init-base` — ready-to-use behavioral base package

### 88 unit tests, 0 warnings
