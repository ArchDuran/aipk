use anyhow::Result;
use std::fs;
use std::path::Path;

/// Create template files for AIPK v2 Epistemic sections in a project directory.
pub fn run(project_dir: &Path) -> Result<()> {
    let dir = project_dir;
    if !dir.exists() {
        anyhow::bail!("Directory not found: {}", dir.display());
    }

    // identity.json — IDTY section
    let idty_path = dir.join("identity.json");
    if !idty_path.exists() {
        fs::write(
            &idty_path,
            r#"{
  "name": "My Agent",
  "role": "domain expert",
  "version": "1.0",
  "owner": "",
  "description": "Describe what this agent is and what it knows."
}
"#,
        )?;
        println!("  created: identity.json  (IDTY — identity contract)");
    } else {
        println!("  exists:  identity.json");
    }

    // answerability.json — ANSP section
    let ansp_path = dir.join("answerability.json");
    if !ansp_path.exists() {
        fs::write(
            &ansp_path,
            r#"{
  "domain": ["keyword1", "keyword2"],
  "adjacent": [],
  "adjacent_allowed": false,
  "confidence_threshold": 0.25,
  "out_of_scope_message": "This query is outside my area of expertise.",
  "escalate_message": ""
}
"#,
        )?;
        println!("  created: answerability.json  (ANSP — answerability gate)");
    } else {
        println!("  exists:  answerability.json");
    }

    // policy.json — PLCY section
    let plcy_path = dir.join("policy.json");
    if !plcy_path.exists() {
        fs::write(
            &plcy_path,
            r#"{
  "citation_required": false,
  "forbidden": [
    "Do not speculate beyond what the knowledge base supports.",
    "Do not provide legal, medical, or financial advice."
  ]
}
"#,
        )?;
        println!("  created: policy.json  (PLCY — answer policy)");
    } else {
        println!("  exists:  policy.json");
    }

    // negative.jsonl — NKNW section
    let nknw_path = dir.join("negative.jsonl");
    if !nknw_path.exists() {
        fs::write(
            &nknw_path,
            r#"{"type":"forbidden","pattern":"competitor pricing","refusal":"I cannot provide information about competitor pricing."}
{"type":"unknown","pattern":"internal roadmap","note":"Information about future roadmap is not available in this package."}
"#,
        )?;
        println!("  created: negative.jsonl  (NKNW — negative knowledge registry)");
    } else {
        println!("  exists:  negative.jsonl");
    }

    println!(
        "\nEpistemic v2 template files created in {}.",
        dir.display()
    );
    println!("Edit these files, then run `aipk build` to embed them into your package.");
    println!("\nSections:");
    println!("  identity.json      → IDTY  (who is this agent, injected into system prompt)");
    println!("  answerability.json → ANSP  (domain gate: refuse/allow/partial/escalate)");
    println!("  policy.json        → PLCY  (citation policy, forbidden behaviors)");
    println!("  negative.jsonl     → NKNW  (forbidden topics, acknowledged unknowns)");

    Ok(())
}
