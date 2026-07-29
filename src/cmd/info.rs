use crate::format::parse;
use anyhow::Result;
use colored::Colorize;
use std::path::Path;

pub fn run(path: &Path) -> Result<()> {
    let raw = std::fs::read(path)?;
    if crate::crypto::is_sealed(&raw) {
        return run_sealed(path, raw);
    }
    let pkg = parse(path)?;
    let meta = pkg.meta()?;

    let pkg_meta = meta.get("package");
    let name = pkg_meta
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(&pkg.name);
    let version = pkg_meta
        .and_then(|m| m.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let desc = pkg_meta
        .and_then(|m| m.get("description"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let author = pkg_meta
        .and_then(|m| m.get("author"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    println!("{}", format!("╭─  {} v{}  ─╮", name, version).bold());
    if !desc.is_empty() {
        println!("  Description: {desc}");
    }
    if !author.is_empty() {
        println!("  Author:      {author}");
    }
    println!(
        "  Created:     {}",
        pkg.created_at.format("%Y-%m-%d %H:%M UTC")
    );
    println!("  Format v:    {}", pkg.version);
    println!();

    println!("{}", "Sections:".bold());
    for sec in &pkg.sections {
        if sec.tag == "INDX" {
            continue;
        }
        let size = format_size(sec.size);
        let detail = section_detail(&pkg, &sec.tag);
        println!("  {:6}  {:>10}  {}", sec.tag.cyan(), size, detail.dimmed());
    }

    print_license(&pkg);

    let model_meta = meta.get("model");
    let compatible: Vec<&str> = model_meta
        .and_then(|m| m.get("compatible"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let min_ctx = model_meta
        .and_then(|m| m.get("min_context"))
        .and_then(|v| v.as_integer())
        .unwrap_or(0);
    let emb = model_meta
        .and_then(|m| m.get("embedding_model"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    println!();
    let compat = if compatible.is_empty() {
        "any".to_string()
    } else {
        compatible.join(", ")
    };
    println!(
        "Model: compatible={}  min_context={}  embedding={}",
        compat, min_ctx, emb
    );

    let graph = meta.get("graph");
    let role = graph
        .and_then(|g| g.get("role"))
        .and_then(|v| v.as_str())
        .unwrap_or("specialist");
    let depth = graph
        .and_then(|g| g.get("depth"))
        .and_then(|v| v.as_integer())
        .unwrap_or(2);
    let max_nodes = graph
        .and_then(|g| g.get("max_nodes"))
        .and_then(|v| v.as_integer())
        .unwrap_or(5);

    let consol = meta.get("consolidation");
    let strategy = consol
        .and_then(|c| c.get("strategy"))
        .and_then(|v| v.as_str())
        .unwrap_or("idle");
    let idle_min = consol
        .and_then(|c| c.get("idle_min"))
        .and_then(|v| v.as_integer())
        .unwrap_or(30);

    println!(
        "Graph:  role={}  depth={}  max_nodes={}  consolidation={} (idle={}min)",
        role, depth, max_nodes, strategy, idle_min
    );

    Ok(())
}

/// Sealed packages show only what is deliberately public: META, LICN, the
/// section inventory, and the signature status. No content previews.
fn run_sealed(_path: &Path, raw: Vec<u8>) -> Result<()> {
    let pkg = crate::format::parse_bytes(raw.clone())?;

    println!("{}", format!("╭─  {}  ─╮", pkg.name).bold());
    println!(
        "  Status:      {} (contents locked by the author)",
        "SEALED".yellow().bold()
    );
    println!(
        "  Created:     {}",
        pkg.created_at.format("%Y-%m-%d %H:%M UTC")
    );

    match crate::cmd::sign::verify_sig_bytes(&raw) {
        Ok(_) => println!("  Signature:   {} (Ed25519)", "valid".green()),
        Err(e) => println!("  Signature:   {} — {e}", "INVALID".red().bold()),
    }

    // META is always plaintext
    if let Ok(meta) = pkg.meta() {
        let pkg_meta = meta.get("package");
        if let Some(author) = pkg_meta
            .and_then(|m| m.get("author"))
            .and_then(|v| v.as_str())
        {
            println!("  Author:      {author}");
        }
        if let Some(desc) = pkg_meta
            .and_then(|m| m.get("description"))
            .and_then(|v| v.as_str())
        {
            if !desc.is_empty() {
                println!("  Description: {desc}");
            }
        }
    }

    print_license(&pkg);

    println!();
    println!("{}", "Sections:".bold());
    for sec in &pkg.sections {
        if sec.tag == "INDX" {
            continue;
        }
        let lock = if sec.flags & crate::crypto::SECTION_FLAG_ENCRYPTED != 0 {
            "encrypted".dimmed()
        } else {
            "plaintext".dimmed()
        };
        println!(
            "  {:6}  {:>10}  {}",
            sec.tag.cyan(),
            format_size(sec.size),
            lock
        );
    }
    println!();
    println!("Use `aipk serve` / `aipk run` normally — the runtime opens sealed packages.");
    println!("`aipk extract` and `aipk export` require the author to unseal first.");
    Ok(())
}

fn print_license(pkg: &crate::format::AipkPackage) {
    let Some(sec) = pkg.section("LICN") else {
        return;
    };
    let Ok(text) = std::str::from_utf8(pkg.section_data(sec)) else {
        return;
    };
    let Ok(licn) = toml::from_str::<toml::Value>(text) else {
        return;
    };
    let l = licn.get("license").unwrap_or(&licn);
    println!();
    println!("{}", "License:".bold());
    for key in ["author", "license", "copyright", "terms", "contact"] {
        if let Some(v) = l.get(key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                println!("  {:12} {}", format!("{key}:"), v);
            }
        }
    }
    for key in ["allow_derivatives", "allow_redistribution"] {
        if let Some(v) = l.get(key).and_then(|v| v.as_bool()) {
            println!("  {:12} {}", format!("{key}:"), v);
        }
    }
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn section_detail(pkg: &crate::format::AipkPackage, tag: &str) -> String {
    match tag {
        "PERS" => {
            let text = pkg.persona().unwrap_or_default();
            let preview: String = text.chars().take(60).collect();
            format!("{}…", preview.replace('\n', " "))
        }
        "SKIL" => {
            let skills = pkg.skills();
            format!(
                "{} skill(s): {}",
                skills.len(),
                skills
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        "TOOL" => {
            if let Some(tools) = pkg.tools_json() {
                let count = tools["mcp_servers"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                format!("{} MCP server(s)", count)
            } else {
                "MCP configs".to_string()
            }
        }
        "KNOW" => "knowledge base (chunks + embeddings)".to_string(),
        "ANNX" => "pre-built HNSW index for KNOW retrieval".to_string(),
        "LINK" => {
            let data = pkg
                .section("LINK")
                .map(|s| pkg.section_data(s))
                .unwrap_or(&[]);
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
                let count = v["links"].as_array().map(|a| a.len()).unwrap_or(0);
                let hint = v["router_hint"].as_str().unwrap_or("");
                if hint.is_empty() {
                    format!("graph links  edges={count}")
                } else {
                    let preview: String = hint.chars().take(50).collect();
                    format!("graph links  edges={count}  hint={preview}…")
                }
            } else {
                "graph links".to_string()
            }
        }
        "THKG" => {
            let data = pkg
                .section("THKG")
                .map(|s| pkg.section_data(s))
                .unwrap_or(&[]);
            let text = std::str::from_utf8(data).unwrap_or("");
            let preview: String = text.chars().take(80).collect();
            let preview = preview.replace('\n', " ");
            format!("router description  {preview}…")
        }
        "IDTY" => {
            let data = pkg
                .section("IDTY")
                .map(|s| pkg.section_data(s))
                .unwrap_or(&[]);
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
                let name = v["name"].as_str().unwrap_or("?");
                let role = v["role"].as_str().unwrap_or("?");
                format!("identity contract  name={name}  role={role}")
            } else {
                "identity contract".to_string()
            }
        }
        "ANSP" => {
            let data = pkg
                .section("ANSP")
                .map(|s| pkg.section_data(s))
                .unwrap_or(&[]);
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
                let threshold = v["confidence_threshold"].as_f64().unwrap_or(0.0);
                let domain_count = v["domain"].as_array().map(|a| a.len()).unwrap_or(0);
                format!(
                    "answerability gate  domain_keywords={domain_count}  threshold={threshold:.2}"
                )
            } else {
                "answerability gate".to_string()
            }
        }
        "PLCY" => {
            let data = pkg
                .section("PLCY")
                .map(|s| pkg.section_data(s))
                .unwrap_or(&[]);
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
                let citation = v["citation_required"].as_bool().unwrap_or(false);
                let forbidden = v["forbidden"].as_array().map(|a| a.len()).unwrap_or(0);
                format!("answer policy  citation_required={citation}  forbidden_rules={forbidden}")
            } else {
                "answer policy".to_string()
            }
        }
        "NKNW" => {
            let data = pkg
                .section("NKNW")
                .map(|s| pkg.section_data(s))
                .unwrap_or(&[]);
            let count = std::str::from_utf8(data)
                .unwrap_or("")
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count();
            format!("negative knowledge  entries={count}")
        }
        "CLMS" => "epistemic claims".to_string(),
        "LICN" => "license / copyright terms".to_string(),
        "SEAL" => "seal salt (author-locked package)".to_string(),
        "SIGN" => "Ed25519 signature".to_string(),
        _ => String::new(),
    }
}
