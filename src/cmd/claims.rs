use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufWriter, Write};
use std::path::Path;

use crate::cmd::extract_claims::now_rfc3339;

// ─── shared helpers ──────────────────────────────────────────────────────────

fn claims_path(dir: &Path) -> std::path::PathBuf {
    dir.join("claims.jsonl")
}

fn load_claims(dir: &Path) -> Result<Vec<Value>> {
    let path = claims_path(dir);
    if !path.exists() {
        bail!(
            "claims.jsonl not found in {}. Run 'aipk extract-claims' first.",
            dir.display()
        );
    }
    let text = fs::read_to_string(&path)?;
    let claims: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<serde_json::Result<_>>()?;
    Ok(claims)
}

fn save_claims(dir: &Path, claims: &[Value]) -> Result<()> {
    let path = claims_path(dir);
    let f = fs::File::create(&path)?;
    let mut w = BufWriter::new(f);
    for c in claims {
        writeln!(w, "{}", serde_json::to_string(c)?)?;
    }
    w.flush()?;
    Ok(())
}

// ─── list ────────────────────────────────────────────────────────────────────

pub fn list(dir: &Path, status_filter: Option<&str>, output_json: bool) -> Result<()> {
    let claims = load_claims(dir)?;

    let filtered: Vec<&Value> = claims
        .iter()
        .filter(|c| {
            if let Some(s) = status_filter {
                c["status"].as_str().unwrap_or("") == s
            } else {
                true
            }
        })
        .collect();

    if output_json {
        println!("{}", serde_json::to_string_pretty(&json!(filtered))?);
        return Ok(());
    }

    if filtered.is_empty() {
        let filter_desc = status_filter
            .map(|s| format!(" with status '{s}'"))
            .unwrap_or_default();
        println!("No claims found{}.", filter_desc);
        return Ok(());
    }

    let label = status_filter.unwrap_or("all");
    println!("Claims [{}]  total={}\n", label, filtered.len());

    for c in &filtered {
        let id = c["id"].as_str().unwrap_or("?");
        let text = c["text"].as_str().unwrap_or("");
        let status = c["status"].as_str().unwrap_or("?");
        let source = c["source"].as_str().unwrap_or("");

        let status_icon = match status {
            "canonical" => "✓",
            "extracted" => "·",
            "reviewed" => "○",
            "deprecated" => "✗",
            _ => "?",
        };

        let text_preview = if text.len() > 80 {
            format!("{}…", &text[..79])
        } else {
            text.to_string()
        };

        println!("{} [{}]  {}", status_icon, id, text_preview);
        if !source.is_empty() {
            println!("  source: {}", source);
        }

        // Show last audit entry
        if let Some(audit) = c["audit"].as_array() {
            if let Some(last) = audit.last() {
                let action = last["action"].as_str().unwrap_or("?");
                let by = last["reviewer"]
                    .as_str()
                    .or_else(|| last["model"].as_str())
                    .unwrap_or("?");
                let at = last["at"].as_str().unwrap_or("");
                println!("  last: {} by {} at {}", action, by, at);
            }
        }
        println!();
    }

    Ok(())
}

// ─── promote ─────────────────────────────────────────────────────────────────

pub fn promote(
    dir: &Path,
    claim_id: &str,
    reviewer: Option<&str>,
    reason: Option<&str>,
    target_status: &str,
) -> Result<()> {
    let mut claims = load_claims(dir)?;

    let claim = claims
        .iter_mut()
        .find(|c| c["id"].as_str() == Some(claim_id));

    let Some(claim) = claim else {
        bail!("Claim '{}' not found.", claim_id);
    };

    let current = claim["status"].as_str().unwrap_or("extracted").to_string();
    if current == "deprecated" {
        bail!("Claim '{}' is deprecated and cannot be promoted.", claim_id);
    }
    if current == target_status {
        bail!("Claim '{}' is already '{}'.", claim_id, target_status);
    }

    claim["status"] = json!(target_status);

    let mut audit_entry = json!({
        "action": "promote",
        "from": current,
        "to": target_status,
        "at": now_rfc3339(),
    });
    if let Some(r) = reviewer {
        audit_entry["reviewer"] = json!(r);
    }
    if let Some(r) = reason {
        audit_entry["reason"] = json!(r);
    }

    if let Some(audit) = claim["audit"].as_array_mut() {
        audit.push(audit_entry);
    } else {
        claim["audit"] = json!([audit_entry]);
    }

    save_claims(dir, &claims)?;

    let reviewer_str = reviewer.map(|r| format!(" by {r}")).unwrap_or_default();
    let reason_str = reason.map(|r| format!(" — {r}")).unwrap_or_default();
    println!(
        "✓ [{}] promoted: {} → {}{}{}",
        claim_id, current, target_status, reviewer_str, reason_str
    );

    Ok(())
}

