use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub fn run(project_dir: &Path, name: &str, command: &str, args: &[String]) -> Result<()> {
    let tools_file = project_dir.join("tools.json");

    let mut tools: Value = if tools_file.exists() {
        serde_json::from_str(&fs::read_to_string(&tools_file)?)?
    } else {
        json!({"mcp_servers": [], "tools": []})
    };

    if tools["mcp_servers"].as_array().is_none() {
        tools["mcp_servers"] = json!([]);
    }

    if tools["mcp_servers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["name"].as_str() == Some(name))
    {
        anyhow::bail!("MCP server '{}' already exists in tools.json", name);
    }

    tools["mcp_servers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": name, "command": command, "args": args}));

    fs::write(&tools_file, serde_json::to_string_pretty(&tools)?)?;
    println!("✓ MCP server '{}' added → tools.json", name);
    println!("  command: {} {}", command, args.join(" "));
    Ok(())
}
