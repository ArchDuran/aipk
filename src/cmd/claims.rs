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

/// Maps a claim's `source` (the human-readable title written by extract-claims,
/// e.g. "frostline-f4-spec.md") to the on-disk path of that source file, via
/// sources.jsonl. Missing/malformed entries are skipped rather than failing
/// the whole load — a lookup miss just means that claim's span can't be
/// checked, not that promotion should hard-error.
fn load_source_titles_to_paths(
    dir: &Path,
) -> std::collections::HashMap<String, std::path::PathBuf> {
    let path = dir.join("sources.jsonl");
    let Ok(text) = fs::read_to_string(&path) else {
        return std::collections::HashMap::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            let title = v["title"].as_str()?.to_string();
            let path = v["path"].as_str()?.to_string();
            Some((title, std::path::PathBuf::from(path)))
        })
        .collect()
}

/// Lowercases, collapses whitespace, drops markdown emphasis markers, and
/// folds common Unicode punctuation variants (curly quotes, en/em dash,
/// minus sign) to their ASCII equivalents, so a span that's byte-for-byte
/// identical in meaning but typographically reformatted still matches its
/// source — e.g. a source line `1. **Frostline** — pallet-moving robots...`
/// against an extracted span `Frostline — pallet-moving robots...` (list
/// numbering and bold markers stripped, otherwise a verbatim quote).
fn normalize_for_match(s: &str) -> String {
    let folded: String = s
        .chars()
        .filter(|c| *c != '*' && *c != '_')
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            other => other,
        })
        .collect();
    folded
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// True if `span` is (near-)verbatim present in `source_text` — the contract
/// verbatim-mode extraction is supposed to honor. Catches both wholesale
/// fabrication (a claim with no basis in the document at all) and quieter
/// corruption (a real sentence with one value silently changed, e.g. "-25 °C"
/// rewritten as "-5 °C" — the mutated span is no longer a substring of the
/// source even though most of the sentence still reads plausibly).
fn span_is_grounded(span: &str, source_text: &str) -> bool {
    let span_norm = normalize_for_match(span);
    if span_norm.is_empty() {
        return false;
    }
    normalize_for_match(source_text).contains(&span_norm)
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
    let source_paths = load_source_titles_to_paths(dir);
    let mut source_cache: std::collections::HashMap<std::path::PathBuf, String> =
        std::collections::HashMap::new();

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
    let mut held_back_span: Vec<String> = Vec::new();
    let mut held_back_digest: Vec<String> = Vec::new();

    for claim in claims.iter_mut() {
        let id = claim["id"].as_str().unwrap_or("").to_string();
        if !ids.contains(&id) {
            continue;
        }

        // Digest-mode claims are deliberate paraphrases, not verbatim quotes —
        // there's no source substring to check them against, so they can't be
        // auto-verified at all and always need a human to promote them by
        // hand. Verbatim-mode (and legacy claims with no "mode" field) get
        // the span-grounding check instead: --auto-promote exists to skip
        // human review, so this is the one automated backstop against
        // extraction hallucinating a claim with no basis in the document.
        let mode = claim["mode"].as_str().unwrap_or("verbatim");
        if mode == "digest" {
            held_back_digest.push(id);
            continue;
        }
        let span = claim["span"].as_str().unwrap_or("");
        let source_title = claim["source"].as_str().unwrap_or("");
        let grounded = source_paths.get(source_title).is_some_and(|path| {
            let text = source_cache
                .entry(path.clone())
                .or_insert_with(|| fs::read_to_string(path).unwrap_or_default());
            span_is_grounded(span, text)
        });
        if !grounded {
            held_back_span.push(id);
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
    if !held_back_span.is_empty() {
        println!(
            "⚠ Held back {} claim(s) whose span doesn't appear in its source document \
             (still '{}' — needs human review via 'aipk claims promote <id>' or 'aipk claims reject <id>'):",
            held_back_span.len(),
            from_status
        );
        for id in &held_back_span {
            println!("    {id}");
        }
    }
    if !held_back_digest.is_empty() {
        println!(
            "⚠ Held back {} digest-mode claim(s) — paraphrases can't be auto-verified \
             (still '{}' — needs human review via 'aipk claims promote <id>' or 'aipk claims reject <id>'):",
            held_back_digest.len(),
            from_status
        );
        for id in &held_back_digest {
            println!("    {id}");
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("aipk_claims_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Regression for the exact failure mode from review: extraction hands
    /// back a claim with no basis in the source document at all (a pallet
    /// robot spec turned into "a missile system"), and --auto-promote used
    /// to wave it straight through to canonical with no check whatsoever.
    #[test]
    fn promote_all_holds_back_a_claim_whose_span_is_not_in_the_source() {
        let dir = scratch_dir("holds_back");
        fs::write(
            dir.join("doc.md"),
            "Operating temperature: -30 C to +45 C.\n",
        )
        .unwrap();
        fs::write(
            dir.join("sources.jsonl"),
            format!(
                "{}\n",
                json!({
                    "id": "s_doc",
                    "title": "doc.md",
                    "path": dir.join("doc.md").display().to_string(),
                })
            ),
        )
        .unwrap();
        let entries = [
            json!({
                "id": "doc_0000", "text": "Operating temperature: -30 C to +45 C.",
                "source": "doc.md", "span": "Operating temperature: -30 C to +45 C.",
                "mode": "verbatim", "status": "extracted", "confidence": 1.0, "audit": []
            }),
            json!({
                "id": "doc_0001", "text": "This robot is a missile system.",
                "source": "doc.md", "span": "This robot is a missile system.",
                "mode": "verbatim", "status": "extracted", "confidence": 1.0, "audit": []
            }),
        ]
        .iter()
        .map(|v| serde_json::to_string(v).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
        fs::write(dir.join("claims.jsonl"), entries + "\n").unwrap();

        promote_all(&dir, "extracted", None).unwrap();

        let claims = load_claims(&dir).unwrap();
        let real = claims.iter().find(|c| c["id"] == "doc_0000").unwrap();
        let fabricated = claims.iter().find(|c| c["id"] == "doc_0001").unwrap();
        assert_eq!(real["status"], "canonical");
        assert_eq!(fabricated["status"], "extracted");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn promote_all_never_auto_promotes_digest_mode() {
        // Digest mode is a deliberate paraphrase, not a verbatim quote — there's
        // no substring to check it against, so --auto-promote can't verify it
        // at all and must always hold it back for a human.
        let dir = scratch_dir("digest_never_promotes");
        fs::write(dir.join("doc.md"), "Some unrelated source text.\n").unwrap();
        fs::write(
            dir.join("sources.jsonl"),
            format!(
                "{}\n",
                json!({
                    "id": "s_doc",
                    "title": "doc.md",
                    "path": dir.join("doc.md").display().to_string(),
                })
            ),
        )
        .unwrap();
        fs::write(
            dir.join("claims.jsonl"),
            format!(
                "{}\n",
                json!({
                    "id": "doc_0000", "text": "A paraphrased summary of the source.",
                    "source": "doc.md", "span": "A paraphrased summary of the source.",
                    "mode": "digest", "status": "extracted", "confidence": 1.0, "audit": []
                })
            ),
        )
        .unwrap();

        promote_all(&dir, "extracted", None).unwrap();

        let claims = load_claims(&dir).unwrap();
        assert_eq!(claims[0]["status"], "extracted");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn span_is_grounded_ignores_punctuation_style() {
        assert!(span_is_grounded(
            "the F4 supports \u{2018}outdoor\u{2019} use",
            "The F4 supports 'outdoor' use in most climates."
        ));
        assert!(!span_is_grounded(
            "the price is $9,000",
            "The device has no listed price."
        ));
    }

    /// Regression: a real benchmark run held back a genuinely-sourced claim
    /// because the source line used markdown list numbering and bold
    /// ("1. **Frostline** — pallet-moving robots...") while the extracted
    /// span dropped both ("Frostline — pallet-moving robots..."), and a
    /// literal substring check doesn't see past a bold marker in the middle
    /// of the doc's text.
    #[test]
    fn span_is_grounded_ignores_markdown_emphasis_and_list_numbering() {
        assert!(span_is_grounded(
            "Frostline — pallet-moving robots for freezer warehouses rated to -30 C.",
            "1. **Frostline** — pallet-moving robots for freezer warehouses rated to -30 C.\n"
        ));
    }
}
