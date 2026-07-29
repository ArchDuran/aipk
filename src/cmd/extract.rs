use anyhow::Result;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::format::{parse, parse_know_section};

pub fn run(pkg_path: &Path, output_dir: &Path) -> Result<()> {
    let raw = fs::read(pkg_path)?;
    if crate::crypto::is_sealed(&raw) {
        anyhow::bail!(
            "Package is sealed by its author — contents cannot be extracted.\n\
             If you are the author: aipk unseal {} --key <your-private-key>",
            pkg_path.display()
        );
    }
    let pkg = parse(pkg_path)?;

    fs::create_dir_all(output_dir)?;

    let mut extracted: Vec<String> = Vec::new();

    // META → manifest.toml
    if let Some(sec) = pkg.section("META") {
        let data = pkg.section_data(sec);
        fs::write(output_dir.join("manifest.toml"), data)?;
        extracted.push("manifest.toml".to_string());
    }

    // PERS → persona.md
    if let Some(sec) = pkg.section("PERS") {
        let data = pkg.section_data(sec);
        fs::write(output_dir.join("persona.md"), data)?;
        extracted.push("persona.md".to_string());
    }

    // TOOL → tools.json
    if let Some(sec) = pkg.section("TOOL") {
        let data = pkg.section_data(sec);
        fs::write(output_dir.join("tools.json"), data)?;
        extracted.push("tools.json".to_string());
    }

    // LICN → license.toml
    if let Some(sec) = pkg.section("LICN") {
        let data = pkg.section_data(sec);
        fs::write(output_dir.join("license.toml"), data)?;
        extracted.push("license.toml".to_string());
    }

    // THKG → thkg.md
    if let Some(sec) = pkg.section("THKG") {
        let data = pkg.section_data(sec);
        fs::write(output_dir.join("thkg.md"), data)?;
        extracted.push("thkg.md".to_string());
    }

    // LINK → links.json
    if let Some(sec) = pkg.section("LINK") {
        let data = pkg.section_data(sec);
        fs::write(output_dir.join("links.json"), data)?;
        extracted.push("links.json".to_string());
    }

    // CLMS → claims.jsonl
    if let Some(sec) = pkg.section("CLMS") {
        let data = pkg.section_data(sec);
        fs::write(output_dir.join("claims.jsonl"), data)?;
        let count = std::str::from_utf8(data)
            .map(|s| s.lines().filter(|l| !l.is_empty()).count())
            .unwrap_or(0);
        extracted.push(format!("claims.jsonl ({} claims)", count));
    }

    // SRCS → sources.jsonl
    if let Some(sec) = pkg.section("SRCS") {
        let data = pkg.section_data(sec);
        fs::write(output_dir.join("sources.jsonl"), data)?;
        extracted.push("sources.jsonl".to_string());
    }

    // SKIL → skills/ directory (reconstruct .md files from binary format)
    if let Some(_sec) = pkg.section("SKIL") {
        let skills = pkg.skills();
        if !skills.is_empty() {
            let skills_dir = output_dir.join("skills");
            fs::create_dir_all(&skills_dir)?;
            for skill in &skills {
                let filename = format!("{}.md", skill.name.to_lowercase().replace(' ', "-"));
                fs::write(skills_dir.join(&filename), &skill.content)?;
            }
            extracted.push(format!("skills/ ({} files)", skills.len()));
        }
    }

    // KNOW → .aipk/ cache (chunks.jsonl + vectors.bin + state.json)
    if let Some(sec) = pkg.section("KNOW") {
        let data = pkg.section_data(sec);
        match parse_know_section(data) {
            Ok((chunks, vectors, dim)) => {
                let cache_dir = output_dir.join(".aipk");
                fs::create_dir_all(&cache_dir)?;

                // chunks.jsonl
                let chunks_file = fs::File::create(cache_dir.join("chunks.jsonl"))?;
                let mut chunks_w = BufWriter::new(chunks_file);
                for chunk in &chunks {
                    let line = serde_json::to_string(&serde_json::json!({
                        "id": chunk.id,
                        "text": chunk.text,
                        "source": chunk.source,
                    }))?;
                    writeln!(chunks_w, "{}", line)?;
                }
                chunks_w.flush()?;

                // vectors.bin
                let mut vec_bytes: Vec<u8> = Vec::with_capacity(vectors.len() * dim as usize * 4);
                for v in &vectors {
                    for f in v {
                        vec_bytes.extend_from_slice(&f.to_le_bytes());
                    }
                }
                fs::write(cache_dir.join("vectors.bin"), &vec_bytes)?;

                // state.json
                let state = serde_json::json!({
                    "embedding_dim": dim,
                    "chunk_count": chunks.len(),
                });
                fs::write(
                    cache_dir.join("state.json"),
                    serde_json::to_string_pretty(&state)?,
                )?;

                extracted.push(format!(".aipk/ ({} chunks, dim={})", chunks.len(), dim));
            }
            Err(e) => eprintln!("  warning: could not extract KNOW section: {e}"),
        }
    }

    println!("✓ Extracted '{}' → {}/", pkg.name, output_dir.display());
    for item in &extracted {
        println!("  {}", item);
    }
    if extracted.is_empty() {
        println!("  (no sections found)");
    }

    Ok(())
}
