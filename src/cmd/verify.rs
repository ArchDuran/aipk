use anyhow::Result;
use serde_json::json;
use std::path::Path;

use crate::format::parse;
use crate::runtime::{ClaimStatus, ClaimsRuntime};

pub fn run(
    pkg_path: &Path,
    answer: &str,
    min_coverage: f32,
    include_unreviewed: bool,
    output_json: bool,
) -> Result<()> {
    if !(0.0..=1.0).contains(&min_coverage) {
        anyhow::bail!("--min-coverage must be between 0.0 and 1.0");
    }

    let pkg = parse(pkg_path)?;
    let claims = ClaimsRuntime::load(&pkg);

    if claims.is_empty() {
        anyhow::bail!(
            "Package '{}' has no CLMS section. Run 'aipk extract-claims' first.",
            pkg.name
        );
    }
    if !include_unreviewed && claims.canonical_count() == 0 {
        anyhow::bail!(
            "Package '{}' has claims, but none are canonical yet. Run 'aipk claims promote <id>' \
             to approve claims, or pass --include-unreviewed to match against unreviewed ones too.",
            pkg.name
        );
    }

    let sentences = split_sentences(answer);
    if sentences.is_empty() {
        anyhow::bail!("No sentences found in answer.");
    }

    let canonical_only = !include_unreviewed;
    let results = score_sentences(&claims, &sentences, canonical_only);

    let supported = results
        .iter()
        .filter(|r| r.verdict == "lexically_supported")
        .count();
    let total = results.len();
    let coverage = if total > 0 {
        supported as f32 / total as f32
    } else {
        0.0
    };
    let unsupported_sentences: Vec<&str> = results
        .iter()
        .filter(|r| r.verdict == "unsupported")
        .map(|r| r.text.as_str())
        .collect();

    if output_json {
        let out = json!({
            "package": pkg.name,
            "canonical_only": canonical_only,
            // Word-overlap + numeric-mismatch matching, not semantic entailment:
            // a citation that resolves and shares enough words/numbers with the
            // sentence counts as "lexically_supported", which is a narrower and
            // more honest claim than "true". See the EXPERIMENTAL note in README's strict-render section.
            "method": "lexical",
            "sentences": results.iter().map(|r| json!({
                "text": r.text,
                "verdict": r.verdict,
                "claim_id": r.claim_id,
                "claim_text": r.claim_text,
                "source": r.source,
                "span": r.span,
                "score": r.score,
            })).collect::<Vec<_>>(),
            "summary": {
                "total": total,
                "lexically_supported": supported,
                "unsupported": total - supported,
                "coverage": coverage,
                "unsupported_sentences": unsupported_sentences,
            }
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!(
            "Package: {}  claims={}  canonical={}",
            pkg.name,
            claims.claims.len(),
            claims.canonical_count()
        );
        println!(
            "Method: lexical (word-overlap + numeric-mismatch check, not semantic entailment)"
        );
        println!("Required coverage: {:.0}%", min_coverage * 100.0);
        if !canonical_only {
            println!("⚠  --include-unreviewed: matching against unreviewed claims too.");
        }
        println!();
        for (i, r) in results.iter().enumerate() {
            let icon = if r.verdict == "lexically_supported" {
                "✓"
            } else {
                "✗"
            };
            println!("{}  Sentence {}: \"{}\"", icon, i + 1, r.text);
            if r.verdict == "lexically_supported" {
                println!(
                    "   LEXICALLY SUPPORTED → [{}] \"{}\"  (score: {:.2})",
                    r.claim_id.as_deref().unwrap_or("?"),
                    r.claim_text.as_deref().unwrap_or(""),
                    r.score
                );
                if let Some(src) = &r.source {
                    let span_part = r
                        .span
                        .as_deref()
                        .map(|s| format!("  span: \"{}\"", s))
                        .unwrap_or_default();
                    println!("   source: {}{}", src, span_part);
                }
            } else {
                println!("   UNSUPPORTED  (best score: {:.2})", r.score);
            }
            println!();
        }
        let bar =
            "█".repeat((coverage * 20.0) as usize) + &"░".repeat(20 - (coverage * 20.0) as usize);
        println!(
            "Coverage: {}/{} sentences lexically supported  [{}] {:.0}%",
            supported,
            total,
            bar,
            coverage * 100.0
        );

        if coverage < 0.5 {
            println!("\n⚠  Low coverage — answer may contain unsupported claims.");
        } else if coverage == 1.0 {
            println!(
                "\n✓  All sentences lexically supported — every citation resolves and \
                 matches its claim's words and numbers. This is not a semantic truth check."
            );
        }
    }

    // Exit codes: 0 = meets requested coverage, 1 = below requested coverage
    if !coverage_meets_threshold(coverage, min_coverage) {
        std::process::exit(1);
    }
    Ok(())
}

/// Matches each sentence against `claims` and scores it lexically_supported/unsupported.
/// `canonical_only` restricts matching to `ClaimStatus::Canonical` claims — with it
/// off, `extracted`/`reviewed` (unreviewed) claims can support a sentence too. Deprecated
/// claims never do, regardless of `canonical_only` — `ClaimsRuntime::load`
/// already drops them, but we don't rely on that alone here.
fn score_sentences(
    claims: &ClaimsRuntime,
    sentences: &[String],
    canonical_only: bool,
) -> Vec<SentenceResult> {
    let mut results = Vec::new();
    for sentence in sentences {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        let relevant = claims.find_relevant(sentence, 1, 0.0, canonical_only);
        let relevant: Vec<_> = relevant
            .into_iter()
            .filter(|(_, c)| c.status != ClaimStatus::Deprecated)
            .collect();
        let result = match relevant.first() {
            Some((score, claim)) if *score >= 0.2 => SentenceResult {
                text: sentence.to_string(),
                verdict: "lexically_supported".to_string(),
                claim_id: Some(claim.id.clone()),
                claim_text: Some(claim.text.clone()),
                source: if claim.source.is_empty() {
                    None
                } else {
                    Some(claim.source.clone())
                },
                span: if claim.span.is_empty() {
                    None
                } else {
                    Some(claim.span.clone())
                },
                score: *score,
            },
            Some((score, _)) => SentenceResult {
                text: sentence.to_string(),
                verdict: "unsupported".to_string(),
                claim_id: None,
                claim_text: None,
                source: None,
                span: None,
                score: *score,
            },
            None => SentenceResult {
                text: sentence.to_string(),
                verdict: "unsupported".to_string(),
                claim_id: None,
                claim_text: None,
                source: None,
                span: None,
                score: 0.0,
            },
        };
        results.push(result);
    }
    results
}

fn coverage_meets_threshold(coverage: f32, min_coverage: f32) -> bool {
    coverage + f32::EPSILON >= min_coverage
}

struct SentenceResult {
    text: String,
    verdict: String,
    claim_id: Option<String>,
    claim_text: Option<String>,
    source: Option<String>,
    span: Option<String>,
    score: f32,
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?') && !current.trim().is_empty() {
            if crate::parsers::is_decimal_point(&current, chars.peek()) {
                continue;
            }
            let s = current.trim().to_string();
            if s.len() > 5 {
                sentences.push(s);
            }
            current.clear();
        }
    }
    if !current.trim().is_empty() && current.trim().len() > 5 {
        sentences.push(current.trim().to_string());
    }
    sentences
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Claim, ClaimStatus};

    #[test]
    fn coverage_threshold_accepts_partial_grounding_when_configured() {
        assert!(coverage_meets_threshold(0.8, 0.8));
        assert!(coverage_meets_threshold(0.9, 0.8));
        assert!(!coverage_meets_threshold(0.79, 0.8));
    }

    #[test]
    fn split_sentences_keeps_final_sentence_without_punctuation() {
        let sentences = split_sentences("First grounded sentence. Final grounded sentence");
        assert_eq!(
            sentences,
            vec!["First grounded sentence.", "Final grounded sentence"]
        );
    }

    fn claim(id: &str, text: &str, status: ClaimStatus) -> Claim {
        Claim {
            id: id.to_string(),
            text: text.to_string(),
            source: "regulations.pdf".to_string(),
            span: text.to_string(),
            status,
        }
    }

    /// Regression for the exact exploit found in review: a sentence with a
    /// fabricated number ("+500 °C") used to score `grounded`/coverage=1.0
    /// against a claim stating "-30 °C to +45 °C", because word overlap on
    /// the surrounding words ("operating", "temperature", "Frostline")
    /// outweighed the wrong number entirely.
    #[test]
    fn false_number_is_not_grounded_by_a_claim_with_different_numbers() {
        let claims = ClaimsRuntime {
            claims: vec![claim(
                "spec_0006",
                "Operating temperature: -30 °C to +45 °C.",
                ClaimStatus::Canonical,
            )],
            vectors: vec![],
        };
        let results = score_sentences(
            &claims,
            &["The operating temperature of the Frostline F4 is +500 °C.".to_string()],
            true,
        );
        assert_eq!(results[0].verdict, "unsupported");
    }

    #[test]
    fn matching_number_still_grounds_normally() {
        let claims = ClaimsRuntime {
            claims: vec![claim(
                "spec_0006",
                "Operating temperature: -30 °C to +45 °C.",
                ClaimStatus::Canonical,
            )],
            vectors: vec![],
        };
        let results = score_sentences(
            &claims,
            &["The operating temperature of the Frostline F4 is +45 °C.".to_string()],
            true,
        );
        assert_eq!(results[0].verdict, "lexically_supported");
    }

    #[test]
    fn canonical_only_ignores_extracted_and_reviewed_claims() {
        let claims = ClaimsRuntime {
            claims: vec![
                claim(
                    "c1",
                    "Data must be retained for no longer than five years.",
                    ClaimStatus::Extracted,
                ),
                claim(
                    "c2",
                    "Data must be retained for no longer than five years.",
                    ClaimStatus::Reviewed,
                ),
            ],
            vectors: vec![],
        };
        let sentences = vec!["Data must be retained for no longer than five years.".to_string()];

        let results = score_sentences(&claims, &sentences, true);
        assert_eq!(results[0].verdict, "unsupported");
    }

    #[test]
    fn canonical_only_matches_canonical_claims() {
        let claims = ClaimsRuntime {
            claims: vec![claim(
                "c1",
                "Data must be retained for no longer than five years.",
                ClaimStatus::Canonical,
            )],
            vectors: vec![],
        };
        let sentences = vec!["Data must be retained for no longer than five years.".to_string()];

        let results = score_sentences(&claims, &sentences, true);
        assert_eq!(results[0].verdict, "lexically_supported");
        assert_eq!(results[0].claim_id.as_deref(), Some("c1"));
    }

    #[test]
    fn canonical_only_never_matches_deprecated_claims() {
        let claims = ClaimsRuntime {
            claims: vec![claim(
                "c1",
                "Data must be retained for no longer than five years.",
                ClaimStatus::Deprecated,
            )],
            vectors: vec![],
        };
        let sentences = vec!["Data must be retained for no longer than five years.".to_string()];

        // Deprecated claims are excluded regardless of canonical_only.
        assert_eq!(
            score_sentences(&claims, &sentences, true)[0].verdict,
            "unsupported"
        );
        assert_eq!(
            score_sentences(&claims, &sentences, false)[0].verdict,
            "unsupported"
        );
    }

    #[test]
    fn include_unreviewed_matches_extracted_claims() {
        let claims = ClaimsRuntime {
            claims: vec![claim(
                "c1",
                "Data must be retained for no longer than five years.",
                ClaimStatus::Extracted,
            )],
            vectors: vec![],
        };
        let sentences = vec!["Data must be retained for no longer than five years.".to_string()];

        let results = score_sentences(&claims, &sentences, false);
        assert_eq!(results[0].verdict, "lexically_supported");
        assert_eq!(results[0].claim_id.as_deref(), Some("c1"));
    }
}
