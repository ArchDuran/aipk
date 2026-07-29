//! Dataset directories — raw materials for knowledge packages, kept separate
//! from any single package so one dataset can feed many builds.
//!
//! Layout: `~/Datasets/<name>/` (override root with $AIPK_DATASETS)
//!   - any files dropped into the directory are part of the dataset
//!   - `sources.toml` declares external links (local dirs, web pages, git repos)
//!   - `.cache/mirror/` holds synced copies of web/git sources
//!
//! Datasets hold raw text only — no embeddings. Embeddings belong to the
//! package (they depend on the embedding model) and are computed by
//! `aipk add-dataset` at ingest time.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cmd::add_docs::IngestSession;
use crate::cmd::add_git;
use crate::cmd::add_web;
use crate::parsers;

// ─── manifest ────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct SourcesManifest {
    #[serde(default)]
    pub dataset: DatasetMeta,
    #[serde(default, rename = "link")]
    pub links: Vec<LinkDef>,
}

#[derive(Deserialize, Default)]
pub struct DatasetMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Deserialize)]
pub struct LinkDef {
    #[serde(rename = "type")]
    pub ltype: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub ext: Vec<String>,
    #[serde(default)]
    pub sitemap: bool,
    #[serde(default = "default_max_pages")]
    pub max_pages: usize,
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u64,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default = "default_depth")]
    pub depth: u32,
    #[serde(default)]
    pub include_code: bool,
}

fn default_max_pages() -> usize {
    50
}
fn default_delay_ms() -> u64 {
    500
}
fn default_depth() -> u32 {
    1
}

// ─── paths ───────────────────────────────────────────────────────────────────

pub fn datasets_root() -> PathBuf {
    if let Ok(dir) = std::env::var("AIPK_DATASETS") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("Datasets")
}

/// Resolve a dataset by name (under the datasets root) or by direct path.
pub fn resolve(name_or_path: &str) -> Result<PathBuf> {
    let as_path = PathBuf::from(name_or_path);
    if as_path.is_dir() && (as_path.is_absolute() || name_or_path.contains('/')) {
        return Ok(as_path);
    }
    let under_root = datasets_root().join(name_or_path);
    if under_root.is_dir() {
        return Ok(under_root);
    }
    bail!(
        "Dataset '{}' not found (looked in {}).\nCreate it with: aipk dataset init {}",
        name_or_path,
        datasets_root().display(),
        name_or_path
    )
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

pub fn load_manifest(dataset_dir: &Path) -> Result<Option<SourcesManifest>> {
    let path = dataset_dir.join("sources.toml");
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    let manifest: SourcesManifest =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(manifest))
}

// ─── init ────────────────────────────────────────────────────────────────────

const SOURCES_TEMPLATE: &str = r#"# AIPK dataset manifest.
# Files placed in this directory are part of the dataset automatically
# (supported: md, txt, rst, html, pdf, docx, csv, tsv, json, yaml, ...).
# Links below add external sources. Secrets never go in this file —
# reference environment variables instead.

[dataset]
name = "{name}"
description = ""

# ── Local directory (e.g. an Obsidian vault) ─────────────────────────
# [[link]]
# type = "dir"
# path = "~/Documents/Obsidian/MyVault"
# ext = ["md"]            # optional filter; default: all supported formats

# ── Web pages or a sitemap (synced into .cache/ by `aipk dataset sync`) ──
# [[link]]
# type = "web"
# url = "https://docs.example.com/sitemap.xml"
# sitemap = true
# max_pages = 50
# delay_ms = 500

# ── Git repository (synced into .cache/ by `aipk dataset sync`) ──────
# [[link]]
# type = "git"
# url = "https://github.com/org/repo"
# branch = "main"
# include_code = false
# ext = []                # extra extensions, e.g. [".rst"]
"#;

pub fn init(name: &str) -> Result<()> {
    let dir = datasets_root().join(name);
    if dir.join("sources.toml").exists() {
        bail!("Dataset '{}' already exists: {}", name, dir.display());
    }
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join("sources.toml"),
        SOURCES_TEMPLATE.replace("{name}", name),
    )?;
    println!("✓ Created dataset: {}", dir.display());
    println!("  Drop files into the directory and/or edit sources.toml,");
    println!("  then: aipk dataset sync {name}   (if you added web/git links)");
    println!("  and:  aipk add-dataset {name} --dir <project>");
    Ok(())
}

