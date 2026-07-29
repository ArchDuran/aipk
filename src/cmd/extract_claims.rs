use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::cmd::add_docs::chunk_text;

const VERBATIM_SYSTEM_PROMPT: &str = r#"Return a JSON array. For each sentence in the source that states a fact, output {"text": sentence, "span": exact sentence}. No markdown."#;

/// Paraphrase mode: for sources whose license does not permit reproducing
/// excerpts (proprietary books, CC-BY-NC-SA/ND material), a claim must be a
/// new sentence stating the fact, not a copy of the source's wording — the
/// package is then a set of study notes with citations, not a searchable
/// copy of the text.
const DIGEST_SYSTEM_PROMPT: &str = r#"Return a JSON array of distinct factual claims found in the source text. For each claim, output {"text": <the fact, restated as a new self-contained sentence in your own words>}. Do NOT copy or closely paraphrase any sentence from the source — state the underlying fact from scratch, the way study notes would. Skip opinions, marketing language, and anything that is not a verifiable fact. No markdown, no verbatim quotes."#;

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    project_dir: &Path,
    files: &[PathBuf],
    model: &str,
    llm_url: &str,
    api_key: &str,
    chunk_size: usize,
    digest: bool,
    source_map: Option<&Path>,
) -> Result<()> {
    let claims_file = project_dir.join("claims.jsonl");
    let sources_file = project_dir.join("sources.jsonl");

    let source_labels = load_source_map(source_map)?;

    // Load existing claim count for ID generation
    let existing_count = if claims_file.exists() {
        fs::read_to_string(&claims_file)?
            .lines()
            .filter(|l| !l.is_empty())
            .count()
    } else {
        0
    };

    let claims_f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&claims_file)?;
    let mut claims_w = BufWriter::new(claims_f);

    let sources_f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&sources_file)?;
    let mut sources_w = BufWriter::new(sources_f);

    let client = crate::agent::build_client();
    let mut claim_id = existing_count;
    let mut total_claims = 0usize;

    let system_prompt = if digest {
        DIGEST_SYSTEM_PROMPT
    } else {
        VERBATIM_SYSTEM_PROMPT
    };
    let mode = if digest { "digest" } else { "verbatim" };

    for file_path in files {
        let text = fs::read_to_string(file_path)
            .with_context(|| format!("reading {}", file_path.display()))?;
        let filename = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let source_prefix = file_path
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("src{}", claim_id));
        let source = source_labels
            .get(&filename)
            .cloned()
            .unwrap_or_else(|| filename.clone());

        // Write source entry
        let source_entry = serde_json::to_string(&json!({
            "id": format!("s_{}", source_prefix),
            "title": source,
            "path": file_path.display().to_string(),
        }))?;
        writeln!(sources_w, "{}", source_entry)?;

        let chunks = chunk_text(&text, chunk_size);
        eprintln!("  {} → {} chunks", source, chunks.len());

        let mut file_claims = 0usize;
        for chunk in &chunks {
            let raw =
                call_llm_for_claims(&client, llm_url, model, chunk, system_prompt, api_key).await;
            let extracted = if digest {
                filter_and_dedup_claims(parse_claims_json(&raw))
            } else {
                claims_with_sentence_supplement(parse_claims_json(&raw), chunk)
            };

            for claim in extracted {
                let claim_text = claim["text"].as_str().unwrap_or("").trim().to_string();
                let span = claim["span"].as_str().unwrap_or("").trim().to_string();
                if claim_text.is_empty() || is_placeholder_claim(&claim_text, &span) {
                    continue;
                }
                let id = format!("{}_{:04}", source_prefix, claim_id);
                let entry = serde_json::to_string(&json!({
                    "id": id,
                    "text": claim_text,
                    "source": source,
                    "span": span,
                    "mode": mode,
                    "status": "extracted",
                    "confidence": 1.0,
                    "audit": [{"action": "extract", "model": model, "at": now_rfc3339()}],
                }))?;
                writeln!(claims_w, "{}", entry)?;
                claim_id += 1;
                file_claims += 1;
            }
        }

        total_claims += file_claims;
        eprintln!("    → {} claims extracted", file_claims);
    }

    claims_w.flush()?;
    sources_w.flush()?;

    println!("✓ Extracted {} claims → claims.jsonl", total_claims);
    println!("  Total claims in file: {}", claim_id);
    println!("  Sources logged → sources.jsonl");
    println!("  Run 'aipk build' to include CLMS section in package.");
    Ok(())
}

