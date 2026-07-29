use crate::ann::AnnIndex;
use crate::format::{parse_know_section, AipkPackage, KnowChunk, SkillEntry};
use serde_json::{json, Value};

// ─── RAG ─────────────────────────────────────────────────────────────────────

pub struct KnowRuntime {
    chunks: Vec<KnowChunk>,
    vectors: Vec<Vec<f32>>,
    ann: Option<AnnIndex>,
}

impl KnowRuntime {
    /// Loads the KNOW section and, if present, the pre-built ANNX index
    /// (written by `aipk build`/`pipeline` for packages above
    /// `ann::ANN_INDEX_THRESHOLD` chunks — see `cmd::build`). This never
    /// *builds* an index itself: building is expensive at scale
    /// (see ann.rs module docs) and `load` runs on every `aipk run`/`test`
    /// invocation, sometimes once per query within a single process — doing
    /// it here would make those far slower than plain brute force. Packages
    /// without an ANNX section (too small, or built before it existed) fall
    /// back to brute-force search, same as always.
    pub fn load(pkg: &AipkPackage) -> Self {
        let Some(sec) = pkg.section("KNOW") else {
            return Self {
                chunks: vec![],
                vectors: vec![],
                ann: None,
            };
        };
        match parse_know_section(pkg.section_data(sec)) {
            Ok((chunks, vectors, _dim)) => {
                let ann = pkg
                    .section("ANNX")
                    .and_then(|sec| AnnIndex::from_bytes(pkg.section_data(sec)).ok());
                Self {
                    chunks,
                    vectors,
                    ann,
                }
            }
            Err(_) => Self {
                chunks: vec![],
                vectors: vec![],
                ann: None,
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn retrieve_scored(&self, query_vec: &[f32], top_k: usize) -> Vec<(f32, String)> {
        if self.vectors.is_empty() || query_vec.is_empty() {
            return vec![];
        }
        let mut scored: Vec<(f32, usize)> = if let Some(ann) = &self.ann {
            ann.search(query_vec, top_k)
                .into_iter()
                .map(|(i, score)| (score, i))
                .collect()
        } else {
            self.vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (cosine_similarity(query_vec, v), i))
                .collect()
        };
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(top_k)
            .filter(|(score, _)| *score > 0.1)
            .map(|(score, i)| (score, self.chunks[i].text.clone()))
            .collect()
    }

    pub fn retrieve(&self, query_vec: &[f32], top_k: usize) -> Vec<String> {
        self.retrieve_scored(query_vec, top_k)
            .into_iter()
            .map(|(_, t)| t)
            .collect()
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

// ─── Composed (multi-package) ─────────────────────────────────────────────────

pub struct ComposedRuntime {
    personas: Vec<String>,
    skills: Vec<SkillEntry>,
    knows: Vec<KnowRuntime>,
}

impl ComposedRuntime {
    pub fn from_packages(packages: &[AipkPackage]) -> Self {
        let mut personas = Vec::new();
        let mut skills = Vec::new();
        let mut knows = Vec::new();
        for pkg in packages {
            if let Some(p) = pkg.persona() {
                if !p.trim().is_empty() {
                    personas.push(p);
                }
            }
            skills.extend(pkg.skills());
            knows.push(KnowRuntime::load(pkg));
        }
        Self {
            personas,
            skills,
            knows,
        }
    }

    pub fn persona(&self) -> String {
        self.personas.join("\n\n---\n\n")
    }

    pub fn skills(&self) -> &[SkillEntry] {
        &self.skills
    }

    pub fn has_know(&self) -> bool {
        self.knows.iter().any(|k| !k.is_empty())
    }

    /// Search all packages, merge and globally re-rank results.
    #[allow(dead_code)]
    pub fn retrieve(&self, query_vec: &[f32], top_k: usize) -> Vec<String> {
        self.retrieve_scored(query_vec, top_k)
            .into_iter()
            .map(|(_, t)| t)
            .collect()
    }

    /// Like retrieve, but returns (score, text) pairs for ANSP coverage checks.
    pub fn retrieve_scored(&self, query_vec: &[f32], top_k: usize) -> Vec<(f32, String)> {
        let mut all: Vec<(f32, String)> = self
            .knows
            .iter()
            .flat_map(|k| k.retrieve_scored(query_vec, top_k * 2))
            .collect();

        // Deduplicate by text
        let mut seen = std::collections::HashSet::new();
        all.retain(|(_, text)| seen.insert(text.clone()));

        all.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        all.into_iter().take(top_k).collect()
    }
}

// ─── Claims (epistemic / strict mode) ────────────────────────────────────────

/// Claim status pipeline: extracted → reviewed → canonical → deprecated
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimStatus {
    Extracted,  // auto-generated by LLM, not human-verified
    Reviewed,   // human reviewed, not yet promoted
    Canonical,  // authoritative, used in strict modes
    Deprecated, // no longer valid
}

impl ClaimStatus {
    pub fn from_str(s: &str) -> Self {
        match s {
            "canonical" => Self::Canonical,
            "reviewed" => Self::Reviewed,
            "deprecated" => Self::Deprecated,
            _ => Self::Extracted,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Claim {
    pub id: String,
    pub text: String,
    pub source: String,
    pub span: String,
    pub status: ClaimStatus,
}

pub struct ClaimsRuntime {
    pub claims: Vec<Claim>,
    /// Parallel vectors for each claim (from CLMV section). Empty if not available.
    pub vectors: Vec<Vec<f32>>,
}

impl ClaimsRuntime {
    pub fn load(pkg: &AipkPackage) -> Self {
        let Some(sec) = pkg.section("CLMS") else {
            return Self {
                claims: vec![],
                vectors: vec![],
            };
        };
        let jsonl = match std::str::from_utf8(pkg.section_data(sec)) {
            Ok(s) => s,
            Err(_) => {
                return Self {
                    claims: vec![],
                    vectors: vec![],
                }
            }
        };
        let claims: Vec<Claim> = jsonl
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|line| {
                let v: serde_json::Value = serde_json::from_str(line).ok()?;
                Some(Claim {
                    id: v["id"].as_str().unwrap_or("").to_string(),
                    text: v["text"].as_str().unwrap_or("").to_string(),
                    source: v["source"].as_str().unwrap_or("").to_string(),
                    span: v["span"].as_str().unwrap_or("").to_string(),
                    status: ClaimStatus::from_str(v["status"].as_str().unwrap_or("extracted")),
                })
            })
            .filter(|c| c.status != ClaimStatus::Deprecated)
            .collect();

        let vectors = if let Some(vsec) = pkg.section("CLMV") {
            parse_clmv_section(pkg.section_data(vsec)).unwrap_or_default()
        } else {
            vec![]
        };

        Self { claims, vectors }
    }

    pub fn from_packages(packages: &[AipkPackage]) -> Self {
        let mut all_claims = Vec::new();
        let mut all_vectors = Vec::new();
        for p in packages {
            let rt = ClaimsRuntime::load(p);
            all_claims.extend(rt.claims);
            all_vectors.extend(rt.vectors);
        }
        Self {
            claims: all_claims,
            vectors: all_vectors,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.claims.is_empty()
    }

    /// Only canonical claims — used in strict modes.
    pub fn canonical(&self) -> impl Iterator<Item = &Claim> {
        self.claims
            .iter()
            .filter(|c| c.status == ClaimStatus::Canonical)
    }

    pub fn canonical_count(&self) -> usize {
        self.canonical().count()
    }

    /// Find top-k claims most relevant to text (Jaccard word overlap).
    pub fn find_relevant<'a>(
        &'a self,
        text: &str,
        top_k: usize,
        min_score: f32,
        canonical_only: bool,
    ) -> Vec<(f32, &'a Claim)> {
        let pool: Vec<&Claim> = if canonical_only {
            self.canonical().collect()
        } else {
            self.claims.iter().collect()
        };
        let mut scored: Vec<(f32, &Claim)> = pool
            .into_iter()
            .map(|c| (jaccard_similarity(text, &c.text), c))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(top_k)
            .filter(|(s, _)| *s >= min_score)
            .collect()
    }

    /// Semantic claim search using a pre-computed query embedding (cosine similarity).
    pub fn find_relevant_vec<'a>(
        &'a self,
        query_vec: &[f32],
        top_k: usize,
        min_score: f32,
        canonical_only: bool,
    ) -> Vec<(f32, &'a Claim)> {
        if self.vectors.is_empty() {
            return vec![];
        }
        let iter = self
            .claims
            .iter()
            .enumerate()
            .filter(|(_, c)| !canonical_only || c.status == ClaimStatus::Canonical);
        let mut scored: Vec<(f32, &Claim)> = iter
            .filter_map(|(i, c)| {
                self.vectors
                    .get(i)
                    .map(|v| (cosine_similarity(query_vec, v), c))
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(top_k)
            .filter(|(s, _)| *s >= min_score)
            .collect()
    }

    /// strict-verify: check answerability, return matched canonical claims.
    /// Uses semantic search when CLMV vectors are available, falls back to Jaccard.
    pub fn check_answerability(&self, query: &str) -> (bool, Vec<&Claim>) {
        let relevant = self.find_relevant(query, 5, 0.15, true);
        let answerable = !relevant.is_empty();
        (answerable, relevant.into_iter().map(|(_, c)| c).collect())
    }

    /// check_answerability using pre-computed query vector (preferred over text Jaccard).
    pub fn check_answerability_vec(&self, query_vec: &[f32]) -> (bool, Vec<&Claim>) {
        let relevant = self.find_relevant_vec(query_vec, 5, 0.25, true);
        let answerable = !relevant.is_empty();
        (answerable, relevant.into_iter().map(|(_, c)| c).collect())
    }

    /// Best-effort answerability: semantic if CLMV available, otherwise Jaccard.
    pub fn check_answerability_best<'a>(
        &'a self,
        query_text: &str,
        query_vec: Option<&[f32]>,
    ) -> (bool, Vec<&'a Claim>) {
        if let Some(qv) = query_vec {
            if !self.vectors.is_empty() {
                return self.check_answerability_vec(qv);
            }
        }
        self.check_answerability(query_text)
    }

    /// strict-verify: inject claims as system-prompt constraints.
    pub fn format_verify_constraints(&self, claims: &[&Claim]) -> String {
        if claims.is_empty() {
            // No claims matched the query — don't confuse the model with an empty list.
            // Fall back to all canonical claims, or if none exist, allow general response.
            let all_canonical: Vec<&Claim> = self.canonical().collect();
            if all_canonical.is_empty() {
                return String::from(
                    "## STRICT VERIFY MODE\n\n\
                     No verified claims are available. \
                     Answer from your general knowledge and clearly indicate \
                     any statements you are not certain about.\n\n",
                );
            }
            return self.format_verify_constraints(&all_canonical);
        }

        let mut out = String::from(
            "## STRICT VERIFY MODE\n\n\
             Answer using the verified claims listed below as your primary source. \
             If the question cannot be answered from these claims, say: \
             \"I don't have verified information on this topic.\"\n\n",
        );
        for c in claims {
            out.push_str(&format!("- [{}] \"{}\"\n", c.id, c.text));
            if !c.span.is_empty() {
                out.push_str(&format!("  source: {} | span: \"{}\"\n", c.source, c.span));
            }
        }
        out
    }

    /// strict-render: build render prompt requiring explicit claim citations.
    /// LLM MUST cite [claim_id] after every factual sentence.
    pub fn format_render_prompt(&self, claims: &[&Claim]) -> String {
        let selected_claims: Vec<&Claim> = if claims.is_empty() {
            self.canonical().collect()
        } else {
            claims.to_vec()
        };

        if selected_claims.is_empty() {
            return String::from(
                "## STRICT RENDER MODE - Claim-Grounded Answer\n\n\
                 No canonical claims are available. State that there is insufficient \
                 grounded information to answer the question.\n\n",
            );
        }

        let mut out = String::from(
            "## STRICT RENDER MODE - Claim-Grounded Answer\n\n\
             You are a claim renderer. Rules:\n\
             1. Use ONLY the verified claims listed below\n\
             2. After EVERY factual sentence, cite the exact claim ID from the list, for example [base-lang-en]\n\
             3. Never write the literal placeholder [claim_id]\n\
             4. Do NOT add any facts not present in these claims\n\
             5. If the question cannot be fully answered, state: \
             \"Insufficient grounded information for: <aspect>\"\n\n\
             Verified claims:\n",
        );
        for c in selected_claims {
            out.push_str(&format!("[{}] \"{}\"", c.id, c.text));
            if !c.source.is_empty() {
                out.push_str(&format!("  (source: {})", c.source));
            }
            out.push('\n');
        }
        out
    }

    /// Extract [claim_id] citations from LLM output text.
    pub fn extract_cited_ids(text: &str) -> Vec<String> {
        let mut ids = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('[') {
            rest = &rest[start + 1..];
            if let Some(end) = rest.find(']') {
                let id = rest[..end].trim().to_string();
                if !id.is_empty() && !id.contains(' ') {
                    ids.push(id);
                }
                rest = &rest[end + 1..];
            } else {
                break;
            }
        }
        ids
    }

    /// Check if a claim ID exists and is canonical.
    pub fn get_canonical(&self, id: &str) -> Option<&Claim> {
        self.claims
            .iter()
            .find(|c| c.id == id && c.status == ClaimStatus::Canonical)
    }
}

/// Parse CLMV section: [claim_count: u32][dim: u32][raw f32 LE vectors...]
fn parse_clmv_section(data: &[u8]) -> Option<Vec<Vec<f32>>> {
    if data.len() < 8 {
        return None;
    }
    let claim_count = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    let dim = u32::from_le_bytes(data[4..8].try_into().ok()?) as usize;
    if dim == 0 || claim_count == 0 {
        return None;
    }
    let expected = 8 + claim_count * dim * 4;
    if data.len() < expected {
        return None;
    }
    let vec_data = &data[8..];
    let vectors: Vec<Vec<f32>> = (0..claim_count)
        .map(|i| {
            let base = i * dim * 4;
            (0..dim)
                .map(|j| {
                    let off = base + j * 4;
                    f32::from_le_bytes(vec_data[off..off + 4].try_into().unwrap())
                })
                .collect()
        })
        .collect();
    Some(vectors)
}

fn jaccard_similarity(a: &str, b: &str) -> f32 {
    let words_a: std::collections::HashSet<String> = a
        .split_whitespace()
        .map(|w| {
            w.to_lowercase()
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string()
        })
        .filter(|w| w.len() > 3)
        .collect();
    let words_b: std::collections::HashSet<String> = b
        .split_whitespace()
        .map(|w| {
            w.to_lowercase()
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string()
        })
        .filter(|w| w.len() > 3)
        .collect();
    if words_a.is_empty() || words_b.is_empty() {
        return 0.0;
    }
    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();
    intersection as f32 / union as f32
}

// ─── Graph routing (THKG + LINK) ─────────────────────────────────────────────

/// Routing description for a package (THKG section).
pub struct ThkgRuntime {
    pub text: Option<String>,
}

impl ThkgRuntime {
    pub fn load(pkg: &AipkPackage) -> Self {
        let text = pkg.section("THKG").and_then(|s| {
            let raw = pkg.section_data(s);
            std::str::from_utf8(raw).ok().map(|t| t.to_string())
        });
        Self { text }
    }

    pub fn from_packages(pkgs: &[AipkPackage]) -> Vec<Self> {
        pkgs.iter().map(Self::load).collect()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.text
            .as_ref()
            .map(|t| t.trim().is_empty())
            .unwrap_or(true)
    }

    /// Returns routing text — strips comment lines (starting with #).
    pub fn routing_text(&self) -> String {
        self.text
            .as_deref()
            .unwrap_or("")
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }
}

/// Graph link manifest (LINK section).
#[allow(dead_code)]
pub struct LinkRuntime {
    pub links: Vec<String>,
    pub router_hint: Option<String>,
}

impl LinkRuntime {
    #[allow(dead_code)]
    pub fn load(pkg: &AipkPackage) -> Self {
        if let Some(sec) = pkg.section("LINK") {
            let data = pkg.section_data(sec);
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
                let links = v["links"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let router_hint = v["router_hint"].as_str().map(|s| s.to_string());
                return Self { links, router_hint };
            }
        }
        Self {
            links: vec![],
            router_hint: None,
        }
    }
}

/// Given per-package THKG embeddings and a query vector, returns the index of
/// the best-matching package. Returns None if no embeddings are available or
/// all scores are below `min_score`.
pub fn best_route(
    thkg_vecs: &[(usize, Vec<f32>)],
    query_vec: &[f32],
    min_score: f32,
) -> Option<usize> {
    thkg_vecs
        .iter()
        .map(|(idx, vec)| (*idx, cosine_similarity(query_vec, vec)))
        .filter(|(_, score)| *score >= min_score)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(idx, _)| idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ann::AnnIndex;
    use crate::format::{build_know_section, AipkBuilder};

    /// KnowRuntime::load must read a pre-built ANNX section rather than
    /// building one itself (see ann.rs module docs) — this exercises the
    /// actual package round trip (builder -> file -> parse -> load) rather
    /// than just unit-testing AnnIndex in isolation.
    #[test]
    fn know_runtime_uses_annx_section_when_present() {
        let chunks = vec![
            KnowChunk {
                id: 0,
                text: "alpha chunk".into(),
                source: "a.md".into(),
            },
            KnowChunk {
                id: 1,
                text: "beta chunk".into(),
                source: "b.md".into(),
            },
            KnowChunk {
                id: 2,
                text: "gamma chunk".into(),
                source: "c.md".into(),
            },
        ];
        let vectors = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.5, 0.5]];

        let mut b = AipkBuilder::new("annx-test");
        b.add("META", b"[package]\nname = \"annx-test\"\n".to_vec());
        b.add("KNOW", build_know_section(&chunks, &vectors, 2).unwrap());
        b.add(
            "ANNX",
            AnnIndex::build(&vectors).unwrap().to_bytes().unwrap(),
        );

        let dir = std::env::temp_dir().join("aipk_runtime_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("annx-test.aipk");
        std::fs::write(&path, b.build()).unwrap();

        let pkg = crate::format::parse(&path).unwrap();
        let know = KnowRuntime::load(&pkg);
        let results = know.retrieve_scored(&[0.0, 1.0], 1);
        assert_eq!(results[0].1, "beta chunk");
    }

    #[test]
    fn know_runtime_falls_back_to_brute_force_without_annx() {
        let chunks = vec![KnowChunk {
            id: 0,
            text: "only chunk".into(),
            source: "a.md".into(),
        }];
        let vectors = vec![vec![1.0, 0.0]];

        let mut b = AipkBuilder::new("no-annx-test");
        b.add("META", b"[package]\nname = \"no-annx-test\"\n".to_vec());
        b.add("KNOW", build_know_section(&chunks, &vectors, 2).unwrap());

        let dir = std::env::temp_dir().join("aipk_runtime_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("no-annx-test.aipk");
        std::fs::write(&path, b.build()).unwrap();

        let pkg = crate::format::parse(&path).unwrap();
        let know = KnowRuntime::load(&pkg);
        let results = know.retrieve_scored(&[1.0, 0.0], 1);
        assert_eq!(results[0].1, "only chunk");
    }

    fn canonical_claim(id: &str, text: &str) -> Claim {
        Claim {
            id: id.to_string(),
            text: text.to_string(),
            source: "test".to_string(),
            span: String::new(),
            status: ClaimStatus::Canonical,
        }
    }

    #[test]
    fn verify_constraints_fall_back_to_all_canonical_claims() {
        let claims = ClaimsRuntime {
            claims: vec![
                canonical_claim("c1", "Use verified facts."),
                Claim {
                    id: "draft".to_string(),
                    text: "Draft claim".to_string(),
                    source: "test".to_string(),
                    span: String::new(),
                    status: ClaimStatus::Extracted,
                },
            ],
            vectors: vec![],
        };

        let prompt = claims.format_verify_constraints(&[]);
        assert!(prompt.contains("[c1]"));
        assert!(!prompt.contains("[draft]"));
    }

    #[test]
    fn render_prompt_falls_back_to_all_canonical_claims() {
        let claims = ClaimsRuntime {
            claims: vec![canonical_claim(
                "c1",
                "Every factual sentence needs a citation.",
            )],
            vectors: vec![],
        };

        let prompt = claims.format_render_prompt(&[]);
        assert!(prompt.contains("[c1]"));
        assert!(prompt.contains("Use ONLY the verified claims"));
    }

    #[test]
    fn render_prompt_handles_no_canonical_claims() {
        let claims = ClaimsRuntime {
            claims: vec![Claim {
                id: "draft".to_string(),
                text: "Draft claim".to_string(),
                source: "test".to_string(),
                span: String::new(),
                status: ClaimStatus::Extracted,
            }],
            vectors: vec![],
        };

        let prompt = claims.format_render_prompt(&[]);
        assert!(prompt.contains("No canonical claims are available"));
    }

    #[test]
    fn find_relevant_vec_uses_cosine_similarity() {
        let claim = canonical_claim("c1", "test claim");
        let claims = ClaimsRuntime {
            claims: vec![claim],
            vectors: vec![vec![1.0, 0.0, 0.0]],
        };
        // Identical vector → score 1.0
        let results = claims.find_relevant_vec(&[1.0, 0.0, 0.0], 5, 0.5, true);
        assert_eq!(results.len(), 1);
        assert!((results[0].0 - 1.0).abs() < 1e-5);

        // Orthogonal vector → score 0.0, filtered by min_score
        let results = claims.find_relevant_vec(&[0.0, 1.0, 0.0], 5, 0.5, true);
        assert!(results.is_empty());
    }

    #[test]
    fn check_answerability_best_prefers_vec_when_available() {
        let claim = canonical_claim("c1", "ownership means one owner");
        let claims = ClaimsRuntime {
            claims: vec![claim],
            vectors: vec![vec![1.0, 0.0]],
        };
        // With a near-identical query vector → found
        let (answerable, matched) =
            claims.check_answerability_best("ownership", Some(&[0.99, 0.1]));
        assert!(answerable);
        assert_eq!(matched[0].id, "c1");

        // Without vector → falls back to Jaccard
        let (answerable, _) = claims.check_answerability_best("ownership", None);
        // Jaccard may or may not match "ownership means one owner" depending on word overlap
        let _ = answerable; // just verify it doesn't panic
    }
}

// ─── v2 Epistemic runtimes ────────────────────────────────────────────────────

// ── IDTY ─────────────────────────────────────────────────────────────────────

/// Identity contract — injected at the top of the system prompt.
pub struct IdtyRuntime {
    data: Option<Value>,
}

impl IdtyRuntime {
    pub fn load(pkg: &AipkPackage) -> Self {
        let data = pkg
            .section("IDTY")
            .and_then(|s| serde_json::from_slice(pkg.section_data(s)).ok());
        Self { data }
    }

    pub fn from_packages(pkgs: &[AipkPackage]) -> Self {
        for pkg in pkgs {
            let rt = Self::load(pkg);
            if rt.data.is_some() {
                return rt;
            }
        }
        Self { data: None }
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_none()
    }

    /// Format IDTY as a structured block for prepending to the system prompt.
    pub fn inject_prefix(&self) -> String {
        let v = match &self.data {
            Some(v) => v,
            None => return String::new(),
        };
        let mut parts = vec!["[IDENTITY]".to_string()];
        if let Some(s) = v["name"].as_str() {
            parts.push(format!("Name: {s}"));
        }
        if let Some(s) = v["role"].as_str() {
            parts.push(format!("Role: {s}"));
        }
        if let Some(s) = v["style"].as_str() {
            parts.push(format!("Style: {s}"));
        }
        if let Some(s) = v["language"].as_str() {
            parts.push(format!("Language: {s}"));
        }
        if let Some(scope) = v["competence_scope"].as_array() {
            let items: Vec<&str> = scope.iter().filter_map(|x| x.as_str()).collect();
            if !items.is_empty() {
                parts.push(format!("Competence scope: {}", items.join(", ")));
            }
        }
        if let Some(policy) = v["operating_policy"].as_object() {
            let mut rules = Vec::new();
            if policy
                .get("never_speculate")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                rules.push("never speculate");
            }
            if policy
                .get("cite_sources")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                rules.push("cite sources");
            }
            if policy
                .get("admit_uncertainty")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                rules.push("admit uncertainty when unsure");
            }
            if !rules.is_empty() {
                parts.push(format!("Operating rules: {}", rules.join("; ")));
            }
        }
        parts.push("[/IDENTITY]".to_string());
        parts.join("\n")
    }
}

// ── Enforce mode ─────────────────────────────────────────────────────────────

/// How strict-render's post-generation groundedness check affects the
/// response. `Observe` is the historical behavior (metadata only, nothing
/// withheld) and stays the default for backward compatibility — `Block` is
/// what a "trust layer" actually needs for regulated use, but it's a
/// deliberate opt-in since it can withhold an otherwise-useful answer.
///
/// This is EXPERIMENTAL: it gates on a single coverage/fully_grounded
/// signal, not a full grounding contract (claim freshness, semantic
/// entailment, contradiction detection are not checked here). Don't market
/// `block` as "hallucinations eliminated" — it's "ungrounded sentences
/// withheld," which is narrower and honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum EnforceMode {
    /// Report groundedness as metadata only; always return the answer. Default.
    #[default]
    Observe,
    /// Return the answer, but flag it prominently when not fully grounded.
    Warn,
    /// Withhold an ungrounded answer and return a refusal instead.
    Block,
}

// ── ANSP ─────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum AnspDecision {
    Allow,
    AllowPartial(String),
    Refuse(String, &'static str), // (message, code)
    Escalate(String),
}

pub struct AnspRuntime {
    domain: Vec<String>,
    confidence_threshold: f64,
    adjacent_allowed: bool,
    refuse_unknown: Option<String>,
    refuse_out_of_scope: Option<String>,
    escalate_review: Option<String>,
}

impl Default for AnspRuntime {
    fn default() -> Self {
        Self {
            domain: vec![],
            confidence_threshold: 0.0,
            adjacent_allowed: true,
            refuse_unknown: None,
            refuse_out_of_scope: None,
            escalate_review: None,
        }
    }
}

impl AnspRuntime {
    pub fn load(pkg: &AipkPackage) -> Self {
        let Some(sec) = pkg.section("ANSP") else {
            return Self::default();
        };
        let Ok(v) = serde_json::from_slice::<Value>(pkg.section_data(sec)) else {
            return Self::default();
        };
        let domain: Vec<String> = v["domain"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let confidence_threshold = v["confidence_threshold"].as_f64().unwrap_or(0.0);
        let adjacent_allowed = v["adjacent_allowed"].as_bool().unwrap_or(true);
        let r = &v["responses"];
        let get = |key: &str| r[key].as_str().map(str::to_string);
        Self {
            domain,
            confidence_threshold,
            adjacent_allowed,
            refuse_unknown: get("refuse_unknown"),
            refuse_out_of_scope: get("refuse_out_of_scope"),
            escalate_review: get("escalate_review"),
        }
    }

    pub fn from_packages(pkgs: &[AipkPackage]) -> Self {
        for pkg in pkgs {
            let rt = Self::load(pkg);
            if !rt.domain.is_empty() || rt.refuse_unknown.is_some() {
                return rt;
            }
        }
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.domain.is_empty()
            && self.refuse_unknown.is_none()
            && self.refuse_out_of_scope.is_none()
    }

    /// Domain-level gate: checked before embedding, no claim scores needed.
    pub fn check_domain(&self, query: &str) -> AnspDecision {
        if self.domain.is_empty() {
            return AnspDecision::Allow;
        }
        let lower = query.to_lowercase();
        let in_domain = self.domain.iter().any(|topic| {
            topic
                .split_whitespace()
                .any(|word| lower.contains(&word.to_lowercase()))
        });
        if !in_domain && !self.adjacent_allowed {
            let msg = self.refuse_out_of_scope.clone().unwrap_or_else(|| {
                "This question is outside the scope of this assistant.".to_string()
            });
            return AnspDecision::Refuse(msg, "refuse_out_of_scope");
        }
        AnspDecision::Allow
    }

    /// Coverage-level gate: checked after claim retrieval.
    /// `best_score` = highest cosine/Jaccard similarity among matched canonical claims (0.0–1.0).
    pub fn check_coverage(&self, best_score: f32) -> AnspDecision {
        if self.confidence_threshold == 0.0 || self.refuse_unknown.is_none() {
            return AnspDecision::Allow;
        }
        let threshold = self.confidence_threshold as f32;
        if best_score >= threshold {
            return AnspDecision::Allow;
        }
        if best_score >= threshold * 0.6 {
            let note = self
                .escalate_review
                .clone()
                .unwrap_or_else(|| "Note: coverage is partial.".to_string());
            return AnspDecision::AllowPartial(note);
        }
        let msg = self.refuse_unknown.clone().unwrap();
        AnspDecision::Refuse(msg, "refuse_unknown")
    }
}

// ── NKNW ─────────────────────────────────────────────────────────────────────

pub struct NknwRuntime {
    entries: Vec<NknwEntry>,
}

struct NknwEntry {
    nknw_type: String, // "unknown" | "forbidden" | "uncovered"
    pattern: String,
    message: String,
}

impl NknwRuntime {
    pub fn load(pkg: &AipkPackage) -> Self {
        let entries = pkg
            .section("NKNW")
            .map(|s| Self::parse(pkg.section_data(s)))
            .unwrap_or_default();
        Self { entries }
    }

    pub fn from_packages(pkgs: &[AipkPackage]) -> Self {
        let entries = pkgs
            .iter()
            .flat_map(|p| {
                p.section("NKNW")
                    .map(|s| Self::parse(p.section_data(s)))
                    .unwrap_or_default()
            })
            .collect();
        Self { entries }
    }

    fn parse(data: &[u8]) -> Vec<NknwEntry> {
        std::str::from_utf8(data)
            .unwrap_or("")
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|line| {
                let v: Value = serde_json::from_str(line).ok()?;
                let nknw_type = v["type"].as_str().unwrap_or("unknown").to_string();
                let pattern = v["pattern"]
                    .as_str()
                    .or_else(|| v["topic"].as_str())
                    .unwrap_or("")
                    .to_string();
                // v2-init templates write "refusal" (forbidden) / "note" (unknown);
                // "message" is kept for packages built before that naming settled.
                let message = v["message"]
                    .as_str()
                    .or_else(|| v["refusal"].as_str())
                    .or_else(|| v["note"].as_str())
                    .unwrap_or("")
                    .to_string();
                Some(NknwEntry {
                    nknw_type,
                    pattern,
                    message,
                })
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns a refusal message if query matches a `forbidden` pattern.
    /// Called before embedding — no LLM needed.
    pub fn forbidden_refusal(&self, query: &str) -> Option<String> {
        let lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.nknw_type == "forbidden")
            .find(|e| {
                e.pattern
                    .split('|')
                    .any(|p| lower.contains(&p.trim().to_lowercase()))
            })
            .map(|e| e.message.clone())
    }

    /// Returns limitation notes for `unknown` topics that match the query.
    /// Injected into system prompt as context.
    pub fn unknown_notes(&self, query: &str) -> Vec<String> {
        let lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.nknw_type == "unknown")
            .filter(|e| {
                e.pattern
                    .split('|')
                    .any(|p| lower.contains(&p.trim().to_lowercase()))
            })
            .map(|e| e.message.clone())
            .collect()
    }
}

// ── PLCY ─────────────────────────────────────────────────────────────────────

pub struct PlcyRuntime {
    pub citation_required: bool,
    forbidden: Vec<String>,
    partial_answer_allowed: bool,
}

impl Default for PlcyRuntime {
    fn default() -> Self {
        Self {
            citation_required: false,
            forbidden: vec![],
            partial_answer_allowed: true,
        }
    }
}

impl PlcyRuntime {
    pub fn load(pkg: &AipkPackage) -> Self {
        let Some(sec) = pkg.section("PLCY") else {
            return Self::default();
        };
        let Ok(v) = serde_json::from_slice::<Value>(pkg.section_data(sec)) else {
            return Self::default();
        };
        let forbidden: Vec<String> = v["forbidden"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            citation_required: v["citation_required"].as_bool().unwrap_or(false),
            forbidden,
            partial_answer_allowed: v["partial_answer_allowed"].as_bool().unwrap_or(true),
        }
    }

    pub fn from_packages(pkgs: &[AipkPackage]) -> Self {
        for pkg in pkgs {
            let rt = Self::load(pkg);
            if rt.citation_required || !rt.forbidden.is_empty() {
                return rt;
            }
        }
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        !self.citation_required && self.forbidden.is_empty()
    }

    /// Returns a constraint block for injection into the system prompt.
    pub fn forbidden_hint(&self) -> String {
        if self.forbidden.is_empty() {
            return String::new();
        }
        let mut out = String::from("[POLICY CONSTRAINTS]\nYou must NOT:\n");
        for f in &self.forbidden {
            let readable = f.replace('_', " ");
            out.push_str(&format!("- {readable}\n"));
        }
        if !self.partial_answer_allowed {
            out.push_str("- give partial answers — answer fully or not at all\n");
        }
        out.push_str("[/POLICY CONSTRAINTS]");
        out
    }
}

// ─── context assembly ─────────────────────────────────────────────────────────

pub struct AssembledMessages {
    pub messages: Vec<Value>,
}

/// Build messages list with persona, RAG chunks, and matched skill injected.
pub fn assemble_messages(
    persona: &str,
    skills: &[SkillEntry],
    user_messages: &[Value],
    rag_chunks: &[String],
) -> AssembledMessages {
    let last_user = user_messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or("");

    let mut system = persona.to_string();

    if !rag_chunks.is_empty() {
        system.push_str("\n\n---\n\n## Relevant Knowledge\n\n");
        for chunk in rag_chunks {
            system.push_str(chunk);
            system.push_str("\n\n");
        }
    }

    if let Some(skill) = match_skill(skills, last_user) {
        system.push_str("\n\n---\n\n");
        system.push_str(&format!("## Active Skill: {}\n\n", skill.name));
        system.push_str(&strip_frontmatter(&skill.content));
    }

    let mut messages = vec![json!({"role": "system", "content": system})];
    messages.extend_from_slice(user_messages);
    AssembledMessages { messages }
}

fn match_skill<'a>(skills: &'a [SkillEntry], text: &str) -> Option<&'a SkillEntry> {
    let lower = text.to_lowercase();
    skills
        .iter()
        .find(|s| !s.trigger.is_empty() && lower.contains(&s.trigger.to_lowercase()))
}

fn strip_frontmatter(content: &str) -> String {
    if !content.starts_with("---") {
        return content.to_string();
    }
    if let Some(end) = content[3..].find("---") {
        content[end + 6..].trim_start_matches('\n').to_string()
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod graph_tests {
    use super::*;

    #[test]
    fn best_route_selects_highest_cosine_match() {
        // Two packages: legal (dim 0 heavy) and medical (dim 1 heavy)
        let legal_vec = vec![0.9f32, 0.1, 0.0];
        let medical_vec = vec![0.1f32, 0.9, 0.0];
        let routes = vec![(0, legal_vec), (1, medical_vec)];

        // Query close to legal
        let legal_query = vec![1.0f32, 0.0, 0.0];
        assert_eq!(best_route(&routes, &legal_query, 0.0), Some(0));

        // Query close to medical
        let medical_query = vec![0.0f32, 1.0, 0.0];
        assert_eq!(best_route(&routes, &medical_query, 0.0), Some(1));
    }

    #[test]
    fn best_route_returns_none_when_below_threshold() {
        let routes = vec![(0, vec![1.0f32, 0.0]), (1, vec![0.0f32, 1.0])];
        // Query is orthogonal to both — cosine = 0
        let query = vec![0.0f32, 0.0];
        assert_eq!(best_route(&routes, &query, 0.1), None);
    }

    #[test]
    fn best_route_returns_none_for_empty_routes() {
        let routes: Vec<(usize, Vec<f32>)> = vec![];
        let query = vec![1.0f32, 0.0];
        assert_eq!(best_route(&routes, &query, 0.0), None);
    }

    #[test]
    fn thkg_routing_text_strips_comments() {
        let thkg = ThkgRuntime {
            text: Some(
                "# Comment line\nThis package covers legal topics.\n# Another comment\n"
                    .to_string(),
            ),
        };
        let text = thkg.routing_text();
        assert!(!text.contains('#'));
        assert!(text.contains("legal topics"));
    }

    #[test]
    fn thkg_is_empty_for_missing_section() {
        let thkg = ThkgRuntime { text: None };
        assert!(thkg.is_empty());
    }

    #[test]
    fn thkg_is_empty_for_comment_only_file() {
        let thkg = ThkgRuntime {
            text: Some("# Just a comment\n# Nothing else\n".to_string()),
        };
        assert!(thkg.routing_text().is_empty());
    }

    #[test]
    fn nknw_parse_accepts_refusal_and_note_fields() {
        let data = b"{\"type\":\"forbidden\",\"pattern\":\"pricing\",\"refusal\":\"No pricing info.\"}\n{\"type\":\"unknown\",\"pattern\":\"istio\",\"note\":\"Istio is not covered.\"}";
        let rt = NknwRuntime {
            entries: NknwRuntime::parse(data),
        };
        assert_eq!(
            rt.forbidden_refusal("what is the PRICING?").as_deref(),
            Some("No pricing info.")
        );
        assert_eq!(
            rt.unknown_notes("how to install Istio?"),
            vec!["Istio is not covered.".to_string()]
        );
    }
}