// ─── list ────────────────────────────────────────────────────────────────────

pub fn list() -> Result<()> {
    let root = datasets_root();
    if !root.is_dir() {
        println!("No datasets yet ({} does not exist).", root.display());
        println!("Create one with: aipk dataset init <name>");
        return Ok(());
    }
    let mut found = 0;
    let mut entries: Vec<_> = fs::read_dir(&root)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    println!("Datasets in {}:", root.display());
    for entry in entries {
        let dir = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let manifest = load_manifest(&dir).unwrap_or(None);
        let (desc, links) = manifest
            .map(|m| (m.dataset.description, m.links.len()))
            .unwrap_or_default();
        let files = count_own_files(&dir);
        let synced = dir.join(".cache").join("mirror").is_dir();
        print!("  {name}  — {files} file(s), {links} link(s)");
        if synced {
            print!(", synced");
        }
        if !desc.is_empty() {
            print!("  · {desc}");
        }
        println!();
        found += 1;
    }
    if found == 0 {
        println!("  (empty)");
    }
    Ok(())
}

// ─── sync ────────────────────────────────────────────────────────────────────

/// Download web/git links into `.cache/mirror/` so ingestion is reproducible
/// and works offline afterwards. `dir` links are read live and need no sync.
pub async fn sync(name_or_path: &str) -> Result<()> {
    let dir = resolve(name_or_path)?;
    let manifest = match load_manifest(&dir)? {
        Some(m) => m,
        None => {
            println!("No sources.toml in {} — nothing to sync.", dir.display());
            return Ok(());
        }
    };
    if manifest.links.is_empty() {
        println!("sources.toml has no [[link]] entries — nothing to sync.");
        return Ok(());
    }

    let mirror = dir.join(".cache").join("mirror");
    fs::create_dir_all(&mirror)?;

    for (i, link) in manifest.links.iter().enumerate() {
        match link.ltype.as_str() {
            "dir" => {
                let p = expand_tilde(&link.path);
                if p.is_dir() {
                    println!(
                        "[{i}] dir  {} — read live at ingest, no sync needed",
                        p.display()
                    );
                } else {
                    eprintln!("[{i}] dir  {} — WARNING: directory not found", p.display());
                }
            }
            "web" => sync_web(link, &mirror, i).await?,
            "git" => sync_git(link, &mirror, i)?,
            "confluence" | "notion" => {
                println!(
                    "[{i}] {}  — API connector not implemented yet. Workaround: export \
                     pages to markdown/HTML and drop the files into the dataset directory.",
                    link.ltype
                );
            }
            other => eprintln!("[{i}] unknown link type '{other}' — skipped"),
        }
    }
    println!("✓ Sync complete: {}", mirror.display());
    Ok(())
}

async fn sync_web(link: &LinkDef, mirror: &Path, idx: usize) -> Result<()> {
    if link.url.is_empty() {
        bail!("[{idx}] web link has no url");
    }
    let client = reqwest::Client::builder()
        .user_agent("aipk-dataset-sync")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let urls: Vec<String> = if link.sitemap {
        let mut all = add_web::fetch_sitemap(&client, &link.url).await?;
        all.truncate(link.max_pages);
        all
    } else {
        vec![link.url.clone()]
    };

    let out_dir = mirror.join(format!(
        "web-{}",
        sanitize_label(&add_web::url_to_label(&link.url))
    ));
    fs::create_dir_all(&out_dir)?;

    println!("[{idx}] web  {} — {} page(s)", link.url, urls.len());
    for (n, url) in urls.iter().enumerate() {
        match add_web::fetch_text(&client, url).await {
            Ok(text) => {
                let label = sanitize_label(&add_web::url_to_label(url));
                fs::write(out_dir.join(format!("{label}.txt")), text)?;
                println!("     [{}/{}] {}", n + 1, urls.len(), url);
            }
            Err(e) => eprintln!("     [{}/{}] {} — failed: {e}", n + 1, urls.len(), url),
        }
        if n + 1 < urls.len() {
            tokio::time::sleep(std::time::Duration::from_millis(link.delay_ms)).await;
        }
    }
    Ok(())
}

