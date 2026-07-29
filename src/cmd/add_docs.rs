use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::llm::embed_query;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    project_dir: &Path,
    files: &[PathBuf],
    embed_model: &str,
    llm_url: &str,
    api_key: &str,
    chunk_size: usize,
) -> Result<()> {
    let cache_dir = project_dir.join(".aipk");
    fs::create_dir_all(&cache_dir)?;

    let state_file = cache_dir.join("state.json");
    let chunks_file = cache_dir.join("chunks.jsonl");
    let vectors_file = cache_dir.join("vectors.bin");

    let mut state: serde_json::Map<String, Value> = if state_file.exists() {
        serde_json::from_str(&fs::read_to_string(&state_file)?)?
    } else {
        serde_json::Map::new()
    };

    if let Some(existing) = state.get("embedding_model").and_then(|v| v.as_str()) {
        if existing != embed_model {
            anyhow::bail!(
                "Embedding model mismatch: project uses '{}', requested '{}'. \
                 Use a consistent model across add-docs calls.",
                existing,
                embed_model
            );
        }
    }

    let mut chunk_id = state
        .get("chunk_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let mut dim: Option<u32> = state
        .get("embedding_dim")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let chunks_f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&chunks_file)?;
    let mut chunks_w = BufWriter::new(chunks_f);

    let vectors_f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&vectors_file)?;
    let mut vectors_w = BufWriter::new(vectors_f);

    let mut added = 0usize;

    for file_path in files {
        let text = read_text_file(file_path)
            .with_context(|| format!("reading {}", file_path.display()))?;
        let source = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let text_chunks = chunk_text(&text, chunk_size);
        eprintln!("  {} → {} chunks", source, text_chunks.len());

        for chunk_text in text_chunks {
            let vec = embed_query(llm_url, embed_model, &chunk_text, api_key)
                .await
                .with_context(|| format!("embedding chunk from {source}"))?;

            if dim.is_none() {
                dim = Some(vec.len() as u32);
            }

            let line = serde_json::to_string(&json!({
                "id": chunk_id,
                "text": chunk_text,
                "source": source,
                "meta": {},
            }))?;
            writeln!(chunks_w, "{}", line)?;

            for &v in &vec {
                vectors_w.write_all(&v.to_le_bytes())?;
            }

            chunk_id += 1;
            added += 1;
        }
    }

    chunks_w.flush()?;
    vectors_w.flush()?;

    state.insert("embedding_model".into(), json!(embed_model));
    state.insert("embedding_dim".into(), json!(dim.unwrap_or(0)));
    state.insert("chunk_count".into(), json!(chunk_id));
    fs::write(
        &state_file,
        serde_json::to_string_pretty(&Value::Object(state))?,
    )?;

    println!(
        "✓ Added {} chunks (total: {}, dim={})",
        added,
        chunk_id,
        dim.unwrap_or(0)
    );
    Ok(())
}

fn read_text_file(path: &Path) -> Result<String> {
    match crate::parsers::extract_text(path)? {
        Some(text) => Ok(text),
        None => Ok(fs::read_to_string(path)?),
    }
}

/// Reusable append-only ingestion into a project's `.aipk` embedding cache.
/// Owns the chunks/vectors writers and the state bookkeeping that `add-docs`,
/// and `add-dataset` share.
pub struct IngestSession {
    state: serde_json::Map<String, Value>,
    state_file: PathBuf,
    chunks_w: BufWriter<fs::File>,
    vectors_w: BufWriter<fs::File>,
    chunk_id: u32,
    dim: Option<u32>,
    added: usize,
    embed_model: String,
    embed_url: String,
    api_key: String,
    chunk_size: usize,
}

impl IngestSession {
    pub fn open(
        project_dir: &Path,
        embed_model: &str,
        embed_url: &str,
        api_key: &str,
        chunk_size: usize,
    ) -> Result<Self> {
        let cache_dir = project_dir.join(".aipk");
        fs::create_dir_all(&cache_dir)?;

        let state_file = cache_dir.join("state.json");
        let state: serde_json::Map<String, Value> = if state_file.exists() {
            serde_json::from_str(&fs::read_to_string(&state_file)?)?
        } else {
            serde_json::Map::new()
        };

        if let Some(existing) = state.get("embedding_model").and_then(|v| v.as_str()) {
            if existing != embed_model {
                anyhow::bail!(
                    "Embedding model mismatch: project uses '{}', requested '{}'.",
                    existing,
                    embed_model
                );
            }
        }

        let chunk_id = state
            .get("chunk_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let dim = state
            .get("embedding_dim")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        let chunks_w = BufWriter::new(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(cache_dir.join("chunks.jsonl"))?,
        );
        let vectors_w = BufWriter::new(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(cache_dir.join("vectors.bin"))?,
        );

        Ok(Self {
            state,
            state_file,
            chunks_w,
            vectors_w,
            chunk_id,
            dim,
            added: 0,
            embed_model: embed_model.to_string(),
            embed_url: embed_url.to_string(),
            api_key: api_key.to_string(),
            chunk_size,
        })
    }

    /// Chunk `text`, embed each chunk, and append to the project cache.
    /// Returns the number of chunks added.
    pub async fn add_text(&mut self, source: &str, text: &str) -> Result<usize> {
        let text_chunks = chunk_text(text, self.chunk_size);
        let count = text_chunks.len();
        for chunk in text_chunks {
            let vec = embed_query(&self.embed_url, &self.embed_model, &chunk, &self.api_key)
                .await
                .with_context(|| format!("embedding chunk from {source}"))?;
            if self.dim.is_none() {
                self.dim = Some(vec.len() as u32);
            }
            let line = serde_json::to_string(&json!({
                "id": self.chunk_id,
                "text": chunk,
                "source": source,
                "meta": {},
            }))?;
            writeln!(self.chunks_w, "{}", line)?;
            for &v in &vec {
                self.vectors_w.write_all(&v.to_le_bytes())?;
            }
            self.chunk_id += 1;
            self.added += 1;
        }
        Ok(count)
    }

    /// Flush writers and persist state. Returns (chunks added, total chunks).
    pub fn finish(mut self) -> Result<(usize, u32)> {
        self.chunks_w.flush()?;
        self.vectors_w.flush()?;
        self.state
            .insert("embedding_model".into(), json!(self.embed_model));
        self.state
            .insert("embedding_dim".into(), json!(self.dim.unwrap_or(0)));
        self.state
            .insert("chunk_count".into(), json!(self.chunk_id));
        fs::write(
            &self.state_file,
            serde_json::to_string_pretty(&Value::Object(self.state))?,
        )?;
        Ok((self.added, self.chunk_id))
    }
}

pub fn chunk_text(text: &str, chunk_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    for para in text.split("\n\n") {
        let words: Vec<&str> = para.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let mut current: Vec<&str> = Vec::new();
        for word in words {
            current.push(word);
            if current.len() >= chunk_size {
                chunks.push(current.join(" "));
                current.clear();
            }
        }
        if !current.is_empty() {
            chunks.push(current.join(" "));
        }
    }
    chunks
        .into_iter()
        .filter(|c| !c.trim().is_empty())
        .collect()
}
