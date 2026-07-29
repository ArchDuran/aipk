use anyhow::Result;
use std::fs;
use std::path::Path;

pub fn run(project_dir: &Path, skill_file: &Path) -> Result<()> {
    if !skill_file.exists() {
        anyhow::bail!("File not found: {}", skill_file.display());
    }
    if skill_file.extension().and_then(|e| e.to_str()) != Some("md") {
        anyhow::bail!("Skill file must be .md");
    }

    let skills_dir = project_dir.join("skills");
    fs::create_dir_all(&skills_dir)?;

    let filename = skill_file
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid filename"))?;
    let dest = skills_dir.join(filename);
    fs::copy(skill_file, &dest)?;

    let content = fs::read_to_string(&dest)?;
    let name = frontmatter_field(&content, "name").unwrap_or_else(|| {
        Path::new(filename)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let trigger = frontmatter_field(&content, "trigger").unwrap_or_default();

    println!(
        "✓ Skill '{}' added → skills/{}",
        name,
        filename.to_string_lossy()
    );
    if !trigger.is_empty() {
        println!("  trigger: '{}'", trigger);
    }
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
