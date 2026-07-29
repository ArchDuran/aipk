use anyhow::Result;
use std::fs;
use std::path::Path;

pub fn run(name: &str, path: Option<&Path>) -> Result<()> {
    // When --dir is omitted, create a named subdirectory.
    // When --dir is given explicitly (even "."), use it as-is.
    let dir = match path {
        None => std::path::PathBuf::from(name),
        Some(p) => p.to_path_buf(),
    };
    fs::create_dir_all(&dir)?;
    fs::create_dir_all(dir.join("skills"))?;
    fs::create_dir_all(dir.join("docs"))?;

    fs::write(dir.join("manifest.toml"), manifest_template(name))?;
    fs::write(dir.join("persona.md"), persona_template(name))?;
    fs::write(dir.join("tools.json"), r#"{"mcp_servers":[],"tools":[]}"#)?;
    fs::write(dir.join("links.json"), links_template())?;
    fs::write(dir.join("thkg.md"), thkg_template(name))?;
    fs::write(dir.join("license.toml"), license_template())?;

    println!(
        "✓ Created {}/ — edit manifest.toml and persona.md, then run `aipk build`",
        dir.display()
    );
    Ok(())
}

fn manifest_template(name: &str) -> String {
    format!(
        r#"[package]
name        = "{name}"
version     = "0.1.0"
author      = ""
description = ""
license     = "MIT"

[model]
compatible      = []
min_context     = 4096
embedding_model = ""
embedding_dim   = 0

[runtime]
mcp_autostart = false
rag_top_k     = 5
rag_min_score = 0.7

[graph]
role      = "specialist"
depth     = 2
max_nodes = 5

[consolidation]
strategy = "idle"
idle_min = 30
keep_top = 0.7
hebbian  = true
"#
    )
}

fn persona_template(name: &str) -> String {
    format!("# Persona: {name}\n\nDescribe the agent's personality and expertise here.\n")
}

fn links_template() -> &'static str {
    r#"{"links":[],"router_hint":"Describe what this package covers."}"#
}

fn license_template() -> &'static str {
    r#"# License and copyright terms for this package and its dataset.
# Packed as the LICN section — always readable, even in sealed packages.

[license]
author    = ""
license   = "proprietary"   # or an SPDX id: MIT, CC-BY-4.0, ...
copyright = ""
terms     = ""              # human-readable usage terms
contact   = ""
allow_derivatives    = false
allow_redistribution = false
"#
}

fn thkg_template(name: &str) -> String {
    format!(
        "# Router description for: {name}\n\
         #\n\
         # Describe what this package knows and when queries should be routed here.\n\
         # This text is embedded at serve time for semantic routing in graph mode.\n\
         # Be specific about the domain, topics, and types of questions this package handles.\n\
         #\n\
         # Example:\n\
         # This package covers legal regulations, GDPR compliance, and data privacy.\n\
         # Route here for questions about data retention, consent, erasure requests,\n\
         # privacy rights, and regulatory obligations.\n\
         \n\
         This package is about {name}.\n"
    )
}
