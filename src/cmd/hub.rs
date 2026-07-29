use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Index types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HubEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub file: String, // filename relative to packages_dir()
    pub installed_at: String,
    pub source: String,     // "local" | "url:<url>"
    pub visibility: String, // "private" | "public"
    pub tags: Vec<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct HubIndex {
    version: u32,
    packages: Vec<HubEntry>,
}

impl HubIndex {
    fn load() -> Result<Self> {
        let p = index_file();
        if !p.exists() {
            return Ok(Self {
                version: 1,
                packages: vec![],
            });
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?)
    }

    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(packages_dir())?;
        std::fs::write(index_file(), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    fn find(&self, spec: &str) -> Option<&HubEntry> {
        let (name, ver) = split_spec(spec);
        self.packages
            .iter()
            .find(|e| e.name == name && ver.as_deref().is_none_or(|v| e.version == v))
    }

    fn search_entries(&self, query: &str) -> Vec<&HubEntry> {
        let q = query.to_lowercase();
        self.packages
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&q)
                    || e.description.to_lowercase().contains(&q)
                    || e.tags.iter().any(|t| t.to_lowercase().contains(&q))
                    || e.author.to_lowercase().contains(&q)
            })
            .collect()
    }

    fn add(&mut self, entry: HubEntry) {
        self.packages
            .retain(|e| !(e.name == entry.name && e.version == entry.version));
        self.packages.push(entry);
    }

    fn remove_entry(&mut self, spec: &str) -> bool {
        let (name, ver) = split_spec(spec);
        let before = self.packages.len();
        self.packages
            .retain(|e| !(e.name == name && ver.as_deref().is_none_or(|v| e.version == v)));
        self.packages.len() < before
    }
}

// ── Paths ────────────────────────────────────────────────────────────────────

fn hub_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".aipk")
}

fn packages_dir() -> PathBuf {
    hub_dir().join("packages")
}