// ─── reject ──────────────────────────────────────────────────────────────────

pub fn reject(
    dir: &Path,
    claim_id: &str,
    reviewer: Option<&str>,
    reason: Option<&str>,
) -> Result<()> {
    let mut claims = load_claims(dir)?;

    let claim = claims
        .iter_mut()
        .find(|c| c["id"].as_str() == Some(claim_id));

    let Some(claim) = claim else {
        bail!("Claim '{}' not found.", claim_id);
    };

    let current = claim["status"].as_str().unwrap_or("extracted").to_string();
    if current == "deprecated" {
        bail!("Claim '{}' is already deprecated.", claim_id);
    }

    claim["status"] = json!("deprecated");

    let mut audit_entry = json!({
        "action": "reject",
        "from": current,
        "to": "deprecated",
        "at": now_rfc3339(),
    });
    if let Some(r) = reviewer {
        audit_entry["reviewer"] = json!(r);
    }
    if let Some(r) = reason {
        audit_entry["reason"] = json!(r);
    }

    if let Some(audit) = claim["audit"].as_array_mut() {
        audit.push(audit_entry);
    } else {
        claim["audit"] = json!([audit_entry]);
    }

    save_claims(dir, &claims)?;

    let reviewer_str = reviewer.map(|r| format!(" by {r}")).unwrap_or_default();
    let reason_str = reason.map(|r| format!(" — {r}")).unwrap_or_default();
    println!(
        "✗ [{}] rejected: {} → deprecated{}{}",
        claim_id, current, reviewer_str, reason_str
    );

    Ok(())
}

// ─── stats ───────────────────────────────────────────────────────────────────

pub fn stats(dir: &Path) -> Result<()> {
    let claims = load_claims(dir)?;

    let mut counts = std::collections::HashMap::<String, usize>::new();
    for c in &claims {
        let status = c["status"].as_str().unwrap_or("unknown").to_string();
        *counts.entry(status).or_insert(0) += 1;
    }

    let total = claims.len();
    println!("Claims summary  total={}", total);
    for status in &["extracted", "reviewed", "canonical", "deprecated"] {
        let n = counts.get(*status).copied().unwrap_or(0);
        let bar = "█".repeat((n * 20).checked_div(total).unwrap_or(0));
        println!("  {:12}  {:4}  {}", status, n, bar);
    }

    Ok(())
}

// ─── promote --all ───────────────────────────────────────────────────────────

pub fn promote_all(dir: &Path, from_status: &str, reviewer: Option<&str>) -> Result<()> {
    let mut claims = load_claims(dir)?;

    let ids: Vec<String> = claims
        .iter()
        .filter(|c| c["status"].as_str().unwrap_or("") == from_status)
        .filter_map(|c| c["id"].as_str().map(|s| s.to_string()))
        .collect();

    if ids.is_empty() {
        println!("No claims with status '{}'.", from_status);
        return Ok(());
    }

    let at = now_rfc3339();
    let mut promoted = 0usize;

    for claim in claims.iter_mut() {
        let id = claim["id"].as_str().unwrap_or("").to_string();
        if !ids.contains(&id) {
            continue;
        }

        claim["status"] = json!("canonical");
        let mut entry = json!({
            "action": "promote",
            "from": from_status,
            "to": "canonical",
            "at": at,
        });
        if let Some(r) = reviewer {
            entry["reviewer"] = json!(r);
        }
        if let Some(audit) = claim["audit"].as_array_mut() {
            audit.push(entry);
        } else {
            claim["audit"] = json!([entry]);
        }
        promoted += 1;
    }

    save_claims(dir, &claims)?;
    println!(
        "✓ Promoted {} claims: {} → canonical",
        promoted, from_status
    );
    Ok(())
}

// ─── review (interactive) ────────────────────────────────────────────────────