async fn call_llm_for_claims(
    client: &reqwest::Client,
    llm_url: &str,
    model: &str,
    chunk: &str,
    system_prompt: &str,
    api_key: &str,
) -> String {
    let payload = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": chunk},
        ],
        "stream": false,
        "temperature": 0.1,
    });

    let resp = match client
        .post(format!(
            "{}/v1/chat/completions",
            llm_url.trim_end_matches('/')
        ))
        .headers(crate::agent::auth_headers(api_key))
        .json(&payload)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("    [LLM error: {e}]");
            return "[]".to_string();
        }
    };

    if !resp.status().is_success() {
        eprintln!("    [LLM error: {}]", resp.status());
        return "[]".to_string();
    }

    let result: Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return "[]".to_string(),
    };

    result["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("[]")
        .to_string()
}

/// Load a `{"filename": "citation string"}` JSON map used to give sources a
/// real bibliographic label instead of the bare filename. Returns an empty
/// map when no path is given.
fn load_source_map(path: Option<&Path>) -> Result<HashMap<String, String>> {
    let Some(path) = path else {
        return Ok(HashMap::new());
    };
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading source map {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| {
        format!(
            "parsing {} as a JSON object of {{\"filename\": \"citation\"}}",
            path.display()
        )
    })
}

fn parse_claims_json(raw: &str) -> Vec<Value> {
    // Strip markdown code fences if present
    let cleaned = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    // Find the JSON array
    let start = cleaned.find('[').unwrap_or(0);
    let end = cleaned.rfind(']').map(|i| i + 1).unwrap_or(cleaned.len());
    let json_str = &cleaned[start..end];

    serde_json::from_str::<Vec<Value>>(json_str).unwrap_or_default()
}

/// Drop empty/placeholder claims and collect the normalized text/span of the
/// survivors, so callers that need to dedupe against them (the verbatim
/// sentence-supplement fallback) don't re-walk the list themselves.
fn filter_and_dedup_claims(mut claims: Vec<Value>) -> Vec<Value> {
    claims.retain(|claim| {
        let text = claim["text"].as_str().unwrap_or("").trim();
        let span = claim["span"].as_str().unwrap_or("").trim();
        !(text.is_empty() || is_placeholder_claim(text, span))
    });
    claims
}

fn claims_with_sentence_supplement(claims: Vec<Value>, chunk: &str) -> Vec<Value> {
    let mut claims = filter_and_dedup_claims(claims);
    let mut seen: HashSet<String> = HashSet::new();
    for claim in &claims {
        let text = claim["text"].as_str().unwrap_or("");
        let span = claim["span"].as_str().unwrap_or("");
        seen.insert(normalize_claim_text(text));
        if !span.is_empty() {
            seen.insert(normalize_claim_text(span));
        }
    }

    for sentence in factual_sentence_candidates(chunk) {
        if !has_similar_claim(&seen, &sentence) {
            seen.insert(normalize_claim_text(&sentence));
            claims.push(json!({
                "text": sentence,
                "span": sentence,
            }));
        }
    }

    claims
}

fn factual_sentence_candidates(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            if crate::parsers::is_decimal_point(&current, chars.peek()) {
                continue;
            }
            push_candidate_sentence(&mut sentences, &current);
            current.clear();
        }
    }
    push_candidate_sentence(&mut sentences, &current);

    sentences
}

fn push_candidate_sentence(out: &mut Vec<String>, raw: &str) {
    let sentence = raw.trim();
    if !looks_like_factual_sentence(sentence) {
        return;
    }
    out.push(sentence.to_string());
}

fn looks_like_factual_sentence(sentence: &str) -> bool {
    let trimmed = sentence.trim();
    if trimmed.len() < 20
        || trimmed.starts_with('#')
        || trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.ends_with(':')
        || trimmed.contains('?')
    {
        return false;
    }

    let word_count = trimmed.split_whitespace().count();
    word_count >= 4 && trimmed.chars().any(|c| c.is_alphabetic())
}

fn has_similar_claim(seen: &HashSet<String>, sentence: &str) -> bool {
    let candidate = normalize_claim_text(sentence);
    if candidate.is_empty() {
        return true;
    }
    seen.iter()
        .any(|existing| existing.contains(&candidate) || candidate.contains(existing))
}