fn index_file() -> PathBuf {
    packages_dir().join("index.json")
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Parse "name@version" → ("name", Some("version")) or ("name", None)
fn split_spec(spec: &str) -> (String, Option<String>) {
    if let Some(at) = spec.find('@') {
        (spec[..at].to_string(), Some(spec[at + 1..].to_string()))
    } else {
        (spec.to_string(), None)
    }
}

/// Read name/version/description/author/tags from a .aipk file.
fn read_meta(path: &Path) -> (String, String, String, String, Vec<String>) {
    let pkg = match crate::format::parse(path) {
        Ok(p) => p,
        Err(_) => {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            return (stem, "0.0.0".into(), String::new(), String::new(), vec![]);
        }
    };

    let meta = pkg
        .meta()
        .unwrap_or(toml::Value::Table(toml::map::Map::new()));

    let get = |section: &str, key: &str| -> String {
        meta.get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    let name = {
        let n = get("package", "name");
        if n.is_empty() {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            n
        }
    };

    let compatible: Vec<String> = meta
        .get("model")
        .and_then(|m| m.get("compatible"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    (
        name,
        get("package", "version"),
        get("package", "description"),
        get("package", "author"),
        compatible,
    )
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List all installed packages.
pub fn list(json: bool) -> Result<()> {
    let idx = HubIndex::load()?;

    if idx.packages.is_empty() {
        println!("Hub is empty. Use `aipk hub install <path>` to add packages.");
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&idx.packages)?);
        return Ok(());
    }

    println!(
        "{:<30} {:<10} {:<12} {:<8}  DESCRIPTION",
        "NAME", "VERSION", "VISIBILITY", "SIZE"
    );
    println!("{}", "─".repeat(90));
    for e in &idx.packages {
        let size = if e.size_bytes >= 1024 * 1024 {
            format!("{:.1}MB", e.size_bytes as f64 / 1_048_576.0)
        } else {
            format!("{:.0}KB", e.size_bytes as f64 / 1024.0)
        };
        let desc: String = e.description.chars().take(40).collect();
        println!(
            "{:<30} {:<10} {:<12} {:<8}  {}",
            e.name, e.version, e.visibility, size, desc
        );
    }
    println!();
    println!(
        "{} package(s) installed  →  {}",
        idx.packages.len(),
        packages_dir().display()
    );
    Ok(())
}

/// Search packages by name, description, or tag.
pub fn search(query: &str, json: bool) -> Result<()> {
    let idx = HubIndex::load()?;
    let results = idx.search_entries(query);

    if results.is_empty() {
        println!("No packages matching '{query}'.");
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }

    println!("Results for '{query}':");
    println!();
    for e in &results {
        println!("  {}@{}  —  {}", e.name, e.version, e.description);
        if !e.tags.is_empty() {
            println!("    tags: {}", e.tags.join(", "));
        }
    }
    println!();
    println!("{} result(s)", results.len());
    Ok(())
}

/// Install a package from a local path or URL.
pub async fn install(source: &str, name_override: Option<&str>) -> Result<()> {
    let bytes = if source.starts_with("http://") || source.starts_with("https://") {
        download_url(source).await?
    } else {
        let p = Path::new(source);
        if !p.exists() {
            bail!("File not found: {source}");
        }
        std::fs::read(p)?
    };

    register_package(bytes, source, "private", name_override)
}

/// Add a local package to the hub (marks source as "published").
pub fn publish(path: &Path, public: bool) -> Result<()> {
    if !path.exists() {
        bail!("File not found: {}", path.display());
    }
    let bytes = std::fs::read(path)?;
    let vis = if public { "public" } else { "private" };
    let source = format!("published:{}", path.display());
    register_package(bytes, &source, vis, None)
}

/// Re-download packages that were installed from URLs.
pub async fn update(name: Option<&str>) -> Result<()> {
    let mut idx = HubIndex::load()?;
    let entries: Vec<HubEntry> = idx
        .packages
        .iter()
        .filter(|e| e.source.starts_with("url:") && name.is_none_or(|n| e.name == n))
        .cloned()
        .collect();

    if entries.is_empty() {
        println!("Nothing to update (only URL-sourced packages can be updated).");
        return Ok(());
    }

    let mut updated = 0;
    for entry in &entries {
        let url = &entry.source["url:".len()..];
        print!("  Updating {} from {} ... ", entry.name, url);
        match download_url(url).await {
            Ok(bytes) => {
                let dest = packages_dir().join(&entry.file);
                std::fs::write(&dest, &bytes)?;
                let size = bytes.len() as u64;
                // Update size in index
                if let Some(e) = idx.packages.iter_mut().find(|e| e.name == entry.name) {
                    e.size_bytes = size;
                    e.installed_at =
                        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                }
                println!("ok ({:.1} KB)", size as f64 / 1024.0);
                updated += 1;
            }
            Err(e) => println!("failed: {e}"),
        }
    }

    idx.save()?;
    println!("{}/{} packages updated", updated, entries.len());
    Ok(())
}

/// Show detailed info about an installed package.
pub fn info(spec: &str) -> Result<()> {
    let idx = HubIndex::load()?;
    let entry = idx.find(spec).ok_or_else(|| {
        anyhow::anyhow!(
            "Package '{}' not found. Use `aipk hub list` to see installed packages.",
            spec
        )
    })?;

    let pkg_path = packages_dir().join(&entry.file);

    println!("Package:     {}@{}", entry.name, entry.version);
    println!("Description: {}", entry.description);
    println!("Author:      {}", entry.author);
    println!("Visibility:  {}", entry.visibility);
    println!("Source:      {}", entry.source);
    println!("Installed:   {}", entry.installed_at);
    println!("File:        {}", pkg_path.display());
    println!("Size:        {:.1} KB", entry.size_bytes as f64 / 1024.0);
    if !entry.tags.is_empty() {
        println!("Tags:        {}", entry.tags.join(", "));
    }

    // Live stats from the package file
    if pkg_path.exists() {
        if let Ok(pkg) = crate::format::parse(&pkg_path) {
            println!();
            println!("Sections:");
            let skills = pkg.skills();
            if !skills.is_empty() {
                println!(
                    "  skills:  {} ({})",
                    skills.len(),
                    skills
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            if let Ok(meta) = pkg.meta() {
                if let Some(model) = meta.get("model") {
                    if let Some(dim) = model.get("embedding_dim").and_then(|v| v.as_integer()) {
                        if dim > 0 {
                            println!("  embedding_dim: {dim}");
                        }
                    }
                    if let Some(em) = model.get("embedding_model").and_then(|v| v.as_str()) {
                        if !em.is_empty() {
                            println!("  embedding_model: {em}");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Remove a package from the hub.
pub fn remove(spec: &str) -> Result<()> {
    let mut idx = HubIndex::load()?;

    let entry = idx
        .find(spec)
        .ok_or_else(|| anyhow::anyhow!("Package '{}' not found.", spec))?
        .clone();

    let pkg_path = packages_dir().join(&entry.file);
    if pkg_path.exists() {
        std::fs::remove_file(&pkg_path)?;
    }

    idx.remove_entry(spec);
    idx.save()?;

    println!("✓ Removed {}@{}", entry.name, entry.version);
    Ok(())
}

/// Paths of all installed hub packages (used by `aipk up`).
pub fn all_package_paths() -> Result<Vec<PathBuf>> {
    let idx = HubIndex::load()?;
    Ok(idx
        .packages
        .iter()
        .map(|e| packages_dir().join(&e.file))
        .filter(|p| p.exists())
        .collect())
}

/// Serve all hub packages (or specific ones by name).
#[allow(clippy::too_many_arguments)]
pub async fn serve(
    names: Vec<String>,
    model: String,
    llm_url: String,
    embed_url: Option<String>,
    llm_api_key: String,
    embed_model: String,
    lang: Option<String>,
    strict_verify: bool,
    strict_render: bool,
    host: &str,
    port: u16,
    trust_tools: bool,
    allow: &[String],
    allow_tool: &[String],
    enforce: crate::runtime::EnforceMode,
) -> Result<()> {
    let idx = HubIndex::load()?;

    if idx.packages.is_empty() {
        bail!("Hub is empty. Install packages first with `aipk hub install <path>`.");
    }

    let packages: Vec<PathBuf> = if names.is_empty() {
        idx.packages
            .iter()
            .map(|e| packages_dir().join(&e.file))
            .filter(|p| p.exists())
            .collect()
    } else {
        let mut paths = vec![];
        for name in &names {
            match idx.find(name) {
                Some(e) => paths.push(packages_dir().join(&e.file)),
                None => bail!("Package '{}' not found in hub.", name),
            }
        }
        paths
    };

    if packages.is_empty() {
        bail!("No valid package files found.");
    }

    crate::cmd::serve::run(
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
        false, // graph mode not supported from hub serve yet
        crate::agent::DEFAULT_MAX_TOKENS,
        host,
        port,
        None, // passphrase not supported from hub serve
        trust_tools,
        allow,
        allow_tool,
    )
    .await
}

/// Show hub directory, package count, and disk usage.
pub fn status() -> Result<()> {
    let dir = packages_dir();
    let idx = HubIndex::load()?;

    println!("Hub directory:  {}", dir.display());
    println!("Index:          {}", index_file().display());
    println!("Packages:       {}", idx.packages.len());

    let total_bytes: u64 = idx.packages.iter().map(|e| e.size_bytes).sum();
    if total_bytes > 0 {
        println!("Total size:     {:.1} MB", total_bytes as f64 / 1_048_576.0);
    }

    if idx.packages.is_empty() {
        println!();
        println!("Hub is empty.");
        println!("  aipk hub install <path>     — add from local file");
        println!("  aipk hub install <url>      — download and add");
        println!("  aipk hub publish <pkg.aipk> — publish your package");
    }
    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────────────────

async fn download_url(url: &str) -> Result<Vec<u8>> {
    let client = crate::agent::build_client();
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        bail!("HTTP {}: {}", resp.status(), url);
    }
    Ok(resp.bytes().await?.to_vec())
}

fn register_package(
    bytes: Vec<u8>,
    source: &str,
    visibility: &str,
    name_override: Option<&str>,
) -> Result<()> {
    // Write to a temp file to parse metadata, then move to final location.
    let tmp = packages_dir().join(format!(".tmp_{}.aipk", std::process::id()));
    std::fs::create_dir_all(packages_dir())?;
    std::fs::write(&tmp, &bytes)?;

    let (pkg_name, version, description, author, tags) = read_meta(&tmp);
    std::fs::remove_file(&tmp)?;

    let name = name_override.unwrap_or(&pkg_name).to_string();
    let ver = if version.is_empty() {
        "0.0.0".to_string()
    } else {
        version
    };

    let filename = format!("{}@{}.aipk", name, ver);
    let dest = packages_dir().join(&filename);
    std::fs::write(&dest, &bytes)?;

    let entry = HubEntry {
        name: name.clone(),
        version: ver.clone(),
        description,
        author,
        file: filename,
        installed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        source: if source.starts_with("http") {
            format!("url:{source}")
        } else {
            source.to_string()
        },
        visibility: visibility.to_string(),
        tags,
        size_bytes: bytes.len() as u64,
    };

    let mut idx = HubIndex::load()?;
    let is_update = idx.find(&format!("{name}@{ver}")).is_some();
    idx.add(entry);
    idx.save()?;

    if is_update {
        println!(
            "✓ Updated {name}@{ver}  ({:.1} KB)",
            bytes.len() as f64 / 1024.0
        );
    } else {
        println!(
            "✓ Installed {name}@{ver}  ({:.1} KB)",
            bytes.len() as f64 / 1024.0
        );
        println!(
            "  {}",
            packages_dir().join(format!("{name}@{ver}.aipk")).display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_spec_without_version() {
        let (name, ver) = split_spec("legal-eu");
        assert_eq!(name, "legal-eu");
        assert!(ver.is_none());
    }

    #[test]
    fn split_spec_with_version() {
        let (name, ver) = split_spec("legal-eu@2.1.0");
        assert_eq!(name, "legal-eu");
        assert_eq!(ver.as_deref(), Some("2.1.0"));
    }

    #[test]
    fn hub_index_search_case_insensitive() {
        let idx = HubIndex {
            version: 1,
            packages: vec![HubEntry {
                name: "Legal-EU".into(),
                version: "1.0.0".into(),
                description: "GDPR compliance assistant".into(),
                author: "test".into(),
                file: "legal-eu@1.0.0.aipk".into(),
                installed_at: "".into(),
                source: "local".into(),
                visibility: "private".into(),
                tags: vec!["legal".into(), "gdpr".into()],
                size_bytes: 1024,
            }],
        };
        assert_eq!(idx.search_entries("gdpr").len(), 1);
        assert_eq!(idx.search_entries("GDPR").len(), 1);
        assert_eq!(idx.search_entries("medical").len(), 0);
    }

    #[test]
    fn hub_index_find_by_name() {
        let idx = HubIndex {
            version: 1,
            packages: vec![HubEntry {
                name: "rust-book".into(),
                version: "0.1.0".into(),
                description: "".into(),
                author: "".into(),
                file: "rust-book@0.1.0.aipk".into(),
                installed_at: "".into(),
                source: "local".into(),
                visibility: "private".into(),
                tags: vec![],
                size_bytes: 0,
            }],
        };
        assert!(idx.find("rust-book").is_some());
        assert!(idx.find("rust-book@0.1.0").is_some());
        assert!(idx.find("rust-book@9.9.9").is_none());
        assert!(idx.find("unknown").is_none());
    }

    #[test]
    fn hub_index_add_replaces_same_version() {
        let mut idx = HubIndex {
            version: 1,
            packages: vec![],
        };
        let entry = |desc: &str| HubEntry {
            name: "pkg".into(),
            version: "1.0.0".into(),
            description: desc.into(),
            author: "".into(),
            file: "pkg@1.0.0.aipk".into(),
            installed_at: "".into(),
            source: "local".into(),
            visibility: "private".into(),
            tags: vec![],
            size_bytes: 0,
        };
        idx.add(entry("first"));
        idx.add(entry("second"));
        assert_eq!(idx.packages.len(), 1);
        assert_eq!(idx.packages[0].description, "second");
    }

    #[test]
    fn hub_index_remove_by_name() {
        let mut idx = HubIndex {
            version: 1,
            packages: vec![HubEntry {
                name: "pkg".into(),
                version: "1.0.0".into(),
                description: "".into(),
                author: "".into(),
                file: "pkg@1.0.0.aipk".into(),
                installed_at: "".into(),
                source: "local".into(),
                visibility: "private".into(),
                tags: vec![],
                size_bytes: 0,
            }],
        };
        assert!(idx.remove_entry("pkg"));
        assert!(idx.packages.is_empty());
        assert!(!idx.remove_entry("pkg")); // already gone
    }
}