fn sync_git(link: &LinkDef, mirror: &Path, idx: usize) -> Result<()> {
    if link.url.is_empty() {
        bail!("[{idx}] git link has no url");
    }
    let tmp_path = add_git::make_tmp_path(&link.url);
    add_git::clone_repo(&link.url, &tmp_path, link.branch.as_deref(), link.depth)
        .with_context(|| format!("cloning {}", link.url))?;
    let _tmp = add_git::TmpDir(tmp_path.clone());

    let mut allowed: Vec<&str> = add_git::DEFAULT_EXTENSIONS.to_vec();
    if link.include_code {
        allowed.extend_from_slice(add_git::CODE_EXTENSIONS);
    }
    for e in &link.ext {
        allowed.push(e.as_str());
    }

    let files = add_git::collect_files(&tmp_path, &allowed);
    let out_dir = mirror.join(format!("git-{}", sanitize_label(&link.url)));
    // Fresh mirror each sync: removed upstream files disappear here too.
    let _ = fs::remove_dir_all(&out_dir);
    fs::create_dir_all(&out_dir)?;

    for (path, rel) in &files {
        let dest = out_dir.join(sanitize_label(rel));
        if let Ok(content) = fs::read(path) {
            fs::write(dest, content)?;
        }
    }
    println!(
        "[{idx}] git  {} — {} file(s) mirrored",
        link.url,
        files.len()
    );
    Ok(())
}

fn sanitize_label(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ─── add-dataset (ingest into a project) ────────────────────────────────────

pub async fn add(
    name_or_path: &str,
    project_dir: &Path,
    embed_model: &str,
    embed_url: &str,
    api_key: &str,
    chunk_size: usize,
) -> Result<()> {
    let dir = resolve(name_or_path)?;
    let manifest = load_manifest(&dir)?;

    let mut session =
        IngestSession::open(project_dir, embed_model, embed_url, api_key, chunk_size)?;
    let mut files_done = 0usize;
    let mut files_skipped = 0usize;

    // 1. Files physically in the dataset directory
    for path in walk_own_files(&dir) {
        let label = path
            .strip_prefix(&dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        match parsers::extract_text(&path) {
            Ok(Some(text)) if !text.trim().is_empty() => {
                let n = session.add_text(&label, &text).await?;
                eprintln!("  {} → {} chunks", label, n);
                files_done += 1;
            }
            Ok(_) => files_skipped += 1,
            Err(e) => {
                eprintln!("  {} — skipped: {e}", label);
                files_skipped += 1;
            }
        }
    }

    // 2. Mirrored web/git content from `aipk dataset sync`
    let mirror = dir.join(".cache").join("mirror");
    if mirror.is_dir() {
        for path in walk_all_files(&mirror) {
            let label = path
                .strip_prefix(&mirror)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            // Mirror files were vetted at sync time — read anything as text.
            let text = match parsers::extract_text(&path) {
                Ok(Some(t)) => t,
                _ => fs::read_to_string(&path).unwrap_or_default(),
            };
            if text.trim().is_empty() {
                files_skipped += 1;
                continue;
            }
            let n = session.add_text(&label, &text).await?;
            eprintln!("  {} → {} chunks", label, n);
            files_done += 1;
        }
    }

    // 3. Live `dir` links (Obsidian vaults etc.)
    let mut has_unsynced_remote = false;
    if let Some(m) = &manifest {
        for link in &m.links {
            match link.ltype.as_str() {
                "dir" => {
                    let root = expand_tilde(&link.path);
                    if !root.is_dir() {
                        eprintln!("  dir link {} — not found, skipped", root.display());
                        continue;
                    }
                    for path in walk_all_files(&root) {
                        let matches_ext = if link.ext.is_empty() {
                            parsers::is_supported(&path)
                        } else {
                            let ext = path
                                .extension()
                                .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                                .unwrap_or_default();
                            link.ext
                                .iter()
                                .any(|allowed| ext == *allowed || ext == format!(".{allowed}"))
                        };
                        if !matches_ext {
                            continue;
                        }
                        let label = format!(
                            "{}/{}",
                            root.file_name()
                                .map(|n| n.to_string_lossy())
                                .unwrap_or_default(),
                            path.strip_prefix(&root).unwrap_or(&path).to_string_lossy()
                        );
                        let text = match parsers::extract_text(&path) {
                            Ok(Some(t)) => t,
                            Ok(None) => fs::read_to_string(&path).unwrap_or_default(),
                            Err(e) => {
                                eprintln!("  {} — skipped: {e}", label);
                                files_skipped += 1;
                                continue;
                            }
                        };
                        if text.trim().is_empty() {
                            files_skipped += 1;
                            continue;
                        }
                        let n = session.add_text(&label, &text).await?;
                        eprintln!("  {} → {} chunks", label, n);
                        files_done += 1;
                    }
                }
                "web" | "git" if !mirror.is_dir() => has_unsynced_remote = true,
                _ => {}
            }
        }
    }

    let (added, total) = session.finish()?;
    let display_name = manifest
        .as_ref()
        .map(|m| m.dataset.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            dir.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        });
    println!(
        "✓ Dataset '{}': {} file(s) ingested, {} skipped, {} chunks added (project total: {})",
        display_name, files_done, files_skipped, added, total
    );
    if has_unsynced_remote {
        println!(
            "  Note: sources.toml has web/git links but no .cache/mirror — run \
             `aipk dataset sync` first to include them."
        );
    }
    Ok(())
}

// ─── walkers ─────────────────────────────────────────────────────────────────

/// Files in the dataset dir itself: skip hidden entries, `.cache`, `sources.toml`.
fn walk_own_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_filtered(dir, &mut out, &|name| {
        !name.starts_with('.') && name != "sources.toml"
    });
    out.sort();
    out
}

