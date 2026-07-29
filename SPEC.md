# AIPK Format Specification v0.2

## Definition

> `.aipk` — a versioned, executable package of identity, competence boundaries, verified claims, and rules for admissible output, from which the runtime constructs only traceable answers or well-formed refusals.

**The runtime's central invariant:** no sentence leaves the system without a known `claim_id` it is grounded in.

---

## Two architecture levels

### Level 1 — RAG runtime (v1, compatible)

Classic specialization: persona + knowledge chunks + skills + tools.
The model uses the package as context, but remains the source of the answer.

### Level 2 — Epistemic runtime (v2)

The model stops being the source of truth. It performs three operations:
1. **Classifies the request** within policy (ANSP gate)
2. **Formulates the answer** from permitted claims (authorized rendering)
3. **Explicitly flags uncovered parts**

```
Request
  → [ANSP gate] — allow | allow_partial | refuse_* | escalate_review
       ↓ allow
  → [CLMS retrieval] — canonical claims only
  → [PLCY check] — which transformations are permitted
  → [Authorized rendering] — the model formulates, does not invent
  → [Sentence-to-claim verification] — every sentence is traced
  → [Emit] — answer + provenance + uncovered
```

---

## File structure

### Magic + Header (96 bytes)

```
Bytes  0- 3:  u8[4]   Magic: "AIPK"
Bytes  4- 7:  u32     Format version (current: 1)
Bytes  8-71:  u8[64]  Package name, null-terminated UTF-8
Bytes 72-79:  u64     Creation timestamp (Unix seconds)
Bytes 80-83:  u32     Section count
Bytes 84-87:  u32     Flags
Bytes 88-95:  u64     Byte offset to the INDX section
```

**Flags:**
```
Bit 0: COMPRESSED — sections are compressed
Bit 1: ENCRYPTED  — sections are encrypted (AES-256-GCM)
Bit 2: SIGNED     — file is Ed25519-signed
```

### Section structure (16-byte header)

```
Bytes 0- 3:  u8[4]   Section type (4-char ASCII)
Bytes 4- 7:  u32     Section flags
Bytes 8-15:  u64     Data size
[DATA: SIZE bytes]
```

---

## Sections

### Identity and contract sections (v2, epistemic)

#### `IDTY` — Identity Contract (JSON)

The immutable core: who this is, how it speaks, how it behaves, what it is entitled to speak about.

```json
{
  "name": "Alexa",
  "role": "legal assistant for Romashka Inc.",
  "style": "formal",
  "language": "en",
  "operating_policy": {
    "never_speculate": true,
    "cite_sources": true,
    "admit_uncertainty": true
  },
  "competence_scope": ["labor law", "contracts", "company regulations"]
}
```

Split into four components:
- `name` + `role` → **identity**: who this is
- `style` + `language` → **style**: how it speaks
- `operating_policy` → **operating_policy**: how it behaves
- `competence_scope` → **competence_scope**: what it is entitled to speak about

> **Note:** `identity` is immutable. `operating_policy` is versioned. `knowledge` is growable. They are never mixed together.

---

#### `CLMS` — Claims (JSONL)

Atomic claims with a lifecycle and audit trail.

```jsonl
{"id": "regs_0012", "text": "Personal data retention must not exceed 5 years under Article 17.", "source": "regulations.pdf", "span": "retention period shall not exceed five years", "status": "canonical", "confidence": 0.95, "audit": [{"action": "extract", "model": "llama3.2", "at": "2026-05-01T08:00:00Z"}, {"action": "promote", "from": "extracted", "to": "canonical", "reviewer": "dr-smith", "reason": "verified against source paragraph 3.2", "at": "2026-05-02T10:00:00Z"}]}
{"id": "regs_0013", "text": "An employment contract must be concluded in written form.", "source": "regulations.pdf", "span": "concluded in written form", "status": "extracted", "confidence": 0.9, "audit": [{"action": "extract", "model": "llama3.2", "at": "2026-05-01T08:00:01Z"}]}
```

**Record fields:**

| Field | Type | Description |
|------|-----|----------|
| `id` | string | Unique identifier (format: `<prefix>_<seq>`) |
| `text` | string | The atomic claim — one self-contained sentence |
| `source` | string | Name of the source file |
| `span` | string | Exact quote from the source the claim was extracted from |
| `status` | string | Current lifecycle status |
| `confidence` | float | Confidence at extraction time (0.0–1.0) |
| `audit` | array | Chronological chain of actions |

