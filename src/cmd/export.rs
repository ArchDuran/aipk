//! `aipk export` — unpack a package into the native layout of an agent
//! harness (Claude Code, OpenClaw, or a generic layout). The .aipk sections
//! map 1:1 onto harness concepts:
//!   PERS → system prompt file (CLAUDE.md / SOUL.md / persona.md)
//!   SKIL → skills/ markdown files
//!   KNOW → knowledge/ markdown files grouped by source
//!   TOOL → MCP server config
//! Claims, policies, and verification stay AIPK-only — harnesses have no
//! equivalent; serve the package with `aipk serve` to keep them.

use anyhow::{bail, Result};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

use crate::crypto::load_package;
use crate::format::{parse_know_section, AipkPackage};

pub fn run(
    pkg_path: &Path,
    target: &str,
    out_dir: Option<&Path>,
    passphrase: Option<&str>,
) -> Result<()> {
    let raw = fs::read(pkg_path)?;
    if crate::crypto::is_sealed(&raw) {
        bail!(
            "Package is sealed by its author — export is blocked.\n\
             If you are the author: aipk unseal {} --key <your-private-key>",
            pkg_path.display()
        );
    }

    let pkg = load_package(pkg_path, passphrase)?;
    let dir = out_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(format!("{}-{}", pkg.name, target)));
    fs::create_dir_all(&dir)?;

    let mut written: Vec<String> = Vec::new();

    match target {
        "claude-code" => {
            export_persona(&pkg, &dir.join("CLAUDE.md"), &mut written)?;
            export_skills(
                &pkg,
                &dir.join(".claude").join("skills"),
                true,
                &mut written,
            )?;
            export_mcp_json(&pkg, &dir.join(".mcp.json"), &mut written)?;
            export_knowledge(&pkg, &dir.join("knowledge"), &mut written)?;
        }
        "openclaw" => {
            export_persona(&pkg, &dir.join("SOUL.md"), &mut written)?;
            export_skills(&pkg, &dir.join("skills"), true, &mut written)?;
            export_mcp_json(&pkg, &dir.join("mcp-servers.json"), &mut written)?;
            export_knowledge(&pkg, &dir.join("knowledge"), &mut written)?;
        }
        "generic" => {
            export_persona(&pkg, &dir.join("persona.md"), &mut written)?;
            export_skills(&pkg, &dir.join("skills"), false, &mut written)?;
            export_mcp_json(&pkg, &dir.join("tools.json"), &mut written)?;
            export_knowledge(&pkg, &dir.join("knowledge"), &mut written)?;
        }
        other => {
            bail!("Unknown export target '{other}'. Available: claude-code, openclaw, generic")
        }
    }

    if written.is_empty() {
        bail!("Package has no exportable sections (PERS/SKIL/TOOL/KNOW).");
    }

    println!(
        "✓ Exported '{}' → {} ({} target)",
        pkg.name,
        dir.display(),
        target
    );
    for w in &written {
        println!("  {w}");
    }
    if pkg.section("CLMS").is_some() {
        println!(
            "  Note: claims/verification have no harness equivalent — use `aipk serve` \
             to keep grounded answers."
        );
    }
    Ok(())
}

fn export_persona(pkg: &AipkPackage, path: &Path, written: &mut Vec<String>) -> Result<()> {
    let Some(persona) = pkg.persona() else {
        return Ok(());
    };
    if persona.trim().is_empty() {
        return Ok(());
    }
    fs::write(path, persona)?;
    written.push(path.to_string_lossy().to_string());
    Ok(())
}

/// `folder_per_skill = true` writes skills/<slug>/SKILL.md (Claude Code /
/// OpenClaw convention); false writes skills/<slug>.md.
fn export_skills(
    pkg: &AipkPackage,
    skills_dir: &Path,
    folder_per_skill: bool,
    written: &mut Vec<String>,
) -> Result<()> {
    let skills = pkg.skills();
    if skills.is_empty() {
        return Ok(());
    }
    for skill in &skills {
        let slug = slugify(&skill.name);
        let path = if folder_per_skill {
            let d = skills_dir.join(&slug);
            fs::create_dir_all(&d)?;
            d.join("SKILL.md")
        } else {
            fs::create_dir_all(skills_dir)?;
            skills_dir.join(format!("{slug}.md"))
        };
        // Skill content already carries its frontmatter (name/trigger)
        fs::write(&path, &skill.content)?;
        written.push(path.to_string_lossy().to_string());
    }
    Ok(())
}

