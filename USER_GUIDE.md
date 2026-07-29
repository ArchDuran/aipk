# AIPK — User Guide

A complete guide: from installation to integrating with any application.

---

## Table of Contents

1. [What is AIPK](#1-what-is-aipk)
2. [Installation](#2-installation)
3. [Installing and Configuring Ollama](#3-installing-and-configuring-ollama)
4. [Quick Start — Your First Package in 5 Minutes](#4-quick-start--your-first-package-in-5-minutes)
5. [Building a Package from Your Documents](#5-building-a-package-from-your-documents)
6. [Managing Claims (Epistemic Mode)](#6-managing-claims-epistemic-mode)
7. [Full Automated Pipeline](#7-full-automated-pipeline)
8. [Running the Server and Integrating with Applications](#8-running-the-server-and-integrating-with-applications)
9. [Base Behavioral Package (aipk-base)](#9-base-behavioral-package-aipk-base)
10. [Working with Multiple Packages](#10-working-with-multiple-packages)
11. [Strict Modes — strict-verify and strict-render](#11-strict-modes--strict-verify-and-strict-render)
12. [Integrating with External Applications](#12-integrating-with-external-applications)
13. [Command Reference](#13-command-reference)
14. [Recommended Models](#14-recommended-models)
15. [Common Issues](#15-common-issues)

---

## 1. What is AIPK

**AIPK** (AI Package) is a knowledge packaging format for language models. A single `.aipk` file contains:

- **Persona** — who this agent is, how it behaves
- **Skills** — specialized instructions that get pulled in for certain queries
- **Knowledge (RAG)** — a vector knowledge base built from your documents
- **Claims** — verified claims with an audit trail (for strict modes)
- **MCP Tools** — external tools connected via stdio

The AIPK server exposes an **OpenAI-compatible API** — any application that can talk to ChatGPT will work with AIPK without code changes.

---

## 2. Installation

### Requirements

- **Rust** 1.75+ (to build from source)
- **Ollama** (for local models)

### Installing Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

Verify:
```bash
rustc --version   # rustc 1.75.0 or newer
cargo --version
```

### Building AIPK

```bash
git clone <repository>
cd aipk-package-format/aipk-rs
cargo build --release
```

The binary will be at `./target/release/aipk`. For convenience, add it to your PATH:

```bash
# Linux / macOS
cp ./target/release/aipk ~/.local/bin/
# or
export PATH="$PATH:/path/to/aipk-rs/target/release"
```

Verify:
```bash
aipk --version   # aipk 0.1.0
aipk --help
```

---

## 3. Installing and Configuring Ollama

Ollama is a local server for running language models. AIPK uses it for:
- Generating responses (chat completions)
- Creating embeddings (vectorizing text for RAG)
- Extracting claims from documents

### Installing Ollama

**Linux:**
```bash
curl -fsSL https://ollama.com/install.sh | sh
```

**macOS:**
```bash
brew install ollama
```

**Windows:**
Download the installer from [ollama.com](https://ollama.com/download).

### Starting the Ollama server

```bash
ollama serve
# Listens on http://localhost:11434 by default
```

On most systems, Ollama starts automatically as a systemd service.
Check its status:
```bash
curl http://localhost:11434/api/tags
```

### Downloading models

**For generating responses:**
```bash
# Lightweight model (2 GB, good speed/quality balance)
ollama pull qwen2.5:3b

# More capable (4.7 GB, better multilingual quality)
ollama pull qwen2.5:7b

# English-focused model (good for EN-only packages)
ollama pull llama3.2:3b
```

**For vectorization (RAG and claim extraction):**
```bash
ollama pull nomic-embed-text
```

Check downloaded models:
```bash
ollama list
```

> **Tip:** `qwen2.5:3b` is the best choice to start with. It handles multiple languages well and weighs only ~2 GB.

---

## 4. Quick Start — Your First Package in 5 Minutes

Let's create a minimal assistant package without documents:

```bash
# 1. Create the project structure
aipk init my-assistant

# The my-assistant/ folder will contain:
#   manifest.toml  — package metadata
#   persona.md     — the assistant's persona
#   tools.json     — MCP tools (empty)
#   links.json     — links to other agents (empty)
#   skills/        — folder for specialized instructions

# 2. Edit persona.md
cat > my-assistant/persona.md << 'EOF'
# Assistant

You are a helpful AI assistant. Reply in the same language the question was asked in.
Be concise and to the point. If you don't know something, say so honestly.
EOF

# 3. Build the package
aipk build my-assistant

# This creates my-assistant/my-assistant.aipk
aipk info my-assistant/my-assistant.aipk

# 4. Try it out
aipk run my-assistant/my-assistant.aipk "What is a quantum?" \
  --model qwen2.5:3b \
  --llm-url http://localhost:11434
```

---

## 5. Building a Package from Your Documents

Suppose you have PDF, markdown, or plain text files with documentation.

### Step 1: Create the project

```bash
aipk init my-docs-bot
```

### Step 2: Edit persona.md

```bash
cat > my-docs-bot/persona.md << 'EOF'
# Documentation Assistant

You help users understand our product.
Answer questions based on the documentation you've been given.
If the information isn't in the documentation, say so explicitly.
EOF
```

### Step 3: Add documents (this builds the vector database)

```bash
aipk add-docs my-docs-bot \
  --dir my-docs-bot \
  docs/guide.md docs/api.md docs/faq.txt \
  --embed-model nomic-embed-text \
  --llm-url http://localhost:11434
```

For each file:
- The text is split into chunks (~400 tokens)
- Each chunk is vectorized via `nomic-embed-text`
- Vectors are stored in `.aipk/`

### Step 4: Add skills (optional)

A skill is a specialized instruction that's pulled in when a question contains a trigger word:

```bash
cat > my-docs-bot/skills/api-help.md << 'EOF'
---
name: api-help
trigger: api
---
When answering API-related questions:
1. Always include the endpoint path
2. Show an example request
3. Mention required parameters
EOF

aipk add-skill my-docs-bot/skills/api-help.md --dir my-docs-bot
```

### Step 5: Build the package

```bash
aipk build my-docs-bot
# → my-docs-bot/my-docs-bot.aipk
```

### Step 6: Test it

```bash
aipk run my-docs-bot/my-docs-bot.aipk "How do I authenticate via the API?" \
  --model qwen2.5:3b \
  --embed-model nomic-embed-text \
  --llm-url http://localhost:11434 \
  --verbose
```

The `--verbose` flag shows how many chunks were injected from RAG.

---

## 6. Managing Claims (Epistemic Mode)

A **claim** is a verified atomic statement about your domain. Claims let you:
- Check AI responses against your documents
- Audit the source of every statement
- Operate in strict modes (answers drawn only from verified data)

### Claim lifecycle

```
extracted → reviewed → canonical → deprecated
```

- **extracted** — automatically extracted by the LLM, not reviewed by a human
- **reviewed** — reviewed, but not yet promoted
- **canonical** — authoritative, used in strict modes
- **deprecated** — outdated, no longer used

### Extracting claims from documents

```bash
aipk extract-claims docs/guide.md docs/api.md \
  --dir my-docs-bot \
  --model qwen2.5:3b \
  --llm-url http://localhost:11434
```

### Reviewing claims

```bash
# All claims
aipk claims list --dir my-docs-bot

# Only extracted (not yet reviewed)
aipk claims list --dir my-docs-bot --status extracted

# Stats by status
aipk claims stats --dir my-docs-bot
```

Example output:
```
Claims: 47 total
  ✓ canonical   12  ██████░░░░░░░░░░░░░░  26%
  ○ reviewed     5  ██░░░░░░░░░░░░░░░░░░  11%
  · extracted   28  ████████████░░░░░░░░  60%
  ✗ deprecated   2  ░░░░░░░░░░░░░░░░░░░░   4%
```

### Interactive claim review

```bash
aipk claims review --dir my-docs-bot --reviewer "Ivan"
```

For each claim:
- `y` — promote to canonical
- `n` — reject (deprecated), will ask for a reason
- `s` — skip
- `q` — save and quit

### Bulk promotion

If you fully trust your documents:
```bash
aipk claims promote-all --dir my-docs-bot --reviewer "auto"
```

### Checking an answer against claims

```bash
aipk verify my-docs-bot/my-docs-bot.aipk \
  "The API uses Bearer token authentication and requires HTTPS."
```

The output shows which sentence is backed by which claim, and the coverage percentage.

For CI/CD:
```bash
# Exit 0 if everything is grounded, 1 if there are unsupported sentences
aipk verify my-docs-bot/my-docs-bot.aipk "answer..." --min-coverage 0.8
echo $?
```

---

## 7. Full Automated Pipeline

The `pipeline` command combines all the steps into one:

```bash
aipk pipeline docs/*.md \
  --dir my-docs-bot \
  --model qwen2.5:3b \
  --embed-model nomic-embed-text \
  --llm-url http://localhost:11434 \
  --auto-promote \
  --output my-docs-bot/my-docs-bot.aipk
```

Flags:
- `--auto-promote` — automatically promotes all claims to canonical (no manual review)
- `--review` — opens an interactive claim review before building
- `--reviewer <name>` — reviewer name for the audit trail

What happens internally:
1. `add-docs` — vectorizes the documents
2. `extract-claims` — extracts claims via the LLM
3. `claims review` or `claims promote-all` — promotes claims
4. `build` — builds the `.aipk` file

---

## 8. Running the Server and Integrating with Applications

### Starting the server

```bash
aipk serve my-docs-bot/my-docs-bot.aipk \
  --model qwen2.5:3b \
  --llm-url http://localhost:11434 \
  --embed-model nomic-embed-text \
  --port 8080
```

The server runs at `http://0.0.0.0:8080/v1/`.

Verify:
```bash
curl http://localhost:8080/health
```

### Testing it with curl

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "How does the authentication API work?"}],
    "model": "my-docs-bot"
  }'
```

### Integrating with Open WebUI

[Open WebUI](https://github.com/open-webui/open-webui) is a web chat interface.

```bash
# Run Open WebUI via Docker
docker run -d \
  -p 3000:8080 \
  -e OPENAI_API_BASE_URL=http://host.docker.internal:8080/v1 \
  -e OPENAI_API_KEY=none \
  --name open-webui \
  ghcr.io/open-webui/open-webui:main
```

Open `http://localhost:3000` → select the `my-docs-bot` model → the chat is ready.

### Integrating with Chatbox

[Chatbox](https://chatboxai.app/) is a desktop chat client.

1. Settings → API Provider → choose **OpenAI API**
2. API Host: `http://localhost:8080`
3. API Key: (anything, e.g. `none`)
4. Model: `my-docs-bot`

### Integrating with LibreChat

```yaml
# in LibreChat's .env file:
OPENAI_API_KEY=none
OPENAI_REVERSE_PROXY=http://localhost:8080/v1
```

### Integrating with Continue.dev (VS Code / JetBrains)

File `~/.continue/config.json`:
```json
{
  "models": [
    {
      "title": "My Docs Bot",
      "provider": "openai",
      "model": "my-docs-bot",
      "apiBase": "http://localhost:8080/v1",
      "apiKey": "none"
    }
  ]
}
```

### Python / OpenAI SDK

```python
from openai import OpenAI

client = OpenAI(
    api_key="none",
    base_url="http://localhost:8080/v1"
)

response = client.chat.completions.create(
    model="my-docs-bot",
    messages=[
        {"role": "user", "content": "How does authentication work?"}
    ]
)
print(response.choices[0].message.content)
```

### JavaScript / Node.js

```javascript
import OpenAI from 'openai';

const client = new OpenAI({
  apiKey: 'none',
  baseURL: 'http://localhost:8080/v1',
});

const response = await client.chat.completions.create({
  model: 'my-docs-bot',
  messages: [{ role: 'user', content: 'How does authentication work?' }],
});

console.log(response.choices[0].message.content);
```

### LangChain (Python)

```python
from langchain_openai import ChatOpenAI

llm = ChatOpenAI(
    model="my-docs-bot",
    openai_api_key="none",
    openai_api_base="http://localhost:8080/v1"
)

response = llm.invoke("How does authentication work?")
print(response.content)
```

---

## 9. Base Behavioral Package (aipk-base)

For small models (1-3B parameters), it's recommended to create a base package with behavioral instructions and use it alongside your domain package.

### Creating the base package

```bash
# English persona (recommended, small models follow EN instructions better)
aipk init-base --lang en --output aipk-base.aipk

# Russian persona
aipk init-base --lang ru --output aipk-base.aipk

# Bilingual (if using 7B+ models)
aipk init-base --lang auto --output aipk-base.aipk
```

The package contains:
- Language rules: reply in the user's language
- Honesty rules: don't make up facts
- 6 skills for common situations
- 6 canonical claims for behavior auditing

### Usage

```bash
# Base only (a minimal, useful assistant)
aipk serve aipk-base.aipk --model qwen2.5:3b

# Base + domain package (recommended)
aipk serve aipk-base.aipk my-docs-bot/my-docs-bot.aipk \
  --model qwen2.5:3b \
  --embed-model nomic-embed-text
```

---

## 10. Working with Multiple Packages

The server supports loading several packages at once. Knowledge is merged:

```bash
aipk serve \
  aipk-base.aipk \
  my-docs-bot/my-docs-bot.aipk \
  my-faq-bot/my-faq-bot.aipk \
  --model qwen2.5:7b \
  --embed-model nomic-embed-text \
  --port 8080
```

What happens on merge:
- **Persona**: all personas are joined with a `---` separator
- **Skills**: skills from all packages are merged into one pool
- **RAG**: search runs across all knowledge bases at once, results are globally re-ranked
- **Claims**: claims from all packages are merged

---

## 11. Strict Modes — strict-verify and strict-render

Strict modes guarantee that answers are based only on verified data.

### strict-verify

The LLM receives only canonical claims as context. If there's no data, it says so.

```bash
# Server
aipk serve my-docs-bot.aipk \
  --strict-verify \
  --model qwen2.5:3b

# One-shot query
aipk run my-docs-bot.aipk "What is a Bearer token?" \
  --strict-verify \
  --model qwen2.5:3b
```

How it works:
1. Keywords are extracted from the user's request
2. Relevant canonical claims are found via Jaccard similarity
3. Claims are injected into the system prompt as constraints
4. The LLM answers within those constraints

### strict-render

The LLM must cite `[claim_id]` after every factual sentence. The response is analyzed post-hoc.

```bash
aipk serve my-docs-bot.aipk \
  --strict-render \
  --model qwen2.5:7b
```

The server's response contains an additional `_aipk` field:
```json
{
  "choices": [{"message": {"content": "The API uses Bearer tokens. [doc_0003]"}}],
  "_aipk": {
    "mode": "strict-render",
    "coverage": 1.0,
    "canonical_claims_used": 1,
    "canonical_claim_ids": ["doc_0003"],
    "uncited_sentences": 0,
    "invalid_claim_ids": [],
    "unsupported_sentences": [],
    "fully_grounded": true
  }
}
```

> **Important:** Strict modes work better with 7B+ models. Small models (3B) may not reliably follow citation instructions.

---

## 12. Integrating with External Applications

### Anatomy of `/v1/chat/completions`

The AIPK server is fully compatible with the OpenAI API. Supported request fields:
- `messages` — the conversation history (array of `{role, content}`)
- `model` — the package name (informational; the actual model used comes from `--model`)
- `temperature` — generation temperature (default 1.0)

The `stream: true` field is fully supported — the response arrives via SSE (`data: {...}\n\n`), compatible with the OpenAI streaming API.

### Endpoints

| Method | Path | Description |
|-------|------|----------|
| GET | `/v1/models` | List of loaded packages |
| POST | `/v1/chat/completions` | Generate a response |
| GET | `/health` | Server status and stats |

### MCP (Model Context Protocol)

AIPK can act as an MCP server to integrate with other agents:

```bash
aipk mcp my-docs-bot.aipk \
  --model qwen2.5:3b \
  --llm-url http://localhost:11434 \
  --embed-model nomic-embed-text
```

Protocol: JSON-RPC 2.0 over stdio. Provides a single tool, `ask(question)`.

Example configuration for Claude Desktop (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "my-docs-bot": {
      "command": "aipk",
      "args": [
        "mcp",
        "/path/to/my-docs-bot.aipk",
        "--model", "qwen2.5:3b",
        "--embed-model", "nomic-embed-text"
      ]
    }
  }
}
```

---

## 13. Command Reference

```
aipk init <name> [--dir <path>]
    Create the project structure.
    --dir: directory (default: creates <name>/ in the current directory)

aipk init-base [--lang en|ru|auto] [--output <path>]
    Create a ready-to-use behavioral package.
    --lang: en (recommended for 3B), auto (for 7B+)

aipk add-docs <files...> --dir <dir>
    Vectorize documents and add them to RAG.
    --embed-model: embedding model (default: nomic-embed-text)
    --chunk-size: chunk size in tokens (default: 400)

aipk add-skill <file> --dir <dir>
    Add a .md skill file to skills/

aipk add-tool <name> --command <cmd> [--args ...] --dir <dir>
    Add an MCP tool to tools.json

aipk extract-claims <files...> --dir <dir>
    Extract claims from documents via the LLM.
    -m, --model: LLM model (default: llama3.2)

aipk claims list --dir <dir> [--status <status>] [--json]
    List claims. Statuses: extracted | reviewed | canonical | deprecated

aipk claims stats --dir <dir>
    Claim statistics by status

aipk claims promote <id> --dir <dir> [--reviewer <name>] [--reason <text>]
    Promote a claim to canonical

aipk claims reject <id> --dir <dir> [--reviewer <name>] [--reason <text>]
    Reject a claim (deprecated)

aipk claims review --dir <dir> [--reviewer <name>]
    Interactive review: y=canonical n=reject s=skip q=quit

aipk claims promote-all --dir <dir> [--from extracted] [--reviewer <name>]
    Bulk-promote to canonical

aipk pipeline <files...> --dir <dir>
    Full pipeline: add-docs → extract-claims → [review] → build
    --auto-promote: automatically canonical, no review
    --review: interactive review before building
    --output: path to the .aipk file

aipk build [<dir>] [--output <path>]
    Build a .aipk from a project directory

aipk extract <package> [--output <dir>]
    Unpack a .aipk into a project directory

aipk info <package>
    Show the package contents

aipk run <package> <query>
    One-shot query against a package.
    --strict-verify: canonical claims only
    --strict-render: cite [claim_id] in the answer
    --verbose: show RAG results and tool calls

aipk serve <packages...> [--port 8080] [--host 0.0.0.0]
    OpenAI-compatible server.
    --strict-verify / --strict-render: strict modes

aipk verify <package> <answer> [--min-coverage 1.0] [--json]
    Check an answer against claims.
    Exit: 0=OK, 1=uncovered, 2=error

aipk mcp <package>
    MCP stdio server for agent-to-agent calls.
    --embed-model: model for RAG

aipk init-base --lang en|ru|auto --output <path>
    Create a base behavioral package
```

---

## 14. Recommended Models

### For generating responses

| Model | Size | Multilingual quality | EN quality | Speed | Recommendation |
|--------|--------|------------|------------|---------|-------------|
| `qwen2.5:3b` | ~2 GB | ★★★☆ | ★★★☆ | Fast | Best starting point |
| `qwen2.5:7b` | ~4.7 GB | ★★★★ | ★★★★ | Medium | Production |
| `llama3.1:8b` | ~5 GB | ★★★☆ | ★★★★ | Medium | Good for EN |
| `mistral:7b` | ~4.1 GB | ★★☆☆ | ★★★★ | Medium | EN-only projects |
| `llama3.2:3b` | ~2 GB | ★★☆☆ | ★★★☆ | Fast | Not recommended (weak multilingual) |

### For embeddings (RAG)

| Model | Size | Quality | Note |
|--------|--------|---------|-----------|
| `nomic-embed-text` | ~270 MB | ★★★★ | Recommended |
| `mxbai-embed-large` | ~670 MB | ★★★★ | Best quality |

### For claim extraction

The same model you use for generation. `qwen2.5:3b` works well.

---

## 15. Common Issues

### Ollama server not responding

```bash
# Check that Ollama is running
curl http://localhost:11434/api/tags

# Restart it
ollama serve
```

### Responses come back in only one language

If the model answers only in one language regardless of the question, use the `aipk-base` package alongside your domain package:
```bash
aipk init-base --lang en
aipk serve aipk-base.aipk your-domain.aipk --model qwen2.5:3b
```

Small models (3B) often don't follow language-switching instructions well. `qwen2.5:3b` or larger is recommended.

### RAG doesn't find an answer

1. Check that documents were added: `aipk info your-pkg.aipk` should show a KNOW section
2. Make sure `--embed-model` is specified when starting the server
3. Try `aipk run ... --verbose` — it shows how many chunks were found

### extract-claims returns 0 claims

- Try a different model: `qwen2.5:3b` is more reliable than `llama3.2:3b` for this task
- Check that the documents contain factual statements (not just questions or headings)
- Reduce chunk size: `--chunk-size 200`

### strict-verify doesn't work (the model still hallucinates)

- Strict modes require 7B+ models
- Check that the package has canonical claims: `aipk claims list --status canonical`
- Try `qwen2.5:7b`

### "no META section" error on aipk run

The package is empty or corrupted. Rebuild it:
```bash
aipk build your-project/
```

### cargo build fails to compile

```bash
# Update Rust
rustup update stable

# Clear the build cache
cargo clean
cargo build --release
```
