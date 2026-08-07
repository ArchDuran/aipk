use anyhow::Result;
use serde_json::json;
use std::path::Path;

use crate::agent::{auth_headers, build_client, run_tool_loop};
use crate::crypto::load_package;
use crate::llm::embed_query;
use crate::mcp_client::McpManager;
use crate::runtime::{
    assemble_messages, AnspDecision, AnspRuntime, ClaimsRuntime, IdtyRuntime, KnowRuntime,
    NknwRuntime, PlcyRuntime,
};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    pkg_path: &Path,
    query: &str,
    llm_url: &str,
    embed_url: Option<&str>,
    model: &str,
    api_key: &str,
    embed_model: &str,
    temperature: f32,
    max_tokens: u32,
    verbose: bool,
    strict_verify: bool,
    strict_render: bool,
    passphrase: Option<&str>,
    trust_tools: bool,
    allow: &[String],
    allow_tool: &[String],
    enforce: crate::runtime::EnforceMode,
) -> Result<()> {
    let embed_url = embed_url.unwrap_or(llm_url);
    let pkg = load_package(pkg_path, passphrase)?;
    let know = KnowRuntime::load(&pkg);
    let claims = ClaimsRuntime::load(&pkg);
    let idty = IdtyRuntime::load(&pkg);
    let ansp = AnspRuntime::load(&pkg);
    let nknw = NknwRuntime::load(&pkg);
    let plcy = PlcyRuntime::load(&pkg);
    let mut mcp = McpManager::new()
        .load_from_pkg(&pkg)
        .with_allowed_tools(allow_tool);
    if !mcp.confirm_launch(trust_tools, allow) {
        anyhow::bail!("MCP server launch declined");
    }
    let started = mcp.start_all();

    if verbose {
        eprintln!(
            "Package: {}  skills={}  rag={}  claims={}  canonical={}",
            pkg.name,
            pkg.skills().len(),
            !know.is_empty(),
            claims.claims.len(),
            claims.canonical_count(),
        );
        if started > 0 {
            eprintln!(
                "MCP: {started} server(s), {} tools",
                mcp.to_openai_tools().len()
            );
        }
        if !idty.is_empty() {
            eprintln!("IDTY: loaded");
        }
        if !ansp.is_empty() {
            eprintln!("ANSP: loaded");
        }
        if !nknw.is_empty() {
            eprintln!("NKNW: loaded");
        }
        if !plcy.is_empty() {
            eprintln!("PLCY: citation_required={}", plcy.citation_required);
        }
    }

    // Compute query embedding once — used for both RAG and semantic claim matching.
    let query_vec: Option<Vec<f32>> = if !embed_model.is_empty() {
        match embed_query(embed_url, embed_model, query, api_key).await {
            Ok(v) => Some(v),
            Err(e) => {
                if verbose {
                    eprintln!("embed failed: {e}");
                }
                None
            }
        }
    } else {
        None
    };

    // ── NKNW: forbidden topics → early exit ─────────────────────────────────
    if let Some(refusal) = nknw.forbidden_refusal(query) {
        println!("{refusal}");
        return Ok(());
    }

    // ── ANSP: domain gate ────────────────────────────────────────────────────
    if !ansp.is_empty() {
        match ansp.check_domain(query) {
            AnspDecision::Refuse(msg, _) | AnspDecision::Escalate(msg) => {
                println!("{msg}");
                return Ok(());
            }
            _ => {}
        }
    }

    let (rag_chunks, best_rag_score) = if !know.is_empty() {
        if let Some(ref qv) = query_vec {
            let scored = know.retrieve_scored(qv, 5);
            let best = scored.first().map(|(sc, _)| *sc).unwrap_or(0.0);
            let chunks: Vec<String> = scored.into_iter().map(|(_, t)| t).collect();
            if verbose && !chunks.is_empty() {
                eprintln!(
                    "RAG: {} chunks injected (best_score={:.3})",
                    chunks.len(),
                    best
                );
            }
            (chunks, best)
        } else {
            (vec![], 0.0)
        }
    } else {
        (vec![], 0.0)
    };

    // ── ANSP: coverage gate after retrieval ──────────────────────────────────
    if !ansp.is_empty() && !know.is_empty() {
        match ansp.check_coverage(best_rag_score) {
            AnspDecision::Refuse(msg, _) => {
                println!("{msg}");
                return Ok(());
            }
            AnspDecision::AllowPartial(note) if verbose => {
                eprintln!("ANSP partial: {note}");
            }
            _ => {}
        }
    }

    // ── Build persona: IDTY prefix + base + PLCY hints + NKNW notes ─────────
    let persona_base = pkg.persona().unwrap_or_default();
    let nknw_notes = nknw.unknown_notes(query);
    let mut persona_parts: Vec<String> = Vec::new();
    let idty_block = idty.inject_prefix();
    if !idty_block.is_empty() {
        persona_parts.push(idty_block);
    }
    if !persona_base.is_empty() {
        persona_parts.push(persona_base.clone());
    }
    let plcy_block = plcy.forbidden_hint();
    if !plcy_block.is_empty() {
        persona_parts.push(plcy_block);
    }
    if !nknw_notes.is_empty() {
        let notes = nknw_notes
            .iter()
            .map(|n| format!("- {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        persona_parts.push(format!("[KNOWLEDGE GAPS]\n{notes}"));
    }
    let persona_v2_base = persona_parts.join("\n\n");

    let skills = pkg.skills();
    let user_msg = json!({"role": "user", "content": query});

    let persona = if strict_verify && !claims.is_empty() {
        let (_, matched) = claims.check_answerability_best(query, query_vec.as_deref());
        let constraints = claims.format_verify_constraints(&matched);
        format!("{persona_v2_base}\n\n{constraints}")
    } else if strict_render && !claims.is_empty() {
        let (_, matched) = claims.check_answerability_best(query, query_vec.as_deref());
        let render_prompt = claims.format_render_prompt(&matched);
        format!("{persona_v2_base}\n\n{render_prompt}")
    } else {
        persona_v2_base
    };

    let assembled = assemble_messages(&persona, &skills, &[user_msg], &rag_chunks);

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
        temperature,
        max_tokens,
        verbose,
    )
    .await?;

    let content = result["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");

    if strict_render && !claims.is_empty() {
        // Compute the full grounding report BEFORE printing anything — the
        // previous version printed `content` first and scored it after,
        // which meant even --enforce=observe was reporting on an answer
        // the user had already seen. Compute-then-decide, always.
        let cited_ids = ClaimsRuntime::extract_cited_ids(content);
        let mut canonical_used: Vec<String> = cited_ids
            .iter()
            .filter(|id| claims.get_canonical(id).is_some())
            .cloned()
            .collect();
        canonical_used.sort();
        canonical_used.dedup();
        let invalid: Vec<String> = cited_ids
            .iter()
            .filter(|id| claims.get_canonical(id).is_none())
            .cloned()
            .collect();
        // A citation-validity check alone is not a groundedness check: an
        // answer with zero citations would trivially pass it. Coverage must
        // also confirm every factual sentence carries one (see `aipk verify`
        // for the full auditable report).
        let uncited = crate::cmd::serve::collect_unsupported_sentences(content, &claims);
        let total = crate::cmd::serve::count_sentences(content).max(1);
        let coverage = (total - uncited.len().min(total)) as f32 / total as f32;
        let fully_grounded = invalid.is_empty() && uncited.is_empty();

        eprintln!(
            "_aipk: canonical_used={} invalid_ids={:?} coverage={:.2} uncited_sentences={} fully_grounded={}",
            canonical_used.len(),
            invalid,
            coverage,
            uncited.len(),
            fully_grounded
        );

        use crate::runtime::EnforceMode;
        match enforce {
            EnforceMode::Block if !fully_grounded => {
                println!(
                    "This answer could not be fully grounded in canonical claims and was \
                     withheld (--enforce block). See the _aipk report above for detail."
                );
                anyhow::bail!("strict-render: ungrounded answer blocked (--enforce block)");
            }
            EnforceMode::Warn if !fully_grounded => {
                eprintln!(
                    "_aipk: flagged — answer returned but not fully grounded (--enforce warn)"
                );
                println!("{content}");
            }
            _ => println!("{content}"),
        }
    } else {
        println!("{content}");
    }

    Ok(())
}
