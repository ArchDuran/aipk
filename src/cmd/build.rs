use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::ann::{AnnIndex, ANN_INDEX_THRESHOLD};
use crate::format::{
    build_know_section, build_skil_section, parse_frontmatter_trigger, AipkBuilder, KnowChunk,
    SkillDef,
};

pub fn run(project_dir: &Path, output: Option<&Path>) -> Result<()> {
    let manifest_path = project_dir.join("manifest.toml");
    if !manifest_path.exists() {
        anyhow::bail!("manifest.toml not found in {}", project_dir.display());
    }

    let manifest_toml = fs::read_to_string(&manifest_path)?;
    let meta: toml::Value = toml::from_str(&manifest_toml)?;
    let name = meta["package"]["name"]
        .as_str()
        .unwrap_or("unnamed")
        .to_string();

    let mut builder = AipkBuilder::new(&name);

    // META
    builder.add("META", manifest_toml.into_bytes());

    // PERS
    let persona_file = project_dir.join("persona.md");
    if persona_file.exists() {
        let text = fs::read_to_string(&persona_file)?;
        if !text.trim().is_empty() {
            builder.add("PERS", text.into_bytes());
        }
    }

    // SKIL
    let skills_dir = project_dir.join("skills");
    if skills_dir.exists() {
        let mut skills: Vec<SkillDef> = Vec::new();
        let mut entries: Vec<_> = fs::read_dir(&skills_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                n.ends_with(".md") && n != "readme.md"
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let content = fs::read_to_string(entry.path())?;
            let filename = entry.file_name().to_string_lossy().to_string();
            let trigger = parse_frontmatter_trigger(&content);
            let skill_name = frontmatter_field(&content, "name").unwrap_or_else(|| {
                Path::new(&filename)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
            skills.push(SkillDef {
                name: skill_name,
                filename,
                trigger,
                content,
            });
        }

        if !skills.is_empty() {
            builder.add("SKIL", build_skil_section(&skills));
        }
    }

    // TOOL
    let tools_file = project_dir.join("tools.json");
    if tools_file.exists() {
        let tools_json = fs::read_to_string(&tools_file)?;
        let tools: serde_json::Value =
            serde_json::from_str(&tools_json).context("tools.json is invalid JSON")?;
        let has_servers = tools["mcp_servers"]
            .as_array()
            .is_some_and(|a| !a.is_empty());
        let has_tools = tools["tools"].as_array().is_some_and(|a| !a.is_empty());
        if has_servers || has_tools {
            builder.add("TOOL", tools_json.into_bytes());
        }
    }

    // THKG — router description for graph mode (thkg.md)
    let thkg_file = project_dir.join("thkg.md");
    if thkg_file.exists() {
        let text = fs::read_to_string(&thkg_file)?;
        if !text.trim().is_empty() {
            builder.add("THKG", text.into_bytes());
            println!("  THKG: router description loaded");
        }
    }

    // LINK — graph edges (links.json)
    let links_file = project_dir.join("links.json");
    if links_file.exists() {
        let links_json = fs::read_to_string(&links_file)?;
        let links: serde_json::Value =
            serde_json::from_str(&links_json).context("links.json is invalid JSON")?;
        if links["links"].as_array().is_some_and(|a| !a.is_empty()) {
            let link_count = links["links"].as_array().map(|a| a.len()).unwrap_or(0);
            builder.add("LINK", links_json.into_bytes());
            println!("  LINK: {link_count} graph edge(s)");
        }
    }

    // CLMS — from claims.jsonl (populated by `aipk extract-claims`)
    let claims_file = project_dir.join("claims.jsonl");
    if claims_file.exists() {
        let content = fs::read_to_string(&claims_file)?;
        let count = content.lines().filter(|l| !l.is_empty()).count();
        if count > 0 {
            builder.add("CLMS", content.into_bytes());
            println!("  CLMS: {} claims", count);
        }
    }

    // SRCS — from sources.jsonl (populated by `aipk extract-claims`)
    let sources_file = project_dir.join("sources.jsonl");
    if sources_file.exists() {
        let content = fs::read_to_string(&sources_file)?;
        if !content.trim().is_empty() {
            builder.add("SRCS", content.into_bytes());
        }
    }

    // KNOW + CLMV — from .aipk/ cache (populated by `aipk add-docs` / `aipk pipeline`)
    let cache_dir = project_dir.join(".aipk");
    let state_file = cache_dir.join("state.json");
    let chunks_file = cache_dir.join("chunks.jsonl");
    let vectors_file = cache_dir.join("vectors.bin");

    if state_file.exists() && chunks_file.exists() && vectors_file.exists() {
        let state: serde_json::Value = serde_json::from_str(&fs::read_to_string(&state_file)?)?;
        let dim = state["embedding_dim"].as_u64().unwrap_or(0) as u32;
        let chunk_count = state["chunk_count"].as_u64().unwrap_or(0) as usize;

        if dim > 0 && chunk_count > 0 {
            let chunks_jsonl = fs::read_to_string(&chunks_file)?;
            let chunks: Vec<KnowChunk> = chunks_jsonl
                .lines()
                .filter(|l| !l.is_empty())
                .filter_map(|line| {
                    let v: serde_json::Value = serde_json::from_str(line).ok()?;
                    Some(KnowChunk {
                        id: v["id"].as_u64().unwrap_or(0) as u32,
                        text: v["text"].as_str().unwrap_or("").to_string(),
                        source: v["source"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect();

            let vec_raw = fs::read(&vectors_file)?;
            let vectors: Vec<Vec<f32>> = (0..chunks.len())
                .map(|i| {
                    let base = i * dim as usize * 4;
                    (0..dim as usize)
                        .map(|j| {
                            let off = base + j * 4;
                            f32::from_le_bytes(vec_raw[off..off + 4].try_into().unwrap())
                        })
                        .collect()
                })
                .collect();

            let know_bytes = build_know_section(&chunks, &vectors, dim)?;
            builder.add("KNOW", know_bytes);
            println!("  KNOW: {} chunks, dim={}", chunks.len(), dim);

            // ANNX — pre-built HNSW index, only above the size where brute-force
            // retrieval stops being comfortable (see ann.rs module docs for why
            // this is built here, once, rather than on every load).
            if chunks.len() > ANN_INDEX_THRESHOLD {
                println!(
                    "  ANNX: building HNSW index for {} chunks (this can take a while)...",
                    chunks.len()
                );
                if let Some(index) = AnnIndex::build(&vectors) {
                    builder.add("ANNX", index.to_bytes()?);
                    println!("  ANNX: index built and embedded");
                }
            }
        }
    }

    // CLMV — claim vectors for semantic matching (populated by `aipk pipeline`)
    let claim_vectors_file = cache_dir.join("claim_vectors.bin");
    if claim_vectors_file.exists() && state_file.exists() {
        let state: serde_json::Value = serde_json::from_str(&fs::read_to_string(&state_file)?)?;
        let claim_count = state["claim_count"].as_u64().unwrap_or(0) as u32;
        let claim_dim = state["claim_dim"].as_u64().unwrap_or(0) as u32;
        if claim_count > 0 && claim_dim > 0 {
            let vec_bytes = fs::read(&claim_vectors_file)?;
            let expected = (claim_count * claim_dim * 4) as usize;
            if vec_bytes.len() >= expected {
                let mut clmv_data = Vec::with_capacity(8 + vec_bytes.len());
                clmv_data.extend_from_slice(&claim_count.to_le_bytes());
                clmv_data.extend_from_slice(&claim_dim.to_le_bytes());
                clmv_data.extend_from_slice(&vec_bytes);
                builder.add("CLMV", clmv_data);
                println!("  CLMV: {} claim vectors (dim={})", claim_count, claim_dim);
            }
        }
    }

    // TEST — from tests.json (optional CI harness)
    let tests_file = project_dir.join("tests.json");
    if tests_file.exists() {
        let json = fs::read_to_string(&tests_file)?;
        let parsed: serde_json::Value =
            serde_json::from_str(&json).context("tests.json is invalid JSON")?;
        let count = parsed["tests"]
            .as_array()
            .map(|a| a.len())
            .or_else(|| parsed.as_array().map(|a| a.len()))
            .unwrap_or(0);
        if count > 0 {
            builder.add("TEST", json.into_bytes());
            println!("  TEST: {count} test case(s)");
        }
    }

    // IDTY — identity contract (identity.json)
    let idty_file = project_dir.join("identity.json");
    if idty_file.exists() {
        let json = fs::read_to_string(&idty_file)?;
        let _: serde_json::Value =
            serde_json::from_str(&json).context("identity.json is invalid JSON")?;
        builder.add("IDTY", json.into_bytes());
        println!("  IDTY: identity contract loaded");
    }

    // ANSP — answerability policy (answerability.json)
    let ansp_file = project_dir.join("answerability.json");
    if ansp_file.exists() {
        let json = fs::read_to_string(&ansp_file)?;
        let _: serde_json::Value =
            serde_json::from_str(&json).context("answerability.json is invalid JSON")?;
        builder.add("ANSP", json.into_bytes());
        println!("  ANSP: answerability gate loaded");
    }

    // PLCY — answer policy (policy.json)
    let plcy_file = project_dir.join("policy.json");
    if plcy_file.exists() {
        let json = fs::read_to_string(&plcy_file)?;
        let _: serde_json::Value =
            serde_json::from_str(&json).context("policy.json is invalid JSON")?;
        builder.add("PLCY", json.into_bytes());
        println!("  PLCY: policy loaded");
    }

    // LICN — license / copyright terms (license.toml). Always stored in
    // plaintext (like META) so recipients can read the terms even in
    // encrypted or sealed packages.
    let license_file = project_dir.join("license.toml");
    if license_file.exists() {
        let text = fs::read_to_string(&license_file)?;
        let _: toml::Value = toml::from_str(&text).context("license.toml is invalid TOML")?;
        if !text.trim().is_empty() {
            builder.add("LICN", text.into_bytes());
            println!("  LICN: license terms loaded");
        }
    }

    // NKNW — negative knowledge registry (negative.jsonl)
    let nknw_file = project_dir.join("negative.jsonl");
    if nknw_file.exists() {
        let content = fs::read_to_string(&nknw_file)?;
        let count = content.lines().filter(|l| !l.trim().is_empty()).count();
        if count > 0 {
            builder.add("NKNW", content.into_bytes());
            println!("  NKNW: {count} negative knowledge entries");
        }
    }

    let bytes = builder.build();
    let out_path = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| project_dir.join(format!("{name}.aipk")));

    fs::write(&out_path, &bytes)?;

    let size = bytes.len();
    let size_str = if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    };

    println!("✓ Built {} ({})", out_path.display(), size_str);
    Ok(())
}

fn frontmatter_field(content: &str, field: &str) -> Option<String> {
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("---")? + 3;
    for line in content[3..end].lines() {
        if let Some(val) = line.strip_prefix(&format!("{field}:")) {
            return Some(val.trim().to_string());
        }
    }
    None
}