**Lifecycle statuses:**

| Status | Description |
|--------|----------|
| `extracted` | Automatically extracted by the LLM, not reviewed by a human |
| `reviewed` | Reviewed by a human, not yet promoted |
| `canonical` | Authoritative, used in strict modes |
| `deprecated` | No longer valid |

**Only `canonical` claims are used in `--strict-verify` and `--strict-render` modes.**

**Audit entry structure:**

```json
{
  "action": "promote" | "reject" | "extract",
  "from": "extracted",
  "to": "canonical",
  "reviewer": "dr-smith",
  "reason": "verified against source paragraph 3.2",
  "model": "llama3.2",
  "at": "2026-05-02T10:00:00Z"
}
```

On `extract`: fields `model` + `at`.
On `promote` / `reject`: fields `from`, `to`, `at`, optionally `reviewer` + `reason`.

When loading a package, the runtime filters out `deprecated` claims — they never enter retrieval.

---

---

#### `CLMV` — Claim Vectors (binary, optional)

Vector representations of the claims in the `CLMS` section (parallel array: the i-th vector corresponds to the i-th record in CLMS).
Used by the runtime for semantic claim search via cosine similarity instead of Jaccard.
If the section is absent, the runtime automatically falls back to keyword (Jaccard) matching.

```
Bytes 0-3:  u32    Number of claims (must match the number of records in CLMS)
Bytes 4-7:  u32    Vector dimensionality (embedding_dim)
[float32 × dim] × claim_count   — little-endian, one vector per claim
```

**Invariant:** `claim_count × dim × 4` must exactly match the data bytes following the header.

This section is built automatically by the `aipk pipeline` command at the embed-claims step.
During `aipk build`, it is included in the package if a vectors file is present at `.aipk/claim_vectors.bin`.

---

#### `SRCS` — Sources & Provenance (JSONL)

Registry of source documents that claims were extracted from.

```jsonl
{"id": "s_regulations", "title": "regulations.pdf", "path": "/data/legal/regulations.pdf"}
{"id": "s_policy", "title": "policy.md", "path": "/data/internal/policy.md"}
```

**Fields:**

| Field | Description |
|------|----------|
| `id` | `s_<stem>` — unique source identifier |
| `title` | File name |
| `path` | Full path to the file at import time |

The source `id` is used as the key when referenced from external systems. The `source` field itself in CLMS stores the `title` (file name) for human readability.

---

#### `PLCY` — Answer Policy (JSON)

Rules for admissible output: what the model is allowed to do with claims.

```json
{
  "allowed_transformations": ["rephrase", "summarize", "cite", "translate"],
  "forbidden": ["infer_beyond_claims", "speculate", "use_model_priors", "extrapolate"],
  "citation_required": true,
  "partial_answer_allowed": true,
  "confidence_threshold": 0.72
}
```

Key principle: `forbidden` contains what the model cannot do **architecturally** — not "please don't do this," but "the runtime will reject the answer if it detects this."

---

#### `ANSP` — Answerability Policy (JSON)

Policy machine: decides whether the runtime is entitled to answer, and in what mode.

```json
{
  "domain": ["labor law", "contracts", "regulations", "statute of limitations"],
  "confidence_threshold": 0.72,
  "adjacent_allowed": true,
  "responses": {
    "allow": null,
    "allow_partial": null,
    "refuse_unknown": "I don't have verified information on this topic.",
    "refuse_out_of_scope": "This is outside my area of competence. I specialize in legal matters for Romashka Inc.",
    "refuse_conflict": "I have conflicting data on this topic. Please check with a lawyer.",
    "escalate_review": "This question requires review by a specialist."
  }
}
```

**ANSP output modes:**

| Mode | Condition | Runtime action |
|---|---|---|
| `allow` | Claims cover the request, confidence ≥ threshold | Authorized rendering |
| `allow_partial` | Partial coverage, ≥ 60% of threshold | Rendering with gaps flagged |
| `refuse_unknown` | No claims found | Returns text without invoking the model |
| `refuse_out_of_scope` | Request is outside the domain | Returns text without invoking the model |
| `refuse_conflict` | Conflicting canonical claims detected | Returns text without invoking the model |
| `escalate_review` | Borderline case, requires a human | Returned with an escalation flag |

