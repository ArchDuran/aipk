use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

use crate::llm::embed_query;

/// Full pipeline: add-docs → extract-claims → [review|promote-all] → embed-claims → build
#[allow(clippy::too_many_arguments)]
pub async fn run(
    files: &[PathBuf],
    dir: &Path,
    model: &str,
    embed_model: &str,
    llm_url: &str,
    api_key: &str,
    chunk_size: usize,
    output: Option<&Path>,
    auto_promote: bool,
    interactive: bool,
    reviewer: Option<&str>,
    digest: bool,
    source_map: Option<&Path>,
) -> Result<()> {
    eprintln!("━━━ Pipeline: {} file(s) → .aipk ━━━", files.len());

    // Step 1: embed documents
    eprintln!("\n[1/5] Embedding documents...");
    crate::cmd::add_docs::run(dir, files, embed_model, llm_url, api_key, chunk_size).await?;

    // Step 1.5: persist embed_model + dim into manifest.toml
    let state_path = dir.join(".aipk/state.json");
    if state_path.exists() {
        let state: serde_json::Value = serde_json::from_str(&fs::read_to_string(&state_path)?)?;
        let dim = state["embedding_dim"].as_u64().unwrap_or(0) as u32;
        let manifest_path = dir.join("manifest.toml");
        if manifest_path.exists() {
            update_manifest_embedding(&manifest_path, embed_model, dim)?;
        }
    }

    // Step 2: extract claims
    eprintln!("\n[2/5] Extracting claims...");
    crate::cmd::extract_claims::run(
        dir, files, model, llm_url, api_key, chunk_size, digest, source_map,
    )
    .await?;

    // Step 3: review / promote
    if interactive {
        eprintln!("\n[3/5] Interactive review...");
        crate::cmd::claims::review(dir, reviewer)?;
    } else if auto_promote {
        eprintln!("\n[3/5] Auto-promoting all extracted claims...");
        crate::cmd::claims::promote_all(dir, "extracted", reviewer)?;
    } else {
        eprintln!("\n[3/5] Skipping review (use --review or --auto-promote to change).");
        eprintln!(
            "      Run 'aipk claims review --dir {}' later.",
            dir.display()
        );
    }

    // Step 4: embed canonical claims for semantic matching
    eprintln!("\n[4/5] Embedding claims for semantic matching...");
    embed_canonical_claims(dir, embed_model, llm_url, api_key).await?;

    // Step 5: build
    eprintln!("\n[5/5] Building package...");
    crate::cmd::build::run(dir, output)?;

    eprintln!("\n✓ Pipeline complete.");
    Ok(())
}

/// Write embedding_model and embedding_dim into manifest.toml's [model] section.
fn update_manifest_embedding(manifest_path: &Path, embed_model: &str, dim: u32) -> Result<()> {
    let content = fs::read_to_string(manifest_path)?;
    let updated: String = content
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("embedding_model") && trimmed.contains('=') {
                format!("embedding_model = \"{embed_model}\"")
            } else if trimmed.starts_with("embedding_dim") && trimmed.contains('=') {
                format!("embedding_dim   = {dim}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let updated = if content.ends_with('\n') {
        updated + "\n"
    } else {
        updated
    };
    fs::write(manifest_path, updated)?;
    Ok(())
}

/// Embed every claim in claims.jsonl (in order) and write raw f32 LE vectors
/// to .aipk/claim_vectors.bin. Dim and count go into state.json.
pub async fn embed_canonical_claims(
    dir: &Path,
    embed_model: &str,
    llm_url: &str,
    api_key: &str,
) -> Result<()> {
    let claims_file = dir.join("claims.jsonl");
    if !claims_file.exists() {
        eprintln!("  (no claims.jsonl — skipping)");
        return Ok(());
    }

    let content = fs::read_to_string(&claims_file)?;
    let claim_texts: Vec<String> = content
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            v["text"].as_str().map(|s| s.to_string())
        })
        .collect();

    if claim_texts.is_empty() {
        eprintln!("  (no claims to embed)");
        return Ok(());
    }

    let cache_dir = dir.join(".aipk");
    fs::create_dir_all(&cache_dir)?;

    let vectors_path = cache_dir.join("claim_vectors.bin");
    let mut all_bytes: Vec<u8> = Vec::new();
    let mut dim: u32 = 0;

    for (i, text) in claim_texts.iter().enumerate() {
        let vec = embed_query(llm_url, embed_model, text, api_key)
            .await
            .map_err(|e| anyhow::anyhow!("embedding claim {i}: {e}"))?;

        if dim == 0 {
            dim = vec.len() as u32;
        }
        for &v in &vec {
            all_bytes.extend_from_slice(&v.to_le_bytes());
        }
    }

    fs::write(&vectors_path, &all_bytes)?;

    // Persist count + dim so build.rs knows the layout
    let state_path = cache_dir.join("state.json");
    let mut state: serde_json::Map<String, serde_json::Value> = if state_path.exists() {
        serde_json::from_str(&fs::read_to_string(&state_path)?)?
    } else {
        serde_json::Map::new()
    };
    state.insert("claim_count".into(), serde_json::json!(claim_texts.len()));
    state.insert("claim_dim".into(), serde_json::json!(dim));
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&serde_json::Value::Object(state))?,
    )?;

    eprintln!("  ✓ {} claim vectors (dim={})", claim_texts.len(), dim);
    Ok(())
}