fn walk_all_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_filtered(dir, &mut out, &|name| !name.starts_with('.'));
    out.sort();
    out
}

fn walk_filtered(dir: &Path, out: &mut Vec<PathBuf>, keep: &dyn Fn(&str) -> bool) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if !keep(&name) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk_filtered(&path, out, keep);
        } else {
            out.push(path);
        }
    }
}

fn count_own_files(dir: &Path) -> usize {
    walk_own_files(dir).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dataset(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("aipk_dataset_tests").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn manifest_parses_links() {
        let dir = tmp_dataset("manifest");
        fs::write(
            dir.join("sources.toml"),
            r#"
[dataset]
name = "test"
description = "demo"

[[link]]
type = "dir"
path = "~/notes"
ext = ["md"]

[[link]]
type = "web"
url = "https://example.com/sitemap.xml"
sitemap = true
max_pages = 10
"#,
        )
        .unwrap();
        let m = load_manifest(&dir).unwrap().unwrap();
        assert_eq!(m.dataset.name, "test");
        assert_eq!(m.links.len(), 2);
        assert_eq!(m.links[0].ltype, "dir");
        assert_eq!(m.links[0].ext, vec!["md"]);
        assert!(m.links[1].sitemap);
        assert_eq!(m.links[1].max_pages, 10);
        assert_eq!(m.links[1].delay_ms, 500); // default
    }

    #[test]
    fn missing_manifest_is_none() {
        let dir = tmp_dataset("empty");
        assert!(load_manifest(&dir).unwrap().is_none());
    }

    #[test]
    fn own_files_walk_skips_cache_and_manifest() {
        let dir = tmp_dataset("walk");
        fs::write(dir.join("sources.toml"), "[dataset]\nname=\"x\"").unwrap();
        fs::write(dir.join("doc.md"), "hello").unwrap();
        fs::create_dir_all(dir.join(".cache/mirror")).unwrap();
        fs::write(dir.join(".cache/mirror/page.txt"), "cached").unwrap();
        fs::create_dir_all(dir.join("notes")).unwrap();
        fs::write(dir.join("notes/a.txt"), "a").unwrap();

        let files = walk_own_files(&dir);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["doc.md", "a.txt"]); // sorted by full path
    }

    #[test]
    fn sanitize_label_replaces_separators() {
        assert_eq!(sanitize_label("docs/page one"), "docs_page_one");
        assert_eq!(sanitize_label("a.b-c_d"), "a.b-c_d");
    }

    #[test]
    fn expand_tilde_home() {
        let p = expand_tilde("~/x");
        assert!(!p.to_string_lossy().starts_with('~'));
        assert!(p.to_string_lossy().ends_with("/x"));
    }
}