fn export_mcp_json(pkg: &AipkPackage, path: &Path, written: &mut Vec<String>) -> Result<()> {
    let Some(tools) = pkg.tools_json() else {
        return Ok(());
    };
    let Some(servers) = tools["mcp_servers"].as_array() else {
        return Ok(());
    };
    if servers.is_empty() {
        return Ok(());
    }
    // Standard `mcpServers` map understood by Claude Code and most MCP hosts
    let mut map = serde_json::Map::new();
    for s in servers {
        let name = s["name"].as_str().unwrap_or("tool").to_string();
        map.insert(
            name,
            json!({
                "command": s["command"],
                "args": s["args"],
            }),
        );
    }
    let out = json!({ "mcpServers": map });
    fs::write(path, serde_json::to_string_pretty(&out)?)?;
    written.push(path.to_string_lossy().to_string());
    Ok(())
}

fn export_knowledge(pkg: &AipkPackage, dir: &Path, written: &mut Vec<String>) -> Result<()> {
    let Some(sec) = pkg.section("KNOW") else {
        return Ok(());
    };
    let (chunks, _vectors, _dim) = parse_know_section(pkg.section_data(sec))?;
    if chunks.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;

    // Group chunks by source, preserving order
    let mut order: Vec<String> = Vec::new();
    let mut by_source: std::collections::HashMap<String, Vec<String>> = Default::default();
    for c in chunks {
        let source = if c.source.is_empty() {
            "unknown".to_string()
        } else {
            c.source
        };
        if !by_source.contains_key(&source) {
            order.push(source.clone());
        }
        by_source.entry(source).or_default().push(c.text);
    }

    for source in order {
        let texts = by_source.remove(&source).unwrap_or_default();
        let path = dir.join(format!("{}.md", slugify(&source)));
        let body = format!("# Source: {source}\n\n{}\n", texts.join("\n\n"));
        fs::write(&path, body)?;
        written.push(path.to_string_lossy().to_string());
    }
    Ok(())
}

fn slugify(s: &str) -> String {
    let slug: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = slug.trim_matches('-').to_string();
    // Collapse repeated dashes
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_dash = false;
    for c in trimmed.chars() {
        if c == '-' {
            if !prev_dash {
                out.push(c);
            }
            prev_dash = true;
        } else {
            out.push(c);
            prev_dash = false;
        }
    }
    if out.is_empty() {
        "unnamed".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{build_skil_section, AipkBuilder, SkillDef};

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join("aipk_export_tests").join(name);
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn build_test_pkg(dir: &Path) -> PathBuf {
        let mut b = AipkBuilder::new("exp-test");
        b.add("META", b"[package]\nname = \"exp-test\"\n".to_vec());
        b.add("PERS", b"You are a test expert.".to_vec());
        b.add(
            "SKIL",
            build_skil_section(&[SkillDef {
                name: "Review Code".to_string(),
                filename: "review.md".to_string(),
                trigger: "review".to_string(),
                content: "---\nname: Review Code\ntrigger: review\n---\nDo a review.".to_string(),
            }]),
        );
        b.add(
            "TOOL",
            br#"{"mcp_servers":[{"name":"fs","command":"npx","args":["-y","server-fs"]}]}"#
                .to_vec(),
        );
        let path = dir.join("exp-test.aipk");
        fs::write(&path, b.build()).unwrap();
        path
    }

    #[test]
    fn export_claude_code_layout() {
        let dir = tmp_dir("claude");
        let pkg = build_test_pkg(&dir);
        let out = dir.join("out");
        run(&pkg, "claude-code", Some(&out), None).unwrap();

        assert_eq!(
            fs::read_to_string(out.join("CLAUDE.md")).unwrap(),
            "You are a test expert."
        );
        let skill = fs::read_to_string(out.join(".claude/skills/review-code/SKILL.md")).unwrap();
        assert!(skill.contains("Do a review."));
        let mcp: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(out.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["fs"]["command"], "npx");
    }

    #[test]
    fn export_openclaw_uses_soul_md() {
        let dir = tmp_dir("openclaw");
        let pkg = build_test_pkg(&dir);
        let out = dir.join("out");
        run(&pkg, "openclaw", Some(&out), None).unwrap();
        assert!(out.join("SOUL.md").exists());
        assert!(out.join("skills/review-code/SKILL.md").exists());
    }

    #[test]
    fn export_unknown_target_fails() {
        let dir = tmp_dir("unknown");
        let pkg = build_test_pkg(&dir);
        assert!(run(&pkg, "cursor", None, None).is_err());
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Review Code"), "review-code");
        assert_eq!(slugify("docs/файл.pdf"), "docs-файл-pdf");
        assert_eq!(slugify("---"), "unnamed");
    }
}
