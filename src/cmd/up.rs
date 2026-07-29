//! `aipk up` — turnkey start: detect a running inference backend, load the
//! hub packages (or explicit .aipk files), and serve them with the built-in
//! chat page. The goal is "one command after download".

use anyhow::{bail, Result};
use std::path::PathBuf;

/// Candidate backends probed in order: (base_url, human name).
pub(crate) const CANDIDATES: &[(&str, &str)] = &[
    ("http://localhost:11434", "ollama"),
    ("http://localhost:8000", "vLLM/SGLang"),
    ("http://localhost:1234", "LM Studio"),
    ("http://localhost:8081", "llama.cpp"),
];

pub async fn run(
    packages: Vec<PathBuf>,
    model: Option<String>,
    llm_url: Option<String>,
    embed_url: Option<String>,
    embed_model: String,
    port: u16,
    trust_tools: bool,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    // ── 1. Find a generation backend ────────────────────────────────────────
    let (backend_url, backend_name, models) = match llm_url {
        Some(url) => {
            let models = list_models(&client, &url).await.unwrap_or_default();
            (url, "custom".to_string(), models)
        }
        None => {
            let mut found = None;
            for (url, name) in CANDIDATES {
                if let Ok(models) = list_models(&client, url).await {
                    eprintln!("✓ Found {} at {}", name, url);
                    found = Some((url.to_string(), name.to_string(), models));
                    break;
                }
            }
            match found {
                Some(f) => f,
                None => bail!(
                    "No inference backend found. Probed: {}.\n\
                     Start one (e.g. `ollama serve`, `vllm serve <model>`) or pass --llm-url.",
                    CANDIDATES
                        .iter()
                        .map(|(u, n)| format!("{n} ({u})"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
    };

    // ── 2. Pick the generation model ────────────────────────────────────────
    let model = match model {
        Some(m) => m,
        None => match models.iter().find(|m| !looks_like_embedder(m)) {
            Some(m) => {
                eprintln!(
                    "✓ Using model: {} (first non-embedding model reported by backend)",
                    m
                );
                m.clone()
            }
            None => bail!(
                "Backend at {} reports no usable models. Pull one (e.g. `ollama pull llama3.2`) \
                 or pass --model.",
                backend_url
            ),
        },
    };

    // ── 3. Embeddings: prefer ollama when the backend is not ollama ─────────
    let embed_url = match embed_url {
        Some(u) => Some(u),
        None if backend_name == "ollama" => None, // same URL
        None => {
            if list_models(&client, "http://localhost:11434").await.is_ok() {
                eprintln!("✓ Using ollama at :11434 for embeddings");
                Some("http://localhost:11434".to_string())
            } else {
                eprintln!(
                    "! No ollama found for embeddings — trying {}/v1/embeddings; \
                     RAG is disabled if the backend can't embed",
                    backend_url
                );
                None
            }
        }
    };

    // ── 4. Packages: explicit paths or the whole hub ─────────────────────────
    let package_paths = if packages.is_empty() {
        let hub = crate::cmd::hub::all_package_paths()?;
        if hub.is_empty() {
            bail!(
                "No packages given and the hub is empty.\n\
                 aipk up <pkg.aipk ...>   or   aipk hub install <path|url>"
            );
        }
        eprintln!("✓ Serving {} hub package(s)", hub.len());
        hub
    } else {
        packages
    };

    eprintln!();
    eprintln!("Chat UI:  http://localhost:{port}/");
    eprintln!("API:      http://localhost:{port}/v1/  (OpenAI-compatible; model = package name)");
    eprintln!();

    crate::cmd::serve::run(
        package_paths,
        model,
        backend_url,
        embed_url,
        String::new(),
        embed_model,
        None,
        false,
        false,
        crate::runtime::EnforceMode::Observe,
        false,
        crate::agent::DEFAULT_MAX_TOKENS,
        "0.0.0.0",
        port,
        None,
        trust_tools,
        &[],
        &[],
    )
    .await
}

/// GET /v1/models — works for ollama, vLLM, SGLang, LM Studio, llama.cpp.
pub(crate) async fn list_models(client: &reqwest::Client, base_url: &str) -> Result<Vec<String>> {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        bail!("{} returned {}", url, resp.status());
    }
    let v: serde_json::Value = resp.json().await?;
    Ok(v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default())
}

fn looks_like_embedder(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("embed") || m.contains("bge") || m.contains("e5-") || m.starts_with("e5")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedder_names_are_filtered() {
        assert!(looks_like_embedder("nomic-embed-text"));
        assert!(looks_like_embedder("BAAI/bge-small-en-v1.5"));
        assert!(!looks_like_embedder("llama3.2"));
        assert!(!looks_like_embedder("Qwen/Qwen2.5-7B-Instruct"));
    }
}