---

#### `NKNW` — Negative Knowledge (JSONL)

Registry of what the system doesn't know, what it's forbidden from asserting, and which questions are frequently left uncovered.

```jsonl
{"type": "unknown", "topic": "tax law", "message": "Tax questions are outside the area of competence"}
{"type": "forbidden", "pattern": "case-specific advice", "message": "Cannot give case-specific legal advice — only general information"}
{"type": "uncovered", "question": "What is the statute of limitations for administrative cases?", "count": 7, "last_seen": "2026-04-28"}
```

Types: `unknown` | `forbidden` | `uncovered`

`uncovered` entries accumulate automatically — a signal that new claims should be added to CLMS.

---

#### `TEST` — Test Harness (JSON)

Built-in tests: expected response modes and claims for specific queries.

```json
{
  "cases": [
    {
      "query": "What is the statute of limitations for a supply contract?",
      "expected_mode": "allow",
      "expected_claim_ids": ["c001"],
      "should_refuse": false
    },
    {
      "query": "How do I buy Apple stock?",
      "expected_mode": "refuse_out_of_scope",
      "should_refuse": true
    },
    {
      "query": "Tell me specifically what to do in my case",
      "expected_mode": "refuse_out_of_scope",
      "should_refuse": true
    }
  ]
}
```

`aipk test <pkg>` — runs all cases and reports pass/fail.

---

### Knowledge sections (v1, RAG-compatible)

#### `META` — Metadata (TOML, required)

```toml
[package]
name        = "legal-assistant"
version     = "1.0.0"
author      = "Romashka Inc."
description = "Legal assistant"
license     = "proprietary"

[model]
compatible      = ["qwen2.5", "llama3.2", "mistral"]
min_context     = 4096
embedding_model = "all-MiniLM-L6-v2"
embedding_dim   = 384

[runtime]
mcp_autostart = false
rag_top_k     = 5
rag_min_score = 0.7

[graph]
role      = "specialist"
depth     = 2
max_nodes = 5

[consolidation]
strategy  = "idle"
idle_min  = 30
keep_top  = 0.7
hebbian   = true
```

#### `PERS` — Persona (plain text / Markdown, optional)

System prompt for RAG mode. Replaced by `IDTY` in epistemic mode.

#### `KNOW` — Knowledge base (binary, optional)

```
Bytes  0- 7: u64    Offset to the chunks block
Bytes  8-15: u64    Offset to the vectors block
Bytes 16-19: u32    Number of chunks
Bytes 20-23: u32    Embedding dimensionality

CHUNKS: gzip(JSONL) — {"id":0,"text":"...","source":"doc.pdf","meta":{}}
VECTORS: raw float32[], little-endian
```

#### `SKIL` — Skills (optional)

```
Bytes  0- 7: u64    Offset to the manifest
Bytes  8-15: u64    Offset to the files

MANIFEST: JSON [{"name":"...", "file":"...", "trigger":"..."}]
FILES: [filename_len:u16][filename][content_len:u32][content] × N
```

#### `TOOL` — MCP tools (JSON, optional)

```json
{"mcp_servers": [{"name": "filesystem", "command": "npx", "args": [...], "env": {}}]}
```

---

### Graph sections (v2)

#### `LINK` — Connection graph (JSON, optional)

```json
{
  "links": [
    {"package": "gdpr.aipk", "rel": "specializes", "weight": 0.9, "triggers": ["gdpr"]}
  ],
  "router_hint": "EU legal compliance"
}
```

`rel` types: `specializes` | `complements` | `requires` | `conflicts`

Weights are updated automatically (Hebbian learning).

#### `THKG` — Router instructions (binary, optional)

```
Bytes  0- 7: u64    Offset to router_prompt
Bytes  8-15: u64    Offset to files
router_prompt: plain text
files: [filename_len:u16][filename][content_len:u32][content] × N
```

---

### Service sections

#### `MEMS` — Episodic memory (JSON, optional)

