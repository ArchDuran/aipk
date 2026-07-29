use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cmd::add_docs::chunk_text;
use crate::llm::embed_query;

pub const DEFAULT_EXTENSIONS: &[&str] = &[".md", ".txt", ".rst", ".mdx", ".adoc"];
pub const CODE_EXTENSIONS: &[&str] = &[
    ".rs", ".py", ".js", ".ts", ".go", ".java", ".cpp", ".c", ".h", ".rb", ".php", ".swift", ".kt",
    ".toml", ".yaml", ".yml", ".json",
];
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".tox",
    "__pycache__",
    "dist",
    "build",
    ".venv",
    "venv",
    ".mypy_cache",
];

/// Temporary directory that is removed when dropped.
pub struct TmpDir(pub PathBuf);

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    project_dir: &Path,
    source: &str,
    embed_model: &str,
    llm_url: &str,
    api_key: &str,
    chunk_size: usize,
    extra_extensions: &[String],
    include_code: bool,
    branch: Option<&str>,
    depth: u32,
) -> Result<()> {
    let is_url =
        source.starts_with("http") || source.starts_with("git@") || source.starts_with("ssh://");

    // ── Resolve repo directory ────────────────────────────────────────────────
    let (_tmp, repo_dir) = if is_url {
        let tmp_path = make_tmp_path(source);
        clone_repo(source, &tmp_path, branch, depth)
            .with_context(|| format!("cloning {source}"))?;
        let tmp = TmpDir(tmp_path.clone());
        (Some(tmp), tmp_path)
    } else {
        let path = PathBuf::from(source);
        if !path.exists() {
            anyhow::bail!("Local path not found: {}", path.display());
        }
        (None, path)
    };

    // ── Collect matching files ────────────────────────────────────────────────
    let mut allowed: Vec<&str> = DEFAULT_EXTENSIONS.to_vec();
    if include_code {
        allowed.extend_from_slice(CODE_EXTENSIONS);
    }
    for ext in extra_extensions {
        allowed.push(ext.as_str());
    }

    let files = collect_files(&repo_dir, &allowed);
    if files.is_empty() {
        println!(
            "No matching files found in {}. Use --include-code or --ext to add more extensions.",
            source
        );
        return Ok(());
    }
    println!(
        "Found {} file(s) in {} with {} extension(s)",
        files.len(),
        source,
        allowed.len()
    );

    // ── Load existing embedding state ─────────────────────────────────────────
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

    // ── Embed files ───────────────────────────────────────────────────────────
    let mut added = 0usize;
    let mut skipped = 0usize;

    for (abs_path, rel_source) in &files {
        let text = match fs::read_to_string(abs_path) {
            Ok(t) => t,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if text.trim().is_empty() {
            skipped += 1;
            continue;
        }

        let text_chunks = chunk_text(&text, chunk_size);
        if text_chunks.is_empty() {
            skipped += 1;
            continue;
        }
        eprintln!("  {} → {} chunks", rel_source, text_chunks.len());

        for chunk in text_chunks {
            let vec = embed_query(llm_url, embed_model, &chunk, api_key)
                .await
                .with_context(|| format!("embedding chunk from {rel_source}"))?;

            if dim.is_none() {
                dim = Some(vec.len() as u32);
            }

            let line = serde_json::to_string(&json!({
                "id": chunk_id,
                "text": chunk,
                "source": rel_source,
                "meta": {"git_source": source},
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
        "✓ git import: {} chunks added, {} file(s) skipped (total: {}, dim={})",
        added,
        skipped,
        chunk_id,
        dim.unwrap_or(0)
    );
    Ok(())
}

/// Clone a git repo into dest_dir (shallow clone, no LFS by default).
pub fn clone_repo(url: &str, dest: &Path, branch: Option<&str>, depth: u32) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("clone")
        .arg("--depth")
        .arg(depth.to_string())
        .arg("--single-branch");

    if let Some(b) = branch {
        cmd.args(["--branch", b]);
    }

    cmd.arg(url).arg(dest);

    let status = cmd
        .status()
        .context("git not found — install git and ensure it is on PATH")?;

    if !status.success() {
        anyhow::bail!("git clone failed for {url}");
    }
    Ok(())
}

/// Build a deterministic temp path from the URL/source string and process ID.
pub fn make_tmp_path(source: &str) -> PathBuf {
    let slug: String = source
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(32)
        .collect();
    std::env::temp_dir().join(format!("aipk_git_{}_{}", std::process::id(), slug))
}

/// Walk `root` and return (absolute_path, repo_relative_source) for files with matching extensions.
/// Skips SKIP_DIRS and binary-ish files.
pub fn collect_files(root: &Path, extensions: &[&str]) -> Vec<(PathBuf, String)> {
    let mut result = Vec::new();
    walk_dir(root, root, extensions, &mut result);
    result.sort_by(|a, b| a.1.cmp(&b.1));
    result
}

fn walk_dir(root: &Path, dir: &Path, extensions: &[&str], out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk_dir(root, &path, extensions, out);
            }
            continue;
        }

        if !path.is_file() {
            continue;
        }

        let has_ext = extensions.iter().any(|ext| name.ends_with(ext));
        if !has_ext {
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| name.clone());

        out.push((path, rel));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_tmp_path_contains_pid() {
        let p = make_tmp_path("https://github.com/example/repo");
        let s = p.to_string_lossy();
        assert!(s.contains("aipk_git_"));
        assert!(s.contains(&std::process::id().to_string()));
    }

    #[test]
    fn collect_files_finds_md_files() {
        let tmp = std::env::temp_dir().join(format!("aipk_test_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("readme.md"), "# Hello").unwrap();
        std::fs::write(tmp.join("script.py"), "print('hi')").unwrap();
        std::fs::write(tmp.join("notes.txt"), "notes").unwrap();

        let files = collect_files(&tmp, &[".md", ".txt"]);
        let names: Vec<&str> = files.iter().map(|(_, s)| s.as_str()).collect();
        assert!(names.contains(&"readme.md"));
        assert!(names.contains(&"notes.txt"));
        assert!(!names.iter().any(|n| n.contains(".py")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn collect_files_skips_git_dir() {
        let tmp = std::env::temp_dir().join(format!("aipk_test_git_{}", std::process::id()));
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        std::fs::write(tmp.join(".git").join("config.md"), "should be ignored").unwrap();
        std::fs::write(tmp.join("readme.md"), "real file").unwrap();

        let files = collect_files(&tmp, &[".md"]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, "readme.md");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn walk_dir_skips_node_modules() {
        let tmp = std::env::temp_dir().join(format!("aipk_test_nm_{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("node_modules")).unwrap();
        std::fs::write(tmp.join("node_modules").join("lib.md"), "should skip").unwrap();
        std::fs::write(tmp.join("index.md"), "keep").unwrap();

        let files = collect_files(&tmp, &[".md"]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, "index.md");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