fn normalize_claim_text(text: &str) -> String {
    text.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_placeholder_claim(text: &str, span: &str) -> bool {
    let text = text.trim().to_ascii_lowercase();
    let span = span.trim().to_ascii_lowercase();
    text == "complete factual claim."
        || text == "complete factual claim"
        || span == "short exact quote from source"
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACTUAL_CHUNK: &str = "AIPK packages can store persona text, skills, tools, links, claims, sources, knowledge chunks, and indexes. The CLMS section stores auditable claims in JSONL form. Canonical claims are used by strict verification modes. The package header is 96 bytes long.";

    #[test]
    fn decimal_numbers_do_not_split_sentences() {
        let sentences = factual_sentence_candidates(
            "The battery stores 8.4 kWh and lasts 11 hours. The robot stops within 0.9 m at full speed.",
        );
        assert_eq!(sentences.len(), 2);
        assert!(sentences[0].contains("8.4 kWh"));
        assert!(sentences[1].contains("0.9 m"));
    }

    #[test]
    fn parse_claims_json_accepts_markdown_fenced_array() {
        let raw = r#"```json
[
  {"text": "The CLMS section stores auditable claims.", "span": "CLMS section stores auditable claims"}
]
```"#;

        let claims = parse_claims_json(raw);
        assert_eq!(claims.len(), 1);
        assert_eq!(
            claims[0]["text"].as_str(),
            Some("The CLMS section stores auditable claims.")
        );
    }

    #[test]
    fn sentence_supplement_adds_missing_factual_sentences() {
        let llm_claims = vec![json!({
            "text": "The CLMS section stores auditable claims in JSONL form.",
            "span": "The CLMS section stores auditable claims in JSONL form."
        })];

        let claims = claims_with_sentence_supplement(llm_claims, FACTUAL_CHUNK);
        let texts: Vec<&str> = claims.iter().filter_map(|c| c["text"].as_str()).collect();

        assert_eq!(texts.len(), 4);
        assert!(texts.contains(&"AIPK packages can store persona text, skills, tools, links, claims, sources, knowledge chunks, and indexes."));
        assert!(texts.contains(&"The CLMS section stores auditable claims in JSONL form."));
        assert!(texts.contains(&"Canonical claims are used by strict verification modes."));
        assert!(texts.contains(&"The package header is 96 bytes long."));
    }

    #[test]
    fn sentence_supplement_filters_placeholder_claims() {
        let llm_claims = vec![json!({
            "text": "Complete factual claim.",
            "span": "short exact quote from source"
        })];

        let claims =
            claims_with_sentence_supplement(llm_claims, "The package header is 96 bytes long.");
        assert_eq!(claims.len(), 1);
        assert_eq!(
            claims[0]["text"].as_str(),
            Some("The package header is 96 bytes long.")
        );
    }

    #[test]
    fn sentence_candidates_ignore_headings_and_questions() {
        let claims = factual_sentence_candidates(
            "# Title\n\nWhat is AIPK? The package header is 96 bytes long.",
        );

        assert_eq!(claims, vec!["The package header is 96 bytes long."]);
    }

    #[test]
    fn filter_and_dedup_claims_drops_empty_and_placeholder_only() {
        let claims = vec![
            json!({"text": "The package header is 96 bytes long.", "span": ""}),
            json!({"text": "", "span": ""}),
            json!({"text": "Complete factual claim.", "span": "short exact quote from source"}),
        ];

        let kept = filter_and_dedup_claims(claims);
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0]["text"].as_str(),
            Some("The package header is 96 bytes long.")
        );
    }

    #[test]
    fn filter_and_dedup_claims_does_not_inject_source_sentences() {
        // Digest mode must not fall back to verbatim source text — that's
        // the whole point of using it for restrictively-licensed sources.
        let llm_claims = vec![json!({"text": "AIPK stores facts as claims."})];
        let kept = filter_and_dedup_claims(llm_claims);
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0]["text"].as_str(),
            Some("AIPK stores facts as claims.")
        );
    }

    #[test]
    fn load_source_map_returns_empty_when_none() {
        let map = load_source_map(None).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn load_source_map_reads_filename_to_citation_json() {
        let tmp =
            std::env::temp_dir().join(format!("aipk_source_map_test_{}.json", std::process::id()));
        std::fs::write(
            &tmp,
            r#"{"SRE_Book.pdf": "Site Reliability Engineering, O'Reilly, 2016"}"#,
        )
        .unwrap();

        let map = load_source_map(Some(&tmp)).unwrap();
        assert_eq!(
            map.get("SRE_Book.pdf").map(String::as_str),
            Some("Site Reliability Engineering, O'Reilly, 2016")
        );

        std::fs::remove_file(&tmp).ok();
    }
}
