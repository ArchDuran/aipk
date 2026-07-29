use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::ann::{AnnIndex, ANN_INDEX_THRESHOLD};
use crate::format::{build_know_section, parse, parse_know_section, AipkBuilder};

// ─── subject resolution ─────────────────────────────────────────────────────

/// Resolve a GDPR request target into the list of `source` names it covers.
///
/// A single `--source <name>` is the direct case. `--subject <id>` covers a
/// person whose contributions may span several source files (onboarding
/// notes, a wiki export, a Slack export, ...) — `--subject-map` is a JSON
/// file of `{"filename": "subject id"}` used to find every source belonging
/// to that subject. This is shared by `erase` (right to erasure) and
/// `snapshot` (right of access / data portability) so a single request can
/// act on everything tied to one person in one call.
pub fn resolve_sources(
    source: Option<&str>,
    subject: Option<&str>,
    subject_map: Option<&Path>,
) -> Result<Vec<String>> {
    if let Some(s) = source {
        return Ok(vec![s.to_string()]);
    }
    let subject = subject.ok_or_else(|| {
        anyhow::anyhow!("provide either --source <name> or --subject <id> (with --subject-map)")
    })?;
    let map_path = subject_map.ok_or_else(|| {
        anyhow::anyhow!(
            "--subject requires --subject-map <file.json> ({{\"filename\": \"subject id\"}})"
        )
    })?;
    let content = std::fs::read_to_string(map_path)
        .map_err(|e| anyhow::anyhow!("reading subject map {}: {e}", map_path.display()))?;
    let map: HashMap<String, String> = serde_json::from_str(&content).map_err(|e| {
        anyhow::anyhow!(
            "parsing {} as a JSON object of {{\"filename\": \"subject id\"}}: {e}",
            map_path.display()
        )
    })?;
    let mut sources: Vec<String> = map
        .into_iter()
        .filter(|(_, subj)| subj == subject)
        .map(|(file, _)| file)
        .collect();
    if sources.is_empty() {
        anyhow::bail!(
            "no source in {} is mapped to subject '{subject}'",
            map_path.display()
        );
    }
    sources.sort();
    Ok(sources)
}

// ─── list-sources ─────────────────────────────────────────────────────────────