pub fn review(dir: &Path, reviewer: Option<&str>) -> Result<()> {
    let mut claims = load_claims(dir)?;

    let pending: Vec<usize> = claims
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let s = c["status"].as_str().unwrap_or("");
            s == "extracted" || s == "reviewed"
        })
        .map(|(i, _)| i)
        .collect();

    if pending.is_empty() {
        println!("No claims pending review (extracted or reviewed).");
        return Ok(());
    }

    println!(
        "Reviewing {} claims — y=canonical  n=reject  s=skip  q=quit\n",
        pending.len()
    );

    let stdin = std::io::stdin();
    let mut changed = 0usize;

    for (pos, &idx) in pending.iter().enumerate() {
        let id = claims[idx]["id"].as_str().unwrap_or("?").to_string();
        let text = claims[idx]["text"].as_str().unwrap_or("").to_string();
        let source = claims[idx]["source"].as_str().unwrap_or("").to_string();
        let span = claims[idx]["span"].as_str().unwrap_or("").to_string();
        let status = claims[idx]["status"].as_str().unwrap_or("?").to_string();

        println!("[{}/{}] ({}) {}", pos + 1, pending.len(), id, status);
        println!("  {}", text);
        if !span.is_empty() {
            println!("  span: \"{}\"", span);
        }
        if !source.is_empty() {
            println!("  src:  {}", source);
        }
        print!("  > ");
        std::io::stdout().flush()?;

        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        let choice = line.trim().to_lowercase();

        let at = now_rfc3339();
        match choice.as_str() {
            "y" | "yes" => {
                claims[idx]["status"] = json!("canonical");
                let mut entry =
                    json!({"action": "promote", "from": status, "to": "canonical", "at": at});
                if let Some(r) = reviewer {
                    entry["reviewer"] = json!(r);
                }
                if let Some(a) = claims[idx]["audit"].as_array_mut() {
                    a.push(entry);
                }
                println!("  ✓ canonical\n");
                changed += 1;
            }
            "n" | "no" => {
                print!("  reason (optional): ");
                std::io::stdout().flush()?;
                let mut reason = String::new();
                stdin.lock().read_line(&mut reason)?;
                let reason = reason.trim().to_string();

                claims[idx]["status"] = json!("deprecated");
                let mut entry =
                    json!({"action": "reject", "from": status, "to": "deprecated", "at": at});
                if let Some(r) = reviewer {
                    entry["reviewer"] = json!(r);
                }
                if !reason.is_empty() {
                    entry["reason"] = json!(reason);
                }
                if let Some(a) = claims[idx]["audit"].as_array_mut() {
                    a.push(entry);
                }
                println!("  ✗ deprecated\n");
                changed += 1;
            }
            "q" | "quit" => {
                println!("  Quit. Saving {} changes.", changed);
                break;
            }
            _ => {
                println!("  — skipped\n");
            }
        }
    }

    save_claims(dir, &claims)?;

    let remaining = claims
        .iter()
        .filter(|c| matches!(c["status"].as_str().unwrap_or(""), "extracted" | "reviewed"))
        .count();

    println!(
        "\nDone. Changed: {}  Remaining pending: {}",
        changed, remaining
    );
    Ok(())
}

// ─── check-conflicts ─────────────────────────────────────────────────────────

/// Check for potential contradictions between canonical claims using LLM as NLI judge.
/// Only compares canonical claims against each other; exits early if < 2 canonical claims.
pub async fn check_conflicts(
    dir: &Path,
    llm_url: &str,
    model: &str,
    api_key: &str,
    max_pairs: usize,
) -> Result<()> {
    let claims = load_claims(dir)?;
    let canonical: Vec<&Value> = claims
        .iter()
        .filter(|c| c["status"].as_str() == Some("canonical"))
        .collect();

    if canonical.len() < 2 {
        println!(
            "Need at least 2 canonical claims to check for conflicts. Found {}.",
            canonical.len()
        );
        return Ok(());
    }

    println!(
        "Checking {} canonical claims for conflicts (max {} pairs)...\n",
        canonical.len(),
        max_pairs
    );

    let client = crate::agent::build_client();
    let url = format!("{}/v1/chat/completions", llm_url.trim_end_matches('/'));
    let headers = crate::agent::auth_headers(api_key);
    let mut found = 0usize;
    let mut checked = 0usize;

    'outer: for i in 0..canonical.len() {
        for j in (i + 1)..canonical.len() {
            if checked >= max_pairs {
                break 'outer;
            }
            checked += 1;

            let a = canonical[i]["text"].as_str().unwrap_or("");
            let b = canonical[j]["text"].as_str().unwrap_or("");
            let id_a = canonical[i]["id"].as_str().unwrap_or("?");
            let id_b = canonical[j]["id"].as_str().unwrap_or("?");

            let prompt = format!(
                "Do the following two statements contradict each other? \
Answer with a single word: CONTRADICT, CONSISTENT, or UNRELATED.\n\
Statement 1: {a}\nStatement 2: {b}"
            );

            let body = json!({
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "temperature": 0.0
            });

            let resp = client
                .post(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await;

            match resp {
                Ok(r) => {
                    let v: Value = r.json().await.unwrap_or_default();
                    let verdict = v["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or("")
                        .trim()
                        .to_uppercase();

                    if verdict.contains("CONTRADICT") {
                        found += 1;
                        println!("CONFLICT [{id_a}] × [{id_b}]");
                        println!("  A: {a}");
                        println!("  B: {b}");
                        println!();
                    }
                }
                Err(e) => {
                    eprintln!("LLM error on pair ({id_a},{id_b}): {e}");
                }
            }
        }
    }

    println!("Checked {checked} pair(s). Conflicts found: {found}.");
    if found > 0 {
        println!("Review conflicting claims with `aipk claims list` and use `aipk claims reject` to deprecate incorrect ones.");
    }
    Ok(())
}
