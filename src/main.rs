mod agent;
mod ann;
mod cmd;
mod crypto;
mod format;
mod llm;
mod mcp_client;
mod parsers;
mod runtime;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "aipk",
    about = "AIPK — AI Package format tools",
    long_about = "AIPK — AI Package format tools\n\nThis shows the core build → review → serve loop. Run `aipk <command> --help` on any command \
                  for details, including ones not listed here (hub, gdpr, keygen, sign, seal, encrypt, claims, \
                  pipeline, and more) — see the README's CLI Reference for the full list.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check the environment: inference backend reachable, models available, current directory writable
    Doctor {
        /// Skip auto-detection and check this backend only
        #[arg(long)]
        llm_url: Option<String>,
    },

    /// Inspect contents of an .aipk package
    Info { path: PathBuf },

    /// Create a new AIPK project
    Init {
        name: String,
        /// Project directory. Omit to create a named subdirectory; pass "." to init in current dir.
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Create a ready-to-use base behavioral .aipk (optimized for small models)
    #[command(hide = true)]
    InitBase {
        /// Primary language for persona and skills: ru | en | auto (bilingual)
        #[arg(short, long, default_value = "en")]
        lang: String,
        /// Output path
        #[arg(short, long, default_value = "aipk-base.aipk")]
        output: PathBuf,
        /// Run 5 self-test queries after building (requires ollama)
        #[arg(long)]
        test: bool,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(short, long, default_value = "llama3.2")]
        model: String,
    },

    /// Check persona for prompt injection patterns and common issues
    Lint {
        /// Project directory or .aipk file (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Add documents to the knowledge base (requires ollama, or any
    /// OpenAI-compatible embedding endpoint via --llm-url/--llm-api-key)
    AddDocs {
        files: Vec<PathBuf>,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value_t = 400)]
        chunk_size: usize,
    },

    /// Import a git repository (URL or local path) as knowledge base
    AddGit {
        /// Git URL (https://, git@) or local path
        source: String,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value_t = 400)]
        chunk_size: usize,
        /// Extra file extensions to include (comma-separated, e.g. ".toml,.cfg")
        #[arg(long, value_delimiter = ',', default_value = "")]
        ext: Vec<String>,
        /// Also include source code files (.rs, .py, .js, .ts, .go, etc.)
        #[arg(long)]
        include_code: bool,
        /// Branch to clone (default: HEAD)
        #[arg(long)]
        branch: Option<String>,
        /// Clone depth (default: 1 for shallow clone)
        #[arg(long, default_value_t = 1)]
        depth: u32,
    },

    /// Import web pages or a sitemap as knowledge base
    AddWeb {
        /// URL to fetch, or sitemap URL (with --sitemap)
        source: String,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value_t = 400)]
        chunk_size: usize,
        /// Treat source as a sitemap XML and import all listed URLs
        #[arg(long)]
        sitemap: bool,
        /// Maximum number of pages to import (default: 50)
        #[arg(long, default_value_t = 50)]
        max_pages: usize,
        /// Delay between HTTP requests in milliseconds (default: 500)
        #[arg(long, default_value_t = 500)]
        delay_ms: u64,
    },

    /// Ingest a dataset (files + synced links) into the project knowledge base
    #[command(hide = true)]
    AddDataset {
        /// Dataset name (under ~/Datasets or $AIPK_DATASETS) or a direct path
        name: String,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        /// Embedding server URL (ollama or any OpenAI-compatible /v1/embeddings)
        #[arg(long, default_value = "http://localhost:11434")]
        embed_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value_t = 400)]
        chunk_size: usize,
    },

    /// Manage datasets: reusable knowledge sources shared across packages
    #[command(hide = true)]
    Dataset {
        #[command(subcommand)]
        action: DatasetAction,
    },

    /// Add a skill (.md file with frontmatter) to the project
    AddSkill {
        file: PathBuf,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },

    /// Add an MCP tool server to tools.json
    AddTool {
        name: String,
        #[arg(long)]
        command: String,
        #[arg(long, num_args = 0.., value_delimiter = ' ')]
        args: Vec<String>,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },

    /// Extract atomic claims from documents using LLM (for epistemic mode)
    #[command(hide = true)]
    ExtractClaims {
        files: Vec<PathBuf>,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        #[arg(short, long, default_value = "llama3.2")]
        model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value_t = 400)]
        chunk_size: usize,
        /// Paraphrase mode: restate each fact in new wording instead of
        /// quoting the source verbatim, and skip the literal-sentence
        /// fallback. Use for copyrighted/restrictively-licensed sources
        /// where the package must not reproduce excerpts of the original.
        #[arg(long)]
        digest: bool,
        /// JSON file mapping source filename to a citation string, e.g.
        /// {"SRE_Book.pdf": "Site Reliability Engineering, O'Reilly, 2016"}.
        /// Sources not listed fall back to the bare filename.
        #[arg(long)]
        source_map: Option<PathBuf>,
    },

    /// Manage claim lifecycle (list / promote / reject / review)
    #[command(hide = true)]
    Claims {
        #[command(subcommand)]
        action: ClaimsAction,
    },

    /// Full pipeline: add-docs + extract-claims + [review] + build
    #[command(hide = true)]
    Pipeline {
        /// Source documents to process
        files: Vec<PathBuf>,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        #[arg(short, long, default_value = "llama3.2")]
        model: String,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value_t = 400)]
        chunk_size: usize,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Auto-promote all extracted claims to canonical (skip review)
        #[arg(long)]
        auto_promote: bool,
        /// Interactively review claims before building (y/n/s/q)
        #[arg(long)]
        review: bool,
        /// Reviewer name written to audit trail
        #[arg(long)]
        reviewer: Option<String>,
        /// Paraphrase mode for claims (see 'aipk extract-claims --help')
        #[arg(long)]
        digest: bool,
        /// JSON file mapping source filename to a citation string
        #[arg(long)]
        source_map: Option<PathBuf>,
    },

    /// Export a package into a harness-native layout (Claude Code, OpenClaw, generic)
    #[command(hide = true)]
    Export {
        package: PathBuf,
        /// Target harness: claude-code | openclaw | generic
        #[arg(long, default_value = "generic")]
        to: String,
        /// Output directory (default: <package>-<target>/)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Passphrase for encrypted packages
        #[arg(long)]
        key: Option<String>,
    },

    /// Turnkey start: detect a running LLM backend, serve hub packages + chat UI
    #[command(hide = true)]
    Up {
        /// Packages to serve (default: everything installed in the hub)
        packages: Vec<PathBuf>,
        /// Generation model (default: first model reported by the backend)
        #[arg(short, long)]
        model: Option<String>,
        /// Backend URL (default: probe ollama/vLLM/LM Studio/llama.cpp ports)
        #[arg(long)]
        llm_url: Option<String>,
        /// Separate embedding URL (default: auto — ollama if available)
        #[arg(long)]
        embed_url: Option<String>,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
        /// Trust and launch MCP servers declared by these packages without an
        /// interactive prompt. They run arbitrary local commands with your full
        /// user permissions — only pass this for packages you built or otherwise trust.
        #[arg(long)]
        yes: bool,
    },

    /// Unpack .aipk contents into a project directory
    #[command(hide = true)]
    Extract {
        package: PathBuf,
        /// Output directory (default: package name without extension)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Build .aipk from project directory
    Build {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Run a one-shot query against a package
    Run {
        package: PathBuf,
        query: String,
        #[arg(short, long, default_value = "llama3.2")]
        model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        /// Separate URL for embeddings (default: same as --llm-url).
        /// Use when the generation backend (vLLM/SGLang) does not serve embeddings.
        #[arg(long)]
        embed_url: Option<String>,
        /// Sampling temperature. Low values give stable factual answers,
        /// high values add variety (and hallucinated embellishments).
        #[arg(short, long, default_value_t = 0.3)]
        temperature: f32,
        /// Cap on generated tokens. A safety net: without it, a model that
        /// never emits a stop token (small local models do this occasionally)
        /// will generate indefinitely instead of returning.
        #[arg(long, default_value_t = 1024)]
        max_tokens: u32,
        #[arg(short, long)]
        verbose: bool,
        /// Inject canonical claims as answer constraints
        #[arg(long)]
        strict_verify: bool,
        /// LLM must cite [claim_id] after every factual sentence
        #[arg(long)]
        strict_render: bool,
        /// Passphrase to decrypt an encrypted package at runtime
        #[arg(long)]
        key: Option<String>,
        /// Trust and launch this package's MCP servers without an interactive prompt.
        /// MCP servers run arbitrary local commands with your full user permissions —
        /// only pass this for packages you built or otherwise trust.
        #[arg(long)]
        yes: bool,
        /// Pre-approve a specific MCP server by name, skipping its prompt without
        /// trusting the whole package via --yes. Repeatable.
        #[arg(long)]
        allow: Vec<String>,
        /// Restrict tool calls to specific "server:tool" pairs, blocking everything
        /// else even on an approved server. Repeatable; omit for no restriction.
        #[arg(long)]
        allow_tool: Vec<String>,
        /// Post-generation enforcement for strict-render's groundedness check: observe
        /// (report only, default) | warn (flag but return) | block (withhold ungrounded
        /// answers). EXPERIMENTAL — gates on a single coverage signal, not a full
        /// grounding contract (no claim-freshness/entailment/contradiction checks).
        #[arg(long, default_value = "observe")]
        enforce: crate::runtime::EnforceMode,
    },

    /// Start an OpenAI-compatible server (supports multiple packages, streaming)
    Serve {
        /// One or more .aipk packages to serve (knowledge is merged)
        packages: Vec<PathBuf>,
        #[arg(short, long, default_value = "llama3.2")]
        model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        /// Separate URL for embeddings (default: same as --llm-url).
        /// Use when the generation backend (vLLM/SGLang) does not serve embeddings.
        #[arg(long)]
        embed_url: Option<String>,
        /// Force response language (e.g. ru, en, zh). Appended to every system prompt.
        #[arg(long)]
        lang: Option<String>,
        /// Strict-verify: inject canonical claims as answer constraints
        #[arg(long)]
        strict_verify: bool,
        /// Strict-render: LLM must cite [claim_id] after every factual sentence
        #[arg(long)]
        strict_render: bool,
        /// Graph mode: route each query to the best-matching package using THKG embeddings
        #[arg(long)]
        graph: bool,
        /// Cap on generated tokens (client requests may override per-call).
        /// Prevents a non-terminating generation from hanging a request forever.
        #[arg(long, default_value_t = 1024)]
        max_tokens: u32,
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        /// Passphrase to decrypt encrypted packages at runtime
        #[arg(long)]
        key: Option<String>,
        /// Trust and launch these packages' MCP servers without an interactive prompt.
        /// MCP servers run arbitrary local commands with your full user permissions —
        /// only pass this for packages you built or otherwise trust.
        #[arg(long)]
        yes: bool,
        /// Pre-approve a specific MCP server by name, skipping its prompt without
        /// trusting the whole package via --yes. Repeatable.
        #[arg(long)]
        allow: Vec<String>,
        /// Restrict tool calls to specific "server:tool" pairs, blocking everything
        /// else even on an approved server. Repeatable; omit for no restriction.
        #[arg(long)]
        allow_tool: Vec<String>,
        /// Post-generation enforcement for strict-render's groundedness check: observe
        /// (report only, default) | warn (flag but return) | block (withhold ungrounded
        /// answers). EXPERIMENTAL — gates on a single coverage signal, not a full
        /// grounding contract (no claim-freshness/entailment/contradiction checks).
        #[arg(long, default_value = "observe")]
        enforce: crate::runtime::EnforceMode,
    },

    /// Verify an answer against package claims (auditable provenance report)
    Verify {
        package: PathBuf,
        /// Answer text to verify
        answer: String,
        /// Minimum grounded sentence coverage required for exit code 0
        #[arg(long, default_value_t = 1.0)]
        min_coverage: f32,
        /// Also match against unreviewed claims (status extracted/reviewed), not just
        /// canonical ones. Off by default: "grounded" should mean a human signed off
        /// on the fact, not merely that the model extracted it.
        #[arg(long)]
        include_unreviewed: bool,
        /// Output as JSON (for tooling / CI)
        #[arg(long)]
        json: bool,
    },

    /// Scale benchmark: KNOW section build/load time, size, and both brute-force
    /// and HNSW (AnnIndex) retrieval latency/recall on synthetic vectors (no
    /// embedding backend required). Not a grounding-quality benchmark — see
    /// bench/results.md for that.
    #[command(hide = true)]
    BenchScale {
        /// Chunk counts to benchmark, e.g. --sizes 1000,10000,100000
        #[arg(long, value_delimiter = ',', default_value = "1000,10000,100000")]
        sizes: Vec<usize>,
        /// Random queries to sample per size for the latency percentiles
        #[arg(long, default_value_t = 200)]
        queries: usize,
        /// Output as JSON (for tooling / CI)
        #[arg(long)]
        json: bool,
    },

    /// Run agent as MCP stdio server (for agent-to-agent calls)
    #[command(hide = true)]
    Mcp {
        package: PathBuf,
        #[arg(short, long, default_value = "llama3.2")]
        model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        /// Trust and launch this package's MCP servers without an interactive prompt.
        /// MCP servers run arbitrary local commands with your full user permissions —
        /// only pass this for packages you built or otherwise trust.
        #[arg(long)]
        yes: bool,
        /// Pre-approve a specific MCP server by name, skipping its prompt without
        /// trusting the whole package via --yes. Repeatable.
        #[arg(long)]
        allow: Vec<String>,
        /// Restrict tool calls to specific "server:tool" pairs, blocking everything
        /// else even on an approved server. Repeatable; omit for no restriction.
        #[arg(long)]
        allow_tool: Vec<String>,
    },

    /// Local package hub — install, search, and serve .aipk packages
    #[command(hide = true)]
    Hub {
        #[command(subcommand)]
        action: HubAction,
    },

    /// Generate an Ed25519 keypair for package signing
    #[command(hide = true)]
    Keygen {
        /// Key name — saved as ~/.aipk/keys/<name>.pem and <name>.pub
        #[arg(long, default_value = "default")]
        name: String,
        /// Overwrite an existing key with the same name
        #[arg(long)]
        force: bool,
    },

    /// Sign a .aipk package with an Ed25519 private key
    #[command(hide = true)]
    Sign {
        /// Package file to sign
        package: PathBuf,
        /// Path to private key (.pem)
        #[arg(long, short)]
        key: PathBuf,
        /// Output path (defaults to overwriting the input file)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Seal a package: encrypt contents + sign. Opaque at rest, immutable
    /// without the author's key; serve/run keep working, extract is blocked.
    #[command(hide = true)]
    Seal {
        /// Package file to seal
        package: PathBuf,
        /// Author's Ed25519 private key (.pem) — required to unseal later
        #[arg(long, short)]
        key: PathBuf,
        /// Output path (defaults to overwriting the input file)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Unseal a package back into an editable one (author's key required)
    #[command(hide = true)]
    Unseal {
        /// Sealed package file
        package: PathBuf,
        /// The same private key (.pem) that sealed the package
        #[arg(long, short)]
        key: PathBuf,
        /// Output path (defaults to overwriting the input file)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Run package tests: static checks + optional live LLM queries
    #[command(hide = true)]
    Test {
        /// Package file to test
        package: PathBuf,
        /// LLM model for live tests (if omitted, only static checks run)
        #[arg(long, short)]
        model: Option<String>,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        /// Embedding model for RAG during live tests
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
    },

    /// Verify the Ed25519 signature on a .aipk package
    #[command(hide = true)]
    VerifySig {
        /// Package file to verify
        package: PathBuf,
        /// Expected public key (.pub) — if omitted, verifies mathematical validity only
        #[arg(long)]
        key: Option<PathBuf>,
    },

    /// Create Epistemic v2 template files (IDTY/ANSP/PLCY/NKNW) in a project directory
    #[command(hide = true)]
    V2Init {
        /// Project directory (defaults to current directory)
        #[arg(default_value = ".")]
        dir: PathBuf,
    },

    /// Encrypt content sections of a .aipk package (AES-256-GCM)
    #[command(hide = true)]
    Encrypt {
        package: PathBuf,
        /// Passphrase (derives AES-256 key via SHA-256)
        #[arg(long)]
        key: String,
        /// Comma-separated section tags to encrypt (default: all content sections)
        #[arg(long, value_delimiter = ',')]
        sections: Vec<String>,
        /// Output path (default: overwrite input)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Decrypt a previously encrypted .aipk package
    #[command(hide = true)]
    Decrypt {
        package: PathBuf,
        /// Passphrase used during encryption
        #[arg(long)]
        key: String,
        /// Output path (default: overwrite input)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// GDPR tooling: list sources, erase by source, compliance report
    #[command(hide = true)]
    Gdpr {
        #[command(subcommand)]
        action: GdprAction,
    },
}

#[derive(Subcommand)]
enum DatasetAction {
    /// Create a new dataset directory with a sources.toml template
    Init { name: String },
    /// List datasets under ~/Datasets (or $AIPK_DATASETS)
    List,
    /// Download web/git links from sources.toml into the dataset cache
    Sync {
        /// Dataset name or direct path
        name: String,
    },
}

#[derive(Subcommand)]
enum GdprAction {
    /// List all source documents embedded in the package
    ListSources {
        package: PathBuf,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Remove all data from a specific source, or from every source
    /// belonging to one person (KNOW, CLMS, SRCS)
    Erase {
        package: PathBuf,
        /// Source name to erase
        #[arg(long)]
        source: Option<String>,
        /// Subject (person) to erase — requires --subject-map. Covers every
        /// source file mapped to this subject in one call.
        #[arg(long)]
        subject: Option<String>,
        /// JSON file mapping source filename to a subject id
        #[arg(long)]
        subject_map: Option<PathBuf>,
        /// Output path (default: overwrite input)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Show what would be removed without writing
        #[arg(long)]
        dry_run: bool,
    },
    /// Export everything held about a source or subject — every knowledge
    /// chunk, claim, and source record — without modifying the package.
    /// The right-of-access / data-portability counterpart to `erase`.
    Snapshot {
        package: PathBuf,
        /// Source name to export
        #[arg(long)]
        source: Option<String>,
        /// Subject (person) to export — requires --subject-map. Covers
        /// every source file mapped to this subject in one call.
        #[arg(long)]
        subject: Option<String>,
        /// JSON file mapping source filename to a subject id
        #[arg(long)]
        subject_map: Option<PathBuf>,
        /// Write the snapshot to a file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Generate a GDPR compliance report for the package
    Report { package: PathBuf },
}

#[derive(Subcommand)]
enum HubAction {
    /// List all installed packages
    List {
        #[arg(long)]
        json: bool,
    },
    /// Search packages by name, description or tag
    Search {
        query: String,
        #[arg(long)]
        json: bool,
    },
    /// Install a package from a local path or URL
    Install {
        /// File path or https:// URL
        source: String,
        /// Override the package name
        #[arg(long)]
        name: Option<String>,
    },
    /// Add a local package to the hub (marks it as yours)
    Publish {
        path: PathBuf,
        /// Mark as publicly shareable (for future remote hub)
        #[arg(long)]
        public: bool,
    },
    /// Re-download URL-sourced packages
    Update {
        /// Update a specific package (default: all URL-sourced)
        name: Option<String>,
    },
    /// Show detailed info about an installed package
    Info {
        /// Package name, optionally with version: name@version
        name: String,
    },
    /// Remove a package from the hub
    Remove {
        /// Package name, optionally with version: name@version
        name: String,
    },
    /// Serve all (or specific) hub packages via OpenAI-compatible API
    Serve {
        /// Package names to serve (default: all installed)
        names: Vec<String>,
        #[arg(short, long, default_value = "llama3.2")]
        model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        /// Separate URL for embeddings (default: same as --llm-url)
        #[arg(long)]
        embed_url: Option<String>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        strict_verify: bool,
        #[arg(long)]
        strict_render: bool,
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        /// Trust and launch these packages' MCP servers without an interactive prompt.
        /// MCP servers run arbitrary local commands with your full user permissions —
        /// only pass this for packages you built or otherwise trust.
        #[arg(long)]
        yes: bool,
        /// Pre-approve a specific MCP server by name, skipping its prompt without
        /// trusting the whole package via --yes. Repeatable.
        #[arg(long)]
        allow: Vec<String>,
        /// Restrict tool calls to specific "server:tool" pairs, blocking everything
        /// else even on an approved server. Repeatable; omit for no restriction.
        #[arg(long)]
        allow_tool: Vec<String>,
        /// Post-generation enforcement for strict-render's groundedness check: observe
        /// (report only, default) | warn (flag but return) | block (withhold ungrounded
        /// answers). EXPERIMENTAL — gates on a single coverage signal, not a full
        /// grounding contract (no claim-freshness/entailment/contradiction checks).
        #[arg(long, default_value = "observe")]
        enforce: crate::runtime::EnforceMode,
    },
    /// Show hub directory, package count, and disk usage
    Status,
}

#[derive(Subcommand)]
enum ClaimsAction {
    /// List claims, optionally filtered by status
    List {
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        /// Filter by status: extracted | reviewed | canonical | deprecated
        #[arg(long)]
        status: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show claim counts per status
    Stats {
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    /// Promote a claim to 'canonical' (or 'reviewed' with --status reviewed)
    Promote {
        /// Claim ID to promote
        id: String,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        /// Reviewer name for audit trail
        #[arg(long)]
        reviewer: Option<String>,
        /// Reason for promotion
        #[arg(long)]
        reason: Option<String>,
        /// Target status (default: canonical)
        #[arg(long, default_value = "canonical")]
        status: String,
    },
    /// Reject a claim (marks it deprecated with audit entry)
    Reject {
        /// Claim ID to reject
        id: String,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        /// Reviewer name for audit trail
        #[arg(long)]
        reviewer: Option<String>,
        /// Reason for rejection
        #[arg(long)]
        reason: Option<String>,
    },
    /// Interactively review pending claims (y=canonical n=reject s=skip q=quit)
    Review {
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        /// Reviewer name written to audit trail
        #[arg(long)]
        reviewer: Option<String>,
    },
    /// Promote all claims of a given status to canonical at once
    PromoteAll {
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        /// Source status to promote from (default: extracted)
        #[arg(long, default_value = "extracted")]
        from: String,
        /// Reviewer name written to audit trail
        #[arg(long)]
        reviewer: Option<String>,
    },
    /// Embed canonical claims into vectors (CLMV) for semantic matching.
    /// Without this, `verify` and strict modes fall back to lexical matching.
    /// (`aipk pipeline` runs it automatically; after a manual
    /// add-docs/extract-claims/build chain run it before `build`.)
    Embed {
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        #[arg(long, default_value = "nomic-embed-text")]
        embed_model: String,
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        #[arg(long, default_value = "")]
        llm_api_key: String,
    },
    /// Check for contradictions between canonical claims using LLM as NLI judge
    CheckConflicts {
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        /// LLM URL
        #[arg(long, default_value = "http://localhost:11434")]
        llm_url: String,
        /// LLM model for NLI judgements
        #[arg(long, default_value = "llama3.2")]
        model: String,
        /// API key for LLM (if required)
        #[arg(long, default_value = "")]
        llm_api_key: String,
        /// Maximum number of claim pairs to check
        #[arg(long, default_value = "100")]
        max_pairs: usize,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Doctor { llm_url } => cmd::doctor::run(llm_url).await,

        Commands::Info { path } => cmd::info::run(&path),

        Commands::Init { name, dir } => cmd::init::run(&name, dir.as_deref()),

        Commands::InitBase {
            lang,
            output,
            test,
            llm_url,
            model,
        } => cmd::init_base::run(&lang, &output, test, &llm_url, &model).await,

        Commands::Lint { path } => cmd::lint::run(&path),

        Commands::AddDocs {
            files,
            dir,
            embed_model,
            llm_url,
            llm_api_key,
            chunk_size,
        } => {
            cmd::add_docs::run(
                &dir,
                &files,
                &embed_model,
                &llm_url,
                &llm_api_key,
                chunk_size,
            )
            .await
        }

        Commands::AddGit {
            source,
            dir,
            embed_model,
            llm_url,
            llm_api_key,
            chunk_size,
            ext,
            include_code,
            branch,
            depth,
        } => {
            let exts: Vec<String> = ext.into_iter().filter(|e| !e.is_empty()).collect();
            cmd::add_git::run(
                &dir,
                &source,
                &embed_model,
                &llm_url,
                &llm_api_key,
                chunk_size,
                &exts,
                include_code,
                branch.as_deref(),
                depth,
            )
            .await
        }

        Commands::AddWeb {
            source,
            dir,
            embed_model,
            llm_url,
            llm_api_key,
            chunk_size,
            sitemap,
            max_pages,
            delay_ms,
        } => {
            cmd::add_web::run(
                &dir,
                &source,
                &embed_model,
                &llm_url,
                &llm_api_key,
                chunk_size,
                sitemap,
                max_pages,
                delay_ms,
            )
            .await
        }

        Commands::AddDataset {
            name,
            dir,
            embed_model,
            embed_url,
            llm_api_key,
            chunk_size,
        } => {
            cmd::dataset::add(
                &name,
                &dir,
                &embed_model,
                &embed_url,
                &llm_api_key,
                chunk_size,
            )
            .await
        }

        Commands::Dataset { action } => match action {
            DatasetAction::Init { name } => cmd::dataset::init(&name),
            DatasetAction::List => cmd::dataset::list(),
            DatasetAction::Sync { name } => cmd::dataset::sync(&name).await,
        },

        Commands::AddSkill { file, dir } => cmd::add_skill::run(&dir, &file),

        Commands::AddTool {
            name,
            command,
            args,
            dir,
        } => cmd::add_tool::run(&dir, &name, &command, &args),

        Commands::ExtractClaims {
            files,
            dir,
            model,
            llm_url,
            llm_api_key,
            chunk_size,
            digest,
            source_map,
        } => {
            cmd::extract_claims::run(
                &dir,
                &files,
                &model,
                &llm_url,
                &llm_api_key,
                chunk_size,
                digest,
                source_map.as_deref(),
            )
            .await
        }

        Commands::Claims { action } => match action {
            ClaimsAction::List { dir, status, json } => {
                cmd::claims::list(&dir, status.as_deref(), json)
            }
            ClaimsAction::Stats { dir } => cmd::claims::stats(&dir),
            ClaimsAction::Promote {
                id,
                dir,
                reviewer,
                reason,
                status,
            } => cmd::claims::promote(&dir, &id, reviewer.as_deref(), reason.as_deref(), &status),
            ClaimsAction::Reject {
                id,
                dir,
                reviewer,
                reason,
            } => cmd::claims::reject(&dir, &id, reviewer.as_deref(), reason.as_deref()),
            ClaimsAction::Review { dir, reviewer } => {
                cmd::claims::review(&dir, reviewer.as_deref())
            }
            ClaimsAction::PromoteAll {
                dir,
                from,
                reviewer,
            } => cmd::claims::promote_all(&dir, &from, reviewer.as_deref()),
            ClaimsAction::Embed {
                dir,
                embed_model,
                llm_url,
                llm_api_key,
            } => {
                cmd::pipeline::embed_canonical_claims(&dir, &embed_model, &llm_url, &llm_api_key)
                    .await
            }
            ClaimsAction::CheckConflicts {
                dir,
                llm_url,
                model,
                llm_api_key,
                max_pairs,
            } => {
                cmd::claims::check_conflicts(&dir, &llm_url, &model, &llm_api_key, max_pairs).await
            }
        },

        Commands::Pipeline {
            files,
            dir,
            model,
            embed_model,
            llm_url,
            llm_api_key,
            chunk_size,
            output,
            auto_promote,
            review,
            reviewer,
            digest,
            source_map,
        } => {
            cmd::pipeline::run(
                &files,
                &dir,
                &model,
                &embed_model,
                &llm_url,
                &llm_api_key,
                chunk_size,
                output.as_deref(),
                auto_promote,
                review,
                reviewer.as_deref(),
                digest,
                source_map.as_deref(),
            )
            .await
        }

        Commands::Export {
            package,
            to,
            output,
            key,
        } => cmd::export::run(&package, &to, output.as_deref(), key.as_deref()),

        Commands::Up {
            packages,
            model,
            llm_url,
            embed_url,
            embed_model,
            port,
            yes,
        } => cmd::up::run(packages, model, llm_url, embed_url, embed_model, port, yes).await,

        Commands::Extract { package, output } => {
            let out_dir = output.unwrap_or_else(|| {
                let stem = package
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "extracted".to_string());
                std::path::PathBuf::from(stem)
            });
            cmd::extract::run(&package, &out_dir)
        }

        Commands::Build { dir, output } => cmd::build::run(&dir, output.as_deref()),

        Commands::Run {
            package,
            query,
            model,
            llm_url,
            llm_api_key,
            embed_model,
            embed_url,
            temperature,
            max_tokens,
            verbose,
            strict_verify,
            strict_render,
            key,
            yes,
            allow,
            allow_tool,
            enforce,
        } => {
            cmd::run::run(
                &package,
                &query,
                &llm_url,
                embed_url.as_deref(),
                &model,
                &llm_api_key,
                &embed_model,
                temperature,
                max_tokens,
                verbose,
                strict_verify,
                strict_render,
                key.as_deref(),
                yes,
                &allow,
                &allow_tool,
                enforce,
            )
            .await
        }

        Commands::Serve {
            packages,
            model,
            llm_url,
            llm_api_key,
            embed_model,
            embed_url,
            lang,
            strict_verify,
            strict_render,
            graph,
            max_tokens,
            port,
            host,
            key,
            yes,
            allow,
            allow_tool,
            enforce,
        } => {
            if packages.is_empty() {
                eprintln!("Error: at least one package required");
                std::process::exit(1);
            }
            cmd::serve::run(
                packages,
                model,
                llm_url,
                embed_url,
                llm_api_key,
                embed_model,
                lang,
                strict_verify,
                strict_render,
                enforce,
                graph,
                max_tokens,
                &host,
                port,
                key.as_deref(),
                yes,
                &allow,
                &allow_tool,
            )
            .await
        }

        Commands::Verify {
            package,
            answer,
            min_coverage,
            include_unreviewed,
            json,
        } => {
            if let Err(e) =
                cmd::verify::run(&package, &answer, min_coverage, include_unreviewed, json)
            {
                eprintln!("Error: {e}");
                std::process::exit(2);
            }
            return;
        }

        Commands::BenchScale {
            sizes,
            queries,
            json,
        } => {
            if let Err(e) = cmd::bench::run(sizes, queries, json) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            return;
        }

        Commands::Mcp {
            package,
            model,
            llm_url,
            llm_api_key,
            embed_model,
            yes,
            allow,
            allow_tool,
        } => {
            cmd::mcp::run(
                &package,
                &llm_url,
                &model,
                &llm_api_key,
                &embed_model,
                yes,
                &allow,
                &allow_tool,
            )
            .await
        }

        Commands::Hub { action } => match action {
            HubAction::List { json } => cmd::hub::list(json),
            HubAction::Search { query, json } => cmd::hub::search(&query, json),
            HubAction::Install { source, name } => {
                cmd::hub::install(&source, name.as_deref()).await
            }
            HubAction::Publish { path, public } => cmd::hub::publish(&path, public),
            HubAction::Update { name } => cmd::hub::update(name.as_deref()).await,
            HubAction::Info { name } => cmd::hub::info(&name),
            HubAction::Remove { name } => cmd::hub::remove(&name),
            HubAction::Serve {
                names,
                model,
                llm_url,
                llm_api_key,
                embed_model,
                embed_url,
                lang,
                strict_verify,
                strict_render,
                port,
                host,
                yes,
                allow,
                allow_tool,
                enforce,
            } => {
                cmd::hub::serve(
                    names,
                    model,
                    llm_url,
                    embed_url,
                    llm_api_key,
                    embed_model,
                    lang,
                    strict_verify,
                    strict_render,
                    &host,
                    port,
                    yes,
                    &allow,
                    &allow_tool,
                    enforce,
                )
                .await
            }
            HubAction::Status => cmd::hub::status(),
        },

        Commands::Test {
            package,
            model,
            llm_url,
            llm_api_key,
            embed_model,
        } => {
            cmd::test::run(
                &package,
                model.as_deref(),
                &llm_url,
                &llm_api_key,
                &embed_model,
            )
            .await
        }

        Commands::Seal {
            package,
            key,
            output,
        } => cmd::seal::seal(&package, &key, output.as_deref()),

        Commands::Unseal {
            package,
            key,
            output,
        } => cmd::seal::unseal(&package, &key, output.as_deref()),

        Commands::Keygen { name, force } => cmd::sign::keygen(&name, force),

        Commands::Sign {
            package,
            key,
            output,
        } => cmd::sign::sign(&package, &key, output.as_deref()),

        Commands::VerifySig { package, key } => cmd::sign::verify_sig(&package, key.as_deref()),

        Commands::V2Init { dir } => cmd::v2_init::run(&dir),

        Commands::Encrypt {
            package,
            key,
            sections,
            output,
        } => cmd::encrypt::encrypt(&package, &key, &sections, output.as_deref()),

        Commands::Decrypt {
            package,
            key,
            output,
        } => cmd::encrypt::decrypt(&package, &key, output.as_deref()),

        Commands::Gdpr { action } => match action {
            GdprAction::ListSources { package, json } => cmd::gdpr::list_sources(&package, json),
            GdprAction::Erase {
                package,
                source,
                subject,
                subject_map,
                output,
                dry_run,
            } => {
                let sources = match cmd::gdpr::resolve_sources(
                    source.as_deref(),
                    subject.as_deref(),
                    subject_map.as_deref(),
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(2);
                    }
                };
                cmd::gdpr::erase(&package, &sources, output.as_deref(), dry_run)
            }
            GdprAction::Snapshot {
                package,
                source,
                subject,
                subject_map,
                output,
            } => {
                let sources = match cmd::gdpr::resolve_sources(
                    source.as_deref(),
                    subject.as_deref(),
                    subject_map.as_deref(),
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(2);
                    }
                };
                cmd::gdpr::snapshot(&package, &sources, output.as_deref())
            }
            GdprAction::Report { package } => cmd::gdpr::report(&package),
        },
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_without_dir_uses_name_as_subdir() {
        let cli = Cli::parse_from(["aipk", "init", "my-pkg"]);
        match cli.command {
            Commands::Init { name, dir } => {
                assert_eq!(name, "my-pkg");
                assert!(dir.is_none(), "no --dir should leave dir as None");
            }
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn init_with_dot_dir_uses_current_directory() {
        let cli = Cli::parse_from(["aipk", "init", "my-pkg", "--dir", "."]);
        match cli.command {
            Commands::Init { name, dir } => {
                assert_eq!(name, "my-pkg");
                assert_eq!(dir, Some(PathBuf::from(".")));
            }
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn init_base_defaults_to_english() {
        let cli = Cli::parse_from(["aipk", "init-base"]);
        match cli.command {
            Commands::InitBase {
                lang, output, test, ..
            } => {
                assert_eq!(lang, "en");
                assert_eq!(output, PathBuf::from("aipk-base.aipk"));
                assert!(!test);
            }
            _ => panic!("expected init-base command"),
        }
    }

    #[test]
    fn verify_defaults_to_full_coverage() {
        let cli = Cli::parse_from(["aipk", "verify", "pkg.aipk", "Grounded answer."]);
        match cli.command {
            Commands::Verify {
                min_coverage, json, ..
            } => {
                assert_eq!(min_coverage, 1.0);
                assert!(!json);
            }
            _ => panic!("expected verify command"),
        }
    }

    #[test]
    fn enforce_defaults_to_observe_and_parses_all_variants() {
        let cli = Cli::parse_from(["aipk", "run", "pkg.aipk", "question"]);
        match cli.command {
            Commands::Run { enforce, .. } => {
                assert_eq!(enforce, crate::runtime::EnforceMode::Observe);
            }
            _ => panic!("expected run command"),
        }

        for (flag, expected) in [
            ("observe", crate::runtime::EnforceMode::Observe),
            ("warn", crate::runtime::EnforceMode::Warn),
            ("block", crate::runtime::EnforceMode::Block),
        ] {
            let cli = Cli::parse_from(["aipk", "run", "pkg.aipk", "question", "--enforce", flag]);
            match cli.command {
                Commands::Run { enforce, .. } => assert_eq!(enforce, expected, "flag={flag}"),
                _ => panic!("expected run command"),
            }
        }
    }
}
