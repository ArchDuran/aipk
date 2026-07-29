use anyhow::Result;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

use crate::agent::{auth_headers, build_client, run_tool_loop, DEFAULT_MAX_TOKENS};
use crate::format::AipkPackage;
use crate::llm::embed_query;
use crate::mcp_client::McpManager;
use crate::runtime::{assemble_messages, ClaimsRuntime, KnowRuntime};

// ── TEST section schema ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct TestSuite {
    #[serde(default = "default_version")]
    pub version: u32,
    pub tests: Vec<TestCase>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TestCase {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub query: String,
    /// "normal" | "strict-verify" | "strict-render"
    #[serde(default = "default_mode")]
    pub mode: String,
    pub expect: TestExpect,
}

fn default_mode() -> String {
    "normal".to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TestExpect {
    /// Response must contain all of these strings (case-insensitive)
    #[serde(default)]
    pub contains: Vec<String>,
    /// Response must NOT contain any of these strings (case-insensitive)
    #[serde(default)]
    pub not_contains: Vec<String>,
    /// _aipk.fully_grounded must equal this value (strict-render only)
    pub fully_grounded: Option<bool>,
    /// _aipk.coverage must be ≥ this value (strict-render only)
    pub min_coverage: Option<f64>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn run(
    pkg_path: &Path,
    model: Option<&str>,
    llm_url: &str,
    api_key: &str,
    embed_model: &str,
) -> Result<()> {
    let pkg = crate::format::parse(pkg_path)?;

    println!("Testing: {}", pkg_path.display());
    println!();

    // ── Phase 1: static checks ────────────────────────────────────────────────
    println!("{}", "Static checks:".bold());
    let static_ok = run_static_checks(&pkg);
    println!();

    // ── Phase 2: live tests ───────────────────────────────────────────────────
    let suite = match load_test_suite(&pkg) {
        Some(s) => s,
        None => {
            println!(
                "{}",
                "No TEST section — add tests.json to your project and rebuild.".yellow()
            );
            if !static_ok {
                std::process::exit(1);
            }
            return Ok(());
        }
    };

    if suite.tests.is_empty() {
        println!("{}", "TEST section is empty.".yellow());
        if !static_ok {
            std::process::exit(1);
        }
        return Ok(());
    }

    let Some(model) = model else {
        println!(
            "{}",
            format!(
                "TEST section has {} test(s). Pass --model to run live tests.",
                suite.tests.len()
            )
            .yellow()
        );
        if !static_ok {
            std::process::exit(1);
        }
        return Ok(());
    };

    println!("{}", format!("Live tests  (model: {model}):").bold());
    let (passed, total) = run_live_tests(&pkg, &suite, llm_url, model, api_key, embed_model).await;
    println!();

    let all_ok = static_ok && passed == total;
    let summary = format!("{passed}/{total} passed");
    if all_ok {
        println!("{}", summary.green().bold());
    } else {
        println!("{}", summary.red().bold());
        std::process::exit(1);
    }

    Ok(())
}

// ── Static checks ─────────────────────────────────────────────────────────────

fn run_static_checks(pkg: &AipkPackage) -> bool {
    let mut ok = true;

    // 1. Package parses (we got here, so yes)
    let size_str = if pkg.raw.len() >= 1024 {
        format!("{:.1} KB", pkg.raw.len() as f64 / 1024.0)
    } else {
        format!("{} B", pkg.raw.len())
    };
    check(
        true,
        &format!(
            "Package valid  ({}, {} section(s))",
            size_str,
            pkg.sections.len()
        ),
    );

    // 2. META
    match pkg.meta() {
        Ok(meta) => {
            let name = meta
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let ver = meta
                .get("package")
                .and_then(|p| p.get("version"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            check(true, &format!("META  ({name} v{ver})"));
        }
        Err(e) => {
            check(false, &format!("META  ({e})"));
            ok = false;
        }
    }

    // 3. PERS lint (re-use lint logic inline — just check for ERROR lines)
    if let Some(persona) = pkg.persona() {
        let errors = crate::cmd::lint::count_errors(&persona);
        if errors == 0 {
            check(true, "PERS  (lint clean)");
        } else {
            check(
                false,
                &format!("PERS  ({errors} lint error(s) — run `aipk lint` for details)"),
            );
            ok = false;
        }
    }

    // 4. Claims stats
    let claims = ClaimsRuntime::load(pkg);
    let total = claims.claims.len();
    let canonical = claims.canonical_count();
    if total > 0 {
        let msg = format!("CLMS  ({canonical} canonical / {total} total)");
        check(canonical > 0, &msg);
        if canonical == 0 {
            ok = false;
        }
    }

    // 5. SIGN — verify if present
    if let Some(sign_sec) = pkg.section("SIGN") {
        use ed25519_dalek::{Verifier, VerifyingKey};
        let data = pkg.section_data(sign_sec);
        let result = (|| -> Result<(), String> {
            if data.len() < 96 {
                return Err("too small".to_string());
            }
            let pk: [u8; 32] = data[0..32].try_into().unwrap();
            let s: [u8; 64] = data[32..96].try_into().unwrap();
            let vk = VerifyingKey::from_bytes(&pk).map_err(|e| e.to_string())?;
            let sig = ed25519_dalek::Signature::from_bytes(&s);
            let start = sign_sec.offset - crate::format::SECTION_HEADER_SIZE;
            vk.verify(&pkg.raw[..start], &sig)
                .map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => check(true, "SIGN  (Ed25519 signature valid)"),
            Err(e) => {
                check(false, &format!("SIGN  ({e})"));
                ok = false;
            }
        }
    }

    // 6. TEST section present
    if let Some(suite) = load_test_suite(pkg) {
        check(true, &format!("TEST  ({} test case(s))", suite.tests.len()));
    }

    ok
}

fn check(pass: bool, msg: &str) {
    if pass {
        println!("  {}  {}", "✓".green(), msg);
    } else {
        println!("  {}  {}", "✗".red(), msg);
    }
}

// ── Live tests ────────────────────────────────────────────────────────────────

async fn run_live_tests(
    pkg: &AipkPackage,
    suite: &TestSuite,
    llm_url: &str,
    model: &str,
    api_key: &str,
    embed_model: &str,
) -> (usize, usize) {
    let mut passed = 0;
    let total = suite.tests.len();

    for tc in &suite.tests {
        let label = if tc.name.is_empty() {
            tc.id.clone()
        } else {
            format!("{} — {}", tc.id, tc.name)
        };
        let t0 = Instant::now();

        match run_test_case(pkg, tc, llm_url, model, api_key, embed_model).await {
            Ok((response, meta)) => {
                let failures = check_expect(&response, &meta, &tc.expect);
                let elapsed = t0.elapsed().as_millis();
                if failures.is_empty() {
                    println!("  {}  {} ({elapsed}ms)", "✓".green(), label);
                    passed += 1;
                } else {
                    println!("  {}  {} ({elapsed}ms)", "✗".red(), label);
                    for f in &failures {
                        println!("       {}", f.yellow());
                    }
                }
            }
            Err(e) => {
                let elapsed = t0.elapsed().as_millis();
                println!("  {}  {} ({elapsed}ms)", "✗".red(), label);
                println!("       {}", format!("error: {e}").yellow());
            }
        }
    }

    (passed, total)
}

async fn run_test_case(
    pkg: &AipkPackage,
    tc: &TestCase,
    llm_url: &str,
    model: &str,
    api_key: &str,
    embed_model: &str,
) -> Result<(String, TestMeta)> {
    let know = KnowRuntime::load(pkg);
    let claims = ClaimsRuntime::load(pkg);
    let mut mcp = McpManager::new().load_from_pkg(pkg);

    let strict_verify = tc.mode == "strict-verify";
    let strict_render = tc.mode == "strict-render";

    let query_vec: Option<Vec<f32>> = if !embed_model.is_empty() && !know.is_empty() {
        embed_query(llm_url, embed_model, &tc.query, api_key)
            .await
            .ok()
    } else {
        None
    };

    let rag_chunks = if !know.is_empty() {
        query_vec
            .as_ref()
            .map(|qv| know.retrieve(qv, 5))
            .unwrap_or_default()
    } else {
        vec![]
    };

    let persona_base = pkg.persona().unwrap_or_default();
    let persona = if strict_verify && !claims.is_empty() {
        let (_, matched) = claims.check_answerability_best(&tc.query, query_vec.as_deref());
        let c = claims.format_verify_constraints(&matched);
        format!("{persona_base}\n\n{c}")
    } else if strict_render && !claims.is_empty() {
        let (_, matched) = claims.check_answerability_best(&tc.query, query_vec.as_deref());
        let r = claims.format_render_prompt(&matched);
        format!("{persona_base}\n\n{r}")
    } else {
        persona_base
    };

    let user_msg = json!({"role": "user", "content": tc.query});
    let assembled = assemble_messages(&persona, &pkg.skills(), &[user_msg], &rag_chunks);

    let client = build_client();
    let url = format!("{}/v1/chat/completions", llm_url.trim_end_matches('/'));
    let headers = auth_headers(api_key);

    let (_, result) = run_tool_loop(
        &client,
        &url,
        headers,
        &mut mcp,
        assembled.messages,
        model,
        1.0,
        DEFAULT_MAX_TOKENS,
        false,
    )
    .await?;

    let content = result["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let meta = if strict_render && !claims.is_empty() {
        let cited_ids = ClaimsRuntime::extract_cited_ids(&content);
        let invalid: Vec<String> = cited_ids
            .iter()
            .filter(|id| claims.get_canonical(id).is_none())
            .cloned()
            .collect();
        let canonical_used: usize = cited_ids
            .iter()
            .filter(|id| claims.get_canonical(id).is_some())
            .count();
        let total_canonical = claims.canonical_count();
        let coverage = if total_canonical > 0 {
            canonical_used as f64 / total_canonical as f64
        } else {
            1.0
        };
        TestMeta {
            fully_grounded: Some(invalid.is_empty()),
            coverage: Some(coverage),
        }
    } else {
        TestMeta::default()
    };

    Ok((content, meta))
}

#[derive(Default)]
struct TestMeta {
    fully_grounded: Option<bool>,
    coverage: Option<f64>,
}

fn check_expect(response: &str, meta: &TestMeta, expect: &TestExpect) -> Vec<String> {
    let mut failures = Vec::new();
    let lower = response.to_lowercase();

    for needle in &expect.contains {
        if !lower.contains(&needle.to_lowercase()) {
            failures.push(format!(
                "expected contains {:?}, not found in response",
                needle
            ));
        }
    }
    for needle in &expect.not_contains {
        if lower.contains(&needle.to_lowercase()) {
            failures.push(format!("expected NOT contains {:?}, but found it", needle));
        }
    }
    if let Some(expected_grounded) = expect.fully_grounded {
        match meta.fully_grounded {
            Some(actual) if actual == expected_grounded => {}
            Some(actual) => failures.push(format!(
                "expected fully_grounded={expected_grounded}, got {actual}"
            )),
            None => failures.push("fully_grounded check requires mode=strict-render".to_string()),
        }
    }
    if let Some(min_cov) = expect.min_coverage {
        match meta.coverage {
            Some(actual) if actual >= min_cov => {}
            Some(actual) => failures.push(format!(
                "expected min_coverage={min_cov:.2}, got {actual:.2}"
            )),
            None => failures.push("min_coverage check requires mode=strict-render".to_string()),
        }
    }

    failures
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn load_test_suite(pkg: &AipkPackage) -> Option<TestSuite> {
    let sec = pkg.section("TEST")?;
    let data = pkg.section_data(sec);
    serde_json::from_slice::<Value>(data).ok().and_then(|v| {
        // Support bare array or { version, tests: [...] }
        if v.is_array() {
            let tests: Vec<TestCase> = serde_json::from_value(v).ok()?;
            Some(TestSuite { version: 1, tests })
        } else {
            serde_json::from_value(v).ok()
        }
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_expect(contains: &[&str], not_contains: &[&str]) -> TestExpect {
        TestExpect {
            contains: contains.iter().map(|s| s.to_string()).collect(),
            not_contains: not_contains.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn check_contains_pass() {
        let expect = make_expect(&["hello", "world"], &[]);
        let failures = check_expect("Hello World!", &TestMeta::default(), &expect);
        assert!(failures.is_empty());
    }

    #[test]
    fn check_contains_fail() {
        let expect = make_expect(&["missing"], &[]);
        let failures = check_expect("Hello World!", &TestMeta::default(), &expect);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("missing"));
    }

    #[test]
    fn check_not_contains_fail() {
        let expect = make_expect(&[], &["world"]);
        let failures = check_expect("Hello World!", &TestMeta::default(), &expect);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("world"));
    }

    #[test]
    fn check_fully_grounded_pass() {
        let expect = TestExpect {
            fully_grounded: Some(true),
            ..Default::default()
        };
        let meta = TestMeta {
            fully_grounded: Some(true),
            coverage: None,
        };
        assert!(check_expect("response", &meta, &expect).is_empty());
    }

    #[test]
    fn check_fully_grounded_fail() {
        let expect = TestExpect {
            fully_grounded: Some(true),
            ..Default::default()
        };
        let meta = TestMeta {
            fully_grounded: Some(false),
            coverage: None,
        };
        let failures = check_expect("response", &meta, &expect);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("fully_grounded"));
    }

    #[test]
    fn check_min_coverage_pass() {
        let expect = TestExpect {
            min_coverage: Some(0.8),
            ..Default::default()
        };
        let meta = TestMeta {
            fully_grounded: None,
            coverage: Some(0.9),
        };
        assert!(check_expect("response", &meta, &expect).is_empty());
    }

    #[test]
    fn check_min_coverage_fail() {
        let expect = TestExpect {
            min_coverage: Some(0.8),
            ..Default::default()
        };
        let meta = TestMeta {
            fully_grounded: None,
            coverage: Some(0.5),
        };
        let failures = check_expect("response", &meta, &expect);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("coverage"));
    }

    #[test]
    fn test_suite_deserialize_object() {
        let json = r#"{"version":1,"tests":[{"id":"t1","query":"hello?","expect":{}}]}"#;
        let suite: TestSuite = serde_json::from_str(json).unwrap();
        assert_eq!(suite.tests.len(), 1);
        assert_eq!(suite.tests[0].id, "t1");
        assert_eq!(suite.tests[0].mode, "normal");
    }

    #[test]
    fn test_suite_deserialize_array() {
        let json = r#"[{"id":"t1","query":"hello?","expect":{"contains":["42"]}}]"#;
        let v: Value = serde_json::from_str(json).unwrap();
        let tests: Vec<TestCase> = serde_json::from_value(v).unwrap();
        assert_eq!(tests[0].expect.contains, vec!["42"]);
    }
}