```json
{
  "sessions": [{"ts": "...", "summary": "...", "key_facts": ["..."]}],
  "shell_history": [{"model": "qwen2.5", "quality_score": 0.91}]
}
```

#### `STAT` — Statistics (JSON, optional)

```json
{
  "chunk_hits": {"42": 87},
  "link_traversals": {"gdpr.aipk": 45},
  "skill_triggers": {"review": 23},
  "last_consolidated": "2026-04-28T03:00:00Z"
}
```

#### `SIGN` — Ed25519 signature (always the last section, after INDX, not indexed)

Section header (16 bytes) + 98 bytes of data = 114 bytes total.

```
Section header (16 bytes):
  Bytes  0-  3: u8[4]   "SIGN"
  Bytes  4-  7: u32     Section flags (0)
  Bytes  8- 15: u64     Data size (98)

Section DATA (98 bytes):
  Bytes  0- 31: u8[32]  Ed25519 public key (verifying key)
  Bytes 32- 95: u8[64]  Ed25519 signature
  Bytes 96- 96: u8      Algorithm version (0x01 = Ed25519)
  Bytes 97- 97: u8      Reserved (0x00)
```

**What is signed:** all file bytes from the start up to the start of the SIGN section (i.e. header + all sections + INDX), with the `SIGNED` flag (bit 2) set in the package header.

**File position:** SIGN always comes after INDX and is not included in INDX. The parser locates it by sequential reading beyond the index.

**Header flag:** signing sets `Bit 2: SIGNED` in the flags field (bytes 84–87 of the package header). This allows quickly determining whether a signature is present without parsing the whole file.

#### `INDX` — Index (always last)

```
Bytes 0-3: u32    Number of entries
[type:u8[4]][offset:u64][size:u64] × N
```

---

## Table of all sections

| Section | Mode | Data type | Description |
|--------|-------|-----------|----------|
| `META` | v1+v2 | TOML | Package metadata (required) |
| `PERS` | v1    | text | Persona / system prompt |
| `KNOW` | v1    | binary | gzip(JSONL chunks) + float32 vectors |
| `SKIL` | v1    | binary | JSON manifest + markdown files |
| `TOOL` | v1    | JSON | MCP server config |
| `IDTY` | v2    | JSON | Identity contract (immutable) |
| `CLMS` | v2    | JSONL | Canonical claims + lifecycle |
| `CLMV` | v1+v2 | binary | float32 vectors for claims (parallel to CLMS) |
| `SRCS` | v2    | JSONL | Source provenance |
| `PLCY` | v2    | JSON | Answer policy (allowed transformations) |
| `ANSP` | v2    | JSON | Answerability policy (gate) |
| `NKNW` | v2    | JSONL | Negative knowledge registry |
| `TEST` | v2    | JSON | Test harness (expected modes & claims) |
| `LINK` | graph | JSON | Package graph connections |
| `THKG` | graph | binary | Router instructions |
| `MEMS` | v1+v2 | JSON | Episodic memory |
| `STAT` | v1+v2 | JSON | Usage statistics |
| `SIGN` | sec   | binary | Ed25519 signature |
| `INDX` | core  | binary | Section directory |

---

## Claim lifecycle — state machine

```
aipk extract-claims
        ↓
   extracted  ──────────────────────────────► deprecated
        │                                         ↑
        │  aipk claims promote                    │ aipk claims reject
        ▼                                         │
    reviewed  ──────────────────────────────► deprecated
        │                                         ↑
        │  aipk claims promote                    │ aipk claims reject
        ▼                                         │
   canonical  ──────────────────────────────► deprecated
```

**Transition rules:**

| Action | Command | Result |
|----------|---------|---------|
| LLM extracted a fact | `aipk extract-claims` | `extracted` + first audit entry |
| Reviewed by a human | `aipk claims promote <id> --status reviewed` | `reviewed` + audit |
| Confirmed as authoritative | `aipk claims promote <id>` | `canonical` + audit |
| Recognized as invalid | `aipk claims reject <id> --reason "..."` | `deprecated` + audit |

**Invariant:** only `canonical` claims are used in `--strict-verify` and `--strict-render`.
`deprecated` claims are filtered out when the package loads and never enter retrieval or rendering.

Every status change is recorded in `audit[]` with reviewer, reason, and timestamp — the field is immutable, append-only.