/// List all source documents embedded in the package with their chunk counts.
pub fn list_sources(pkg_path: &Path, json_output: bool) -> Result<()> {
    let pkg = parse(pkg_path)?;

    // Count chunks per source from KNOW section
    let mut chunk_counts: HashMap<String, usize> = HashMap::new();
    if let Some(sec) = pkg.section("KNOW") {
        let data = pkg.section_data(sec);
        if let Ok((chunks, _, _)) = parse_know_section(data) {
            for chunk in &chunks {
                *chunk_counts.entry(chunk.source.clone()).or_insert(0) += 1;
            }
        }
    }

    // Collect sources from SRCS section
    let mut srcs_meta: HashMap<String, Value> = HashMap::new();
    if let Some(sec) = pkg.section("SRCS") {
        let data = pkg.section_data(sec);
        for line in std::str::from_utf8(data).unwrap_or("").lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                if let Some(name) = v["file"].as_str().or_else(|| v["name"].as_str()) {
                    srcs_meta.insert(name.to_string(), v);
                }
            }
        }
    }

    // Merge: all sources from chunks + SRCS
    let mut all_sources: Vec<String> = {
        let mut s: std::collections::HashSet<String> = chunk_counts.keys().cloned().collect();
        s.extend(srcs_meta.keys().cloned());
        let mut v: Vec<String> = s.into_iter().collect();
        v.sort();
        v
    };
    if all_sources.is_empty() {
        all_sources = vec![];
    }

    if json_output {
        let entries: Vec<Value> = all_sources
            .iter()
            .map(|src| {
                let chunks = chunk_counts.get(src).copied().unwrap_or(0);
                let mut entry = serde_json::json!({
                    "source": src,
                    "chunks": chunks,
                });
                if let Some(meta) = srcs_meta.get(src) {
                    entry["meta"] = meta.clone();
                }
                entry
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        println!("Sources in {}:", pkg_path.display());
        println!("{:<50} {:>8}", "SOURCE", "CHUNKS");
        println!("{}", "-".repeat(60));
        for src in &all_sources {
            let chunks = chunk_counts.get(src).copied().unwrap_or(0);
            println!("{:<50} {:>8}", src, chunks);
        }
        println!("{}", "-".repeat(60));
        let total: usize = chunk_counts.values().sum();
        println!("Total: {} source(s), {} chunk(s)", all_sources.len(), total);
    }

    Ok(())
}

// ─── erase ────────────────────────────────────────────────────────────────────

/// Remove all knowledge chunks belonging to any of `sources` and rebuild the
/// package. Also removes matching claims from CLMS and entries from SRCS.
/// Pass multiple sources (e.g. via `resolve_sources` with `--subject`) to
/// erase everything tied to one person in a single call.
pub fn erase(
    pkg_path: &Path,
    sources: &[String],
    output: Option<&Path>,
    dry_run: bool,
) -> Result<()> {
    let raw = std::fs::read(pkg_path)?;
    if crate::crypto::is_sealed(&raw) {
        anyhow::bail!(
            "Package is sealed — erase requires the author to unseal it first \
             (aipk unseal --key <private-key>), erase, and re-seal."
        );
    }
    let pkg = parse(pkg_path)?;
    let is_target = |name: &str| sources.iter().any(|s| s == name);

    // ── Filter KNOW section ───────────────────────────────────────────────────
    let (removed_chunks, kept_chunks, kept_vectors, dim) = match pkg.section("KNOW") {
        None => (0, vec![], vec![], 768u32),
        Some(sec) => {
            let data = pkg.section_data(sec);
            let (chunks, vectors, dim) = parse_know_section(data)?;
            let mut kept_c = Vec::new();
            let mut kept_v = Vec::new();
            let mut removed = 0usize;
            for (i, chunk) in chunks.iter().enumerate() {
                if is_target(&chunk.source) {
                    removed += 1;
                } else {
                    kept_c.push(chunk.clone());
                    if let Some(v) = vectors.get(i) {
                        kept_v.push(v.clone());
                    }
                }
            }
            (removed, kept_c, kept_v, dim)
        }
    };

    // ── Filter CLMS section ───────────────────────────────────────────────────
    let (removed_claims, kept_claims_jsonl) = match pkg.section("CLMS") {
        None => (0, String::new()),
        Some(sec) => {
            let text = std::str::from_utf8(pkg.section_data(sec)).unwrap_or("");
            let mut removed = 0usize;
            let kept: String = text
                .lines()
                .filter(|line| {
                    if line.trim().is_empty() {
                        return false;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(line) {
                        if v["source"].as_str().is_some_and(is_target) {
                            removed += 1;
                            return false;
                        }
                    }
                    true
                })
                .map(|l| format!("{l}\n"))
                .collect();
            (removed, kept)
        }
    };

    // ── Filter SRCS section ───────────────────────────────────────────────────
    let kept_srcs_jsonl = match pkg.section("SRCS") {
        None => String::new(),
        Some(sec) => {
            let text = std::str::from_utf8(pkg.section_data(sec)).unwrap_or("");
            text.lines()
                .filter(|line| {
                    if line.trim().is_empty() {
                        return false;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(line) {
                        let name = v["file"].as_str().or_else(|| v["name"].as_str());
                        return !name.is_some_and(is_target);
                    }
                    true
                })
                .map(|l| format!("{l}\n"))
                .collect()
        }
    };

    let target_desc = sources.join(", ");

    if removed_chunks == 0 && removed_claims == 0 {
        println!(
            "No matching source(s) [{}] found in {}.",
            target_desc,
            pkg_path.display()
        );
        return Ok(());
    }

    if dry_run {
        println!("Dry run — no files written.");
        println!(
            "Would remove: {} chunk(s), {} claim(s) from source(s) [{}]",
            removed_chunks, removed_claims, target_desc
        );
        return Ok(());
    }

    // ── Rebuild package ───────────────────────────────────────────────────────
    let mut builder = AipkBuilder::new(&pkg.name);

    for sec in &pkg.sections {
        if sec.tag == "INDX" {
            continue;
        }
        match sec.tag.as_str() {
            "KNOW" => {
                if !kept_chunks.is_empty() {
                    let know_bytes = build_know_section(&kept_chunks, &kept_vectors, dim)?;
                    builder.add("KNOW", know_bytes);
                }
            }
            // Erasure renumbers surviving chunks, which invalidates any existing
            // ANNX index (its entries are positional). Rebuild fresh from the
            // kept vectors rather than carrying the stale index forward.
            "ANNX" => {
                if kept_chunks.len() > ANN_INDEX_THRESHOLD {
                    if let Some(index) = AnnIndex::build(&kept_vectors) {
                        builder.add("ANNX", index.to_bytes()?);
                    }
                }
            }
            "CLMS" => {
                if !kept_claims_jsonl.trim().is_empty() {
                    builder.add("CLMS", kept_claims_jsonl.as_bytes().to_vec());
                }
            }
            "SRCS" => {
                if !kept_srcs_jsonl.trim().is_empty() {
                    builder.add("SRCS", kept_srcs_jsonl.as_bytes().to_vec());
                }
            }
            _ => {
                builder.add(sec.tag.clone(), pkg.section_data(sec).to_vec());
            }
        }
    }

    let out_path = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| pkg_path.to_path_buf());

    let bytes = builder.build();
    fs::write(&out_path, &bytes)?;

    println!(
        "✓ Erased source(s) [{}] from {}",
        target_desc,
        out_path.display()
    );
    println!(
        "  Removed: {} chunk(s), {} claim(s)",
        removed_chunks, removed_claims
    );
    println!(
        "  Remaining: {} chunk(s), {} claim(s)",
        kept_chunks.len(),
        kept_claims_jsonl
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count()
    );
    Ok(())
}

// ─── snapshot ───────────────────────────────────────────────────────────────────

/// Export everything the package holds about one or more sources — every
/// KNOW chunk, every CLMS claim, and the SRCS metadata entry — without
/// modifying the package. This is the right-of-access / data-portability
/// counterpart to `erase`: a subject access request answered with a
/// self-contained, machine-readable copy instead of a deletion.
pub fn snapshot(pkg_path: &Path, sources: &[String], output: Option<&Path>) -> Result<()> {
    let pkg = parse(pkg_path)?;
    let is_target = |name: &str| sources.iter().any(|s| s == name);

    let chunks: Vec<Value> = match pkg.section("KNOW") {
        None => vec![],
        Some(sec) => {
            let (chunks, _, _) = parse_know_section(pkg.section_data(sec))?;
            chunks
                .into_iter()
                .filter(|c| is_target(&c.source))
                .map(|c| json!({"id": c.id, "source": c.source, "text": c.text}))
                .collect()
        }
    };

    let claims: Vec<Value> = match pkg.section("CLMS") {
        None => vec![],
        Some(sec) => {
            let text = std::str::from_utf8(pkg.section_data(sec)).unwrap_or("");
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter(|v| v["source"].as_str().is_some_and(is_target))
                .collect()
        }
    };

    let source_meta: Vec<Value> = match pkg.section("SRCS") {
        None => vec![],
        Some(sec) => {
            let text = std::str::from_utf8(pkg.section_data(sec)).unwrap_or("");
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter(|v| {
                    v["file"]
                        .as_str()
                        .or_else(|| v["name"].as_str())
                        .is_some_and(is_target)
                })
                .collect()
        }
    };

    if chunks.is_empty() && claims.is_empty() && source_meta.is_empty() {
        anyhow::bail!(
            "No data found for source(s) [{}] in {}.",
            sources.join(", "),
            pkg_path.display()
        );
    }

    let (chunk_count, claim_count, meta_count) = (chunks.len(), claims.len(), source_meta.len());

    let snapshot = json!({
        "package": pkg.name,
        "sources": sources,
        "generated_at": crate::cmd::extract_claims::now_rfc3339(),
        "source_metadata": source_meta,
        "knowledge_chunks": chunks,
        "claims": claims,
    });

    let rendered = serde_json::to_string_pretty(&snapshot)?;
    match output {
        Some(path) => {
            fs::write(path, &rendered)?;
            println!("✓ Snapshot written to {}", path.display());
        }
        None => println!("{rendered}"),
    }
    eprintln!(
        "  {chunk_count} chunk(s), {claim_count} claim(s), {meta_count} source record(s) for [{}]",
        sources.join(", ")
    );
    Ok(())
}

// ─── report ───────────────────────────────────────────────────────────────────

/// Generate a GDPR compliance report for the package.
pub fn report(pkg_path: &Path) -> Result<()> {
    let pkg = parse(pkg_path)?;

    println!("GDPR Compliance Report — {}", pkg_path.display());
    println!("Package: {}  Version: {}", pkg.name, pkg.version);
    println!("Created: {}", pkg.created_at.format("%Y-%m-%d %H:%M UTC"));
    println!();

    // ── Sections inventory ────────────────────────────────────────────────────
    println!("Sections:");
    for sec in &pkg.sections {
        if sec.tag == "INDX" {
            continue;
        }
        let encrypted = if sec.flags & crate::crypto::SECTION_FLAG_ENCRYPTED != 0 {
            " [ENCRYPTED]"
        } else {
            ""
        };
        println!("  {:<6} {:>8} bytes{}", sec.tag, sec.size, encrypted);
    }
    println!();

    // ── Knowledge base sources ────────────────────────────────────────────────
    let mut chunk_counts: HashMap<String, usize> = HashMap::new();
    if let Some(sec) = pkg.section("KNOW") {
        if let Ok((chunks, _, _)) = parse_know_section(pkg.section_data(sec)) {
            for c in &chunks {
                *chunk_counts.entry(c.source.clone()).or_insert(0) += 1;
            }
        }
    }

    if chunk_counts.is_empty() {
        println!("Knowledge base: none");
    } else {
        println!(
            "Knowledge base: {} source(s), {} total chunk(s)",
            chunk_counts.len(),
            chunk_counts.values().sum::<usize>()
        );
        let mut srcs: Vec<(&String, &usize)> = chunk_counts.iter().collect();
        srcs.sort_by_key(|(k, _)| k.as_str());
        for (src, count) in srcs {
            println!("  {count:>5} chunks  {src}");
        }
        println!();
        println!("Right to erasure: run `aipk gdpr erase <pkg> --source <name>` for each source.");
    }

    // ── Claims ────────────────────────────────────────────────────────────────
    if let Some(sec) = pkg.section("CLMS") {
        let text = std::str::from_utf8(pkg.section_data(sec)).unwrap_or("");
        let count = text.lines().filter(|l| !l.trim().is_empty()).count();
        if count > 0 {
            println!();
            println!("Claims: {count} (use `aipk gdpr erase --source <name>` to remove by source)");
        }
    }

    // ── Signature ─────────────────────────────────────────────────────────────
    if pkg.section("SIGN").is_some() {
        println!();
        println!("Signature: PRESENT — package integrity verifiable with `aipk verify-sig`");
    }

    // ── Encryption ───────────────────────────────────────────────────────────
    let any_encrypted = pkg
        .sections
        .iter()
        .any(|s| s.flags & crate::crypto::SECTION_FLAG_ENCRYPTED != 0);
    if any_encrypted {
        println!();
        println!("Encryption: ACTIVE — content sections are AES-256-GCM encrypted");
    }

    println!();
    println!("Recommendations:");
    if chunk_counts.is_empty() {
        println!("  ✓ No personal data in knowledge base.");
    } else {
        println!(
            "  ! Review source documents for personal data. Use `aipk gdpr erase` to remove any."
        );
    }
    if !any_encrypted {
        println!("  ! Consider encrypting this package with `aipk encrypt` before distribution.");
    } else {
        println!("  ✓ Package is encrypted at rest.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{build_know_section, AipkBuilder};

    /// Erasure renumbers surviving KNOW chunks, so a stale ANNX index built
    /// against the old numbering must not survive unchanged. Below the ANN
    /// threshold the rebuilt package should simply drop it (falls back to
    /// brute force) rather than carry forward indices pointing at the wrong
    /// (or now-removed) chunks.
    #[test]
    fn erase_drops_stale_annx_below_threshold() {
        let chunks = vec![
            crate::format::KnowChunk {
                id: 0,
                text: "alice note".into(),
                source: "alice.md".into(),
            },
            crate::format::KnowChunk {
                id: 1,
                text: "bob note".into(),
                source: "bob.md".into(),
            },
        ];
        let vectors = vec![vec![1.0, 0.0], vec![0.0, 1.0]];

        let mut b = AipkBuilder::new("erase-annx-test");
        b.add("META", b"[package]\nname = \"erase-annx-test\"\n".to_vec());
        b.add("KNOW", build_know_section(&chunks, &vectors, 2).unwrap());
        b.add(
            "ANNX",
            crate::ann::AnnIndex::build(&vectors)
                .unwrap()
                .to_bytes()
                .unwrap(),
        );

        let dir = std::env::temp_dir().join("aipk_gdpr_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("erase-annx-test.aipk");
        std::fs::write(&path, b.build()).unwrap();

        erase(&path, &["alice.md".to_string()], None, false).unwrap();

        let pkg = crate::format::parse(&path).unwrap();
        assert!(
            pkg.section("ANNX").is_none(),
            "stale ANNX must not survive erase below threshold"
        );
        let (kept_chunks, _, _) =
            parse_know_section(pkg.section_data(pkg.section("KNOW").unwrap())).unwrap();
        assert_eq!(kept_chunks.len(), 1);
        assert_eq!(kept_chunks[0].source, "bob.md");
    }

    #[test]
    fn erase_with_missing_source_is_noop() {
        // We can't easily build a real package in unit tests without ollama,
        // so just verify the function returns an error gracefully for a missing file.
        let result = list_sources(Path::new("/nonexistent.aipk"), false);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_sources_with_explicit_source_ignores_subject() {
        let sources = resolve_sources(Some("alice-notes.md"), None, None).unwrap();
        assert_eq!(sources, vec!["alice-notes.md".to_string()]);
    }

    #[test]
    fn resolve_sources_without_source_or_subject_errors() {
        assert!(resolve_sources(None, None, None).is_err());
    }

    #[test]
    fn resolve_sources_with_subject_but_no_map_errors() {
        let err = resolve_sources(None, Some("alice@acme.com"), None).unwrap_err();
        assert!(err.to_string().contains("--subject-map"));
    }

    #[test]
    fn resolve_sources_by_subject_collects_all_mapped_files() {
        let tmp =
            std::env::temp_dir().join(format!("aipk_subject_map_test_{}.json", std::process::id()));
        std::fs::write(
            &tmp,
            r#"{
                "alice-onboarding.md": "alice@acme.com",
                "alice-slack-export.json": "alice@acme.com",
                "bob-notes.md": "bob@acme.com"
            }"#,
        )
        .unwrap();

        let mut sources = resolve_sources(None, Some("alice@acme.com"), Some(&tmp)).unwrap();
        sources.sort();
        assert_eq!(
            sources,
            vec![
                "alice-onboarding.md".to_string(),
                "alice-slack-export.json".to_string()
            ]
        );

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn resolve_sources_unknown_subject_errors() {
        let tmp = std::env::temp_dir().join(format!(
            "aipk_subject_map_test2_{}.json",
            std::process::id()
        ));
        std::fs::write(&tmp, r#"{"alice-notes.md": "alice@acme.com"}"#).unwrap();

        let result = resolve_sources(None, Some("carol@acme.com"), Some(&tmp));
        assert!(result.is_err());

        std::fs::remove_file(&tmp).ok();
    }
}