---

## Threat model

| Threat | Mitigation |
|--------|----------|
| Prompt injection from the user | ANSP gate blocks before the model is invoked |
| Conflicting claims | `refuse_conflict` mode, detected in ANSP |
| Stale knowledge | `valid_until` on a claim, `uncovered` in NKNW |
| Poisoned MEMS | `observed → candidate → validated` — manual review before promotion |
| Over-broad promotion | Only `canonical` used in rendering, PLCY forbids `infer_beyond_claims` |
| Model hallucination in rendering | Sentence-to-claim verification rejects uncovered sentences |

---

## Implementation status (v0.1 Rust runtime)

| Section | Implemented | Note |
|--------|-------------|-----------|
| `META` | ✓ | TOML, required |
| `PERS` | ✓ | plain text persona |
| `KNOW` | ✓ | gzip JSONL + float32 vectors, cosine retrieval |
| `SKIL` | ✓ | JSON manifest + markdown, keyword trigger |
| `TOOL` | ✓ | MCP stdio, JSON-RPC 2.0 tool calling |
| `CLMS` | ✓ | JSONL, full lifecycle, audit trail |
| `CLMV` | ✓ | float32 claim vectors, cosine semantic matching; fallback to Jaccard |
| `SRCS` | ✓ | JSONL source registry |
| `INDX` | ✓ | section index |
| `IDTY` | — | roadmap: v2 epistemic |
| `PLCY` | — | roadmap: v2 epistemic |
| `ANSP` | — | roadmap: v2 epistemic |
| `NKNW` | — | roadmap: v2 epistemic |
| `TEST` | — | roadmap: `aipk test` |
| `LINK` | — | roadmap: graph mode |
| `THKG` | — | roadmap: graph mode |
| `MEMS` | — | roadmap |
| `STAT` | — | roadmap |
| `SIGN` | ✓ | Ed25519, 98 bytes of data, always after INDX |

The current strict modes (`--strict-verify`, `--strict-render`) implement a subset of the v2 epistemic pipeline via CLMS + SRCS without ANSP/PLCY/IDTY.

---

## Compatibility

`.aipk` is compatible with any OpenAI-compatible LLM:
- ollama: `http://localhost:11434`
- LM Studio: `http://localhost:1234`
- any OpenAI-compatible endpoint

```bash
# Normal mode (RAG)
aipk serve agent.aipk --llm-url http://localhost:1234 --model qwen2.5-7b

# Force response language
aipk serve agent.aipk --llm-url http://localhost:1234 --model qwen2.5-7b --lang ru

# Strict mode — canonical claims only
aipk serve agent.aipk --strict-verify --llm-url http://localhost:1234 --model qwen2.5-7b

# Strict rendering — the LLM must cite [claim_id]
aipk serve agent.aipk --strict-render --llm-url http://localhost:1234 --model qwen2.5-7b
```

`aipk serve` supports SSE streaming: the `"stream": true` field in the request. Normal mode proxies the stream directly; strict modes convert the final JSON into SSE.

### `aipk lint`

Static persona analysis before building a package:

```bash
aipk lint ./my-project      # checks persona.md in the directory
aipk lint agent.aipk        # checks the PERS section from a package
```

Checks:
- Empty / too-short persona
- Prompt injection patterns: `ignore previous instructions`, `jailbreak`, `dan mode`, `developer mode`, etc.
- Homoglyphs (Cyrillic characters in an ASCII context)
- Missing explicit language directive

Exits `1` if ERROR-level issues are found.

### `aipk init-base`

Generates a ready-to-use behavioral package optimized for small models (1b+):

```bash
aipk init-base --lang en                      # English persona and skills
aipk init-base --lang ru                      # Russian
aipk init-base --lang auto                    # bilingual (EN + RU)
aipk init-base --lang en --test --model llama3.2  # with self-test (5 queries)
```

Includes 3 skills (factual-answer, command-safety, uncertainty) and 6 canonical claims (EN+RU pairs).

### `aipk verify --min-coverage`

```bash
# CI: fail if coverage < 80%
aipk verify agent.aipk "Answer text..." --min-coverage 0.8 --json
```

Exit codes: `0` — coverage ≥ threshold; `1` — below threshold or unsupported sentences; `2` — error.
