use anyhow::Result;
use axum::{
    body::Body,
    extract::State,
    http::{header, Response, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use futures_util::stream::TryStreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

use crate::agent::{auth_headers, build_client, run_tool_loop};
use crate::crypto::load_package;
use crate::llm::embed_query;
use crate::mcp_client::McpManager;
use crate::runtime::{
    assemble_messages, best_route, AnspDecision, AnspRuntime, ClaimsRuntime, ComposedRuntime,
    IdtyRuntime, NknwRuntime, PlcyRuntime, ThkgRuntime,
};

/// Per-package runtimes used in graph routing mode.
struct PerPkgState {
    name: String,
    composed: ComposedRuntime,
    claims: ClaimsRuntime,
    idty: IdtyRuntime,
    ansp: AnspRuntime,
    nknw: NknwRuntime,
    plcy: PlcyRuntime,
}

struct AppState {
    composed: ComposedRuntime,
    claims: ClaimsRuntime,
    idty: IdtyRuntime,
    ansp: AnspRuntime,
    nknw: NknwRuntime,
    plcy: PlcyRuntime,
    /// Per-package runtimes for graph routing (populated when --graph is set).
    per_pkg: Vec<PerPkgState>,
    /// THKG embeddings per package: (pkg_index, embedding). Empty when graph mode is off.
    graph_thkg_vecs: Vec<(usize, Vec<f32>)>,
    graph_mode: bool,
    pkg_names: Vec<String>,
    mcp: Mutex<McpManager>,
    model: String,
    llm_url: String,
    /// Where embedding requests go. Defaults to llm_url; override with --embed-url
    /// when the generation backend (vLLM/SGLang) does not serve embeddings.
    embed_url: String,
    api_key: String,
    embed_model: String,
    lang: Option<String>,
    strict_verify: bool,
    strict_render: bool,
    enforce: crate::runtime::EnforceMode,
    max_tokens: u32,
}

#[derive(Deserialize)]
struct ChatRequest {
    messages: Vec<Value>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    temperature: f32,
    #[serde(default)]
    stream: bool,
    /// Per-request override; falls back to the server's --max-tokens when absent.
    max_tokens: Option<u32>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    package_paths: Vec<PathBuf>,
    model: String,
    llm_url: String,
    embed_url: Option<String>,
    api_key: String,
    embed_model: String,
    lang: Option<String>,
    strict_verify: bool,
    strict_render: bool,
    enforce: crate::runtime::EnforceMode,
    graph_mode: bool,
    max_tokens: u32,
    host: &str,
    port: u16,
    passphrase: Option<&str>,
    trust_tools: bool,
    allow: &[String],
    allow_tool: &[String],
) -> Result<()> {
    let packages: Vec<_> = package_paths
        .iter()
        .map(|p| load_package(p, passphrase))
        .collect::<Result<_>>()?;

    let pkg_names: Vec<String> = packages.iter().map(|p| p.name.clone()).collect();
    let composed = ComposedRuntime::from_packages(&packages);
    let claims = ClaimsRuntime::from_packages(&packages);
    let idty = IdtyRuntime::from_packages(&packages);
    let ansp = AnspRuntime::from_packages(&packages);
    let nknw = NknwRuntime::from_packages(&packages);
    let plcy = PlcyRuntime::from_packages(&packages);

    // ── Per-package runtimes for graph routing ───────────────────────────────
    let per_pkg: Vec<PerPkgState> = packages
        .iter()
        .map(|pkg| {
            let single = std::slice::from_ref(pkg);
            PerPkgState {
                name: pkg.name.clone(),
                composed: ComposedRuntime::from_packages(single),
                claims: ClaimsRuntime::from_packages(single),
                idty: IdtyRuntime::from_packages(single),
                ansp: AnspRuntime::from_packages(single),
                nknw: NknwRuntime::from_packages(single),
                plcy: PlcyRuntime::from_packages(single),
            }
        })
        .collect();

    let embed_url = embed_url.unwrap_or_else(|| llm_url.clone());

    // ── THKG embeddings for routing ──────────────────────────────────────────
    let mut graph_thkg_vecs: Vec<(usize, Vec<f32>)> = vec![];
    if graph_mode && !embed_model.is_empty() {
        let thkgs = ThkgRuntime::from_packages(&packages);
        for (i, thkg) in thkgs.iter().enumerate() {
            let text = thkg.routing_text();
            if !text.is_empty() {
                match embed_query(&embed_url, &embed_model, &text, &api_key).await {
                    Ok(vec) => {
                        eprintln!("Graph: embedded THKG for '{}'", packages[i].name);
                        graph_thkg_vecs.push((i, vec));
                    }
                    Err(e) => {
                        eprintln!(
                            "Graph: failed to embed THKG for '{}': {e}",
                            packages[i].name
                        );
                    }
                }
            }
        }
    }

    let mut mcp = packages
        .iter()
        .fold(McpManager::new(), |m, p| m.load_from_pkg(p))
        .with_allowed_tools(allow_tool);
    if !mcp.confirm_launch(trust_tools, allow) {
        anyhow::bail!("MCP server launch declined");
    }
    let started = mcp.start_all();

    eprintln!("Packages: {}", pkg_names.join(" + "));
    eprintln!(
        "  skills={}  rag={}  claims={}  canonical={}  mcp={}/{}",
        composed.skills().len(),
        composed.has_know(),
        claims.claims.len(),
        claims.canonical_count(),
        started,
        mcp.servers.len(),
    );
    if let Some(ref l) = lang {
        eprintln!("Language: {l}");
    }
    if strict_verify {
        eprintln!("Mode: STRICT-VERIFY (canonical claims injected as constraints)");
    }
    if strict_render {
        eprintln!("Mode: STRICT-RENDER (LLM must cite [claim_id] inline)");
    }
    if graph_mode {
        if graph_thkg_vecs.len() >= 2 {
            eprintln!(
                "Mode: GRAPH (routing across {} packages with THKG)",
                graph_thkg_vecs.len()
            );
        } else {
            eprintln!(
                "Mode: GRAPH requested but only {}/{} packages have THKG — falling back to merged",
                graph_thkg_vecs.len(),
                packages.len()
            );
        }
    }
    eprintln!("LLM: {}  model={}", llm_url, model);
    if embed_url != llm_url {
        eprintln!("Embeddings: {}  model={}", embed_url, embed_model);
    }
    eprintln!("Listening on http://{}:{}/v1/", host, port);

    if !idty.is_empty() {
        eprintln!("IDTY: identity contract loaded");
    }
    if !ansp.is_empty() {
        eprintln!("ANSP: answerability gate loaded");
    }
    if !nknw.is_empty() {
        eprintln!("NKNW: negative knowledge registry loaded");
    }
    if !plcy.is_empty() {
        eprintln!(
            "PLCY: policy loaded (citation_required={})",
            plcy.citation_required
        );
    }

    let state = Arc::new(AppState {
        composed,
        claims,
        idty,
        ansp,
        nknw,
        plcy,
        per_pkg,
        graph_thkg_vecs,
        graph_mode,
        pkg_names,
        mcp: Mutex::new(mcp),
        model,
        llm_url,
        embed_url,
        api_key,
        embed_model,
        lang,
        strict_verify,
        strict_render,
        enforce,
        max_tokens,
    });

    let app = Router::new()
        .route("/", get(chat_page))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/retrieve", post(retrieve))
        .route("/health", get(health))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}")).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn list_models(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "object": "list",
        "data": s.pkg_names.iter().map(|n| json!({"id": n, "object": "model"})).collect::<Vec<_>>()
    }))
}

#[derive(serde::Deserialize)]
struct RetrieveRequest {
    query: String,
    #[serde(default = "default_retrieve_top_k")]
    top_k: usize,
}

fn default_retrieve_top_k() -> usize {
    5
}

/// Raw RAG retrieval for a query: top-k chunks with cosine scores.
/// Used by the Open WebUI pipeline (SHOW_SOURCES) and for package debugging.
async fn retrieve(
    State(s): State<Arc<AppState>>,
    Json(req): Json<RetrieveRequest>,
) -> impl IntoResponse {
    if !s.composed.has_know() {
        return Json(json!({"query": req.query, "chunks": [], "results": []})).into_response();
    }
    match crate::llm::embed_query(&s.embed_url, &s.embed_model, &req.query, &s.api_key).await {
        Ok(qv) => {
            let scored = s.composed.retrieve_scored(&qv, req.top_k);
            let results: Vec<Value> = scored
                .iter()
                .map(|(score, text)| json!({"score": score, "text": text}))
                .collect();
            let chunks: Vec<&String> = scored.iter().map(|(_, text)| text).collect();
            Json(json!({"query": req.query, "chunks": chunks, "results": results})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("embed failed: {e}")})),
        )
            .into_response(),
    }
}

/// Built-in minimal chat UI, served at `/`. Lets a user talk to any loaded
/// package from the browser with zero extra software.
const CHAT_HTML: &str = include_str!("chat.html");

async fn chat_page() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(CHAT_HTML))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn health(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let tools_count = s.mcp.lock().await.to_openai_tools().len();
    Json(json!({
        "status": "ok",
        "packages": s.pkg_names,
        "skills": s.composed.skills().len(),
        "rag": s.composed.has_know(),
        "claims": s.claims.claims.len(),
        "canonical_claims": s.claims.canonical_count(),
        "strict_verify": s.strict_verify,
        "strict_render": s.strict_render,
        "graph_mode": s.graph_mode,
        "graph_packages": s.graph_thkg_vecs.len(),
        "streaming": true,
        "mcp_tools": tools_count,
    }))
}

async fn chat_completions(
    State(s): State<Arc<AppState>>,
    Json(body): Json<ChatRequest>,
) -> impl IntoResponse {
    let last_user = body
        .messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or("")
        .to_string();

    // ── Package-as-model dispatch ────────────────────────────────────────────
    // A client `model` matching a loaded package name selects that package;
    // the real backend model comes from --model. Any other value is forwarded
    // to the backend unchanged (classic OpenAI passthrough).
    let selected_pkg: Option<usize> = if body.model.is_empty() {
        None
    } else {
        s.pkg_names.iter().position(|n| n == &body.model)
    };
    let used_model = if body.model.is_empty() || selected_pkg.is_some() {
        s.model.clone()
    } else {
        body.model.clone()
    };
    // What the client should see as `model` in the response: the package name
    // when one was explicitly selected (so a chat UI's model picker shows
    // "k8s-expert", not the backend LLM tag), otherwise the real backend model.
    let echo_model = if selected_pkg.is_some() {
        body.model.clone()
    } else {
        used_model.clone()
    };
    let temperature = if body.temperature == 0.0 {
        1.0
    } else {
        body.temperature
    };
    let max_tokens = body.max_tokens.unwrap_or(s.max_tokens);

    // Compute query embedding once — reused for routing, RAG, strict-verify, and strict-render.
    let query_vec: Option<Vec<f32>> = if !s.embed_model.is_empty() {
        embed_query(&s.embed_url, &s.embed_model, &last_user, &s.api_key)
            .await
            .ok()
    } else {
        None
    };

    // ── Runtime selection: explicit package > graph routing > merged ─────────
    let (composed_rt, claims_rt, idty_rt, ansp_rt, nknw_rt, plcy_rt): (
        &ComposedRuntime,
        &ClaimsRuntime,
        &IdtyRuntime,
        &AnspRuntime,
        &NknwRuntime,
        &PlcyRuntime,
    ) = if let Some(idx) = selected_pkg {
        let pkg = &s.per_pkg[idx];
        (
            &pkg.composed,
            &pkg.claims,
            &pkg.idty,
            &pkg.ansp,
            &pkg.nknw,
            &pkg.plcy,
        )
    } else if s.graph_mode && s.graph_thkg_vecs.len() >= 2 {
        if let Some(ref qv) = query_vec {
            match best_route(&s.graph_thkg_vecs, qv, 0.1) {
                Some(idx) => {
                    let pkg = &s.per_pkg[idx];
                    eprintln!("Graph → '{}'", pkg.name);
                    (
                        &pkg.composed,
                        &pkg.claims,
                        &pkg.idty,
                        &pkg.ansp,
                        &pkg.nknw,
                        &pkg.plcy,
                    )
                }
                None => (&s.composed, &s.claims, &s.idty, &s.ansp, &s.nknw, &s.plcy),
            }
        } else {
            (&s.composed, &s.claims, &s.idty, &s.ansp, &s.nknw, &s.plcy)
        }
    } else {
        (&s.composed, &s.claims, &s.idty, &s.ansp, &s.nknw, &s.plcy)
    };

    // ── NKNW: forbidden topics → immediate refusal (no LLM call) ────────────
    if let Some(refusal) = nknw_rt.forbidden_refusal(&last_user) {
        let resp = json!({
            "id": "chatcmpl-nknw",
            "object": "chat.completion",
            "model": echo_model,
            "choices": [{"index": 0, "message": {"role": "assistant", "content": refusal}, "finish_reason": "stop"}],
            "_aipk": {"mode": "nknw-refusal"}
        });
        return if body.stream {
            json_to_sse_response(resp)
        } else {
            Json(resp).into_response()
        };
    }

    // ── ANSP: domain gate → refuse out-of-scope queries (no LLM call) ───────
    if !ansp_rt.is_empty() {
        match ansp_rt.check_domain(&last_user) {
            AnspDecision::Refuse(msg, _code) => {
                let resp = json!({
                    "id": "chatcmpl-ansp",
                    "object": "chat.completion",
                    "model": echo_model,
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": msg}, "finish_reason": "stop"}],
                    "_aipk": {"mode": "ansp-out-of-scope"}
                });
                return if body.stream {
                    json_to_sse_response(resp)
                } else {
                    Json(resp).into_response()
                };
            }
            AnspDecision::Escalate(msg) => {
                let resp = json!({
                    "id": "chatcmpl-ansp",
                    "object": "chat.completion",
                    "model": echo_model,
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": msg}, "finish_reason": "stop"}],
                    "_aipk": {"mode": "ansp-escalate"}
                });
                return if body.stream {
                    json_to_sse_response(resp)
                } else {
                    Json(resp).into_response()
                };
            }
            _ => {}
        }
    }

    // Build persona with optional language hint, IDTY prefix, and PLCY hints
    let base_persona = build_persona(
        composed_rt.persona(),
        s.lang.as_deref(),
        idty_rt,
        plcy_rt,
        &nknw_rt.unknown_notes(&last_user),
    );

    // ── Strict-verify: LLM answers only from canonical claims
    if s.strict_verify && !claims_rt.is_empty() {
        let (_, matched) = claims_rt.check_answerability_best(&last_user, query_vec.as_deref());
        let constraints = claims_rt.format_verify_constraints(&matched);
        let persona = format!("{base_persona}\n\n{constraints}");
        let assembled = assemble_messages(&persona, composed_rt.skills(), &body.messages, &[]);
        let msgs = match s.lang.as_deref() {
            Some(l) if !l.is_empty() => inject_lang(assembled.messages, l),
            _ => assembled.messages,
        };
        let mut result = llm_tool_loop_json(&s, msgs, used_model, temperature, max_tokens).await;
        result["model"] = json!(echo_model);
        return if body.stream {
            json_to_sse_response(result)
        } else {
            Json(result).into_response()
        };
    }

    // ── Strict-render: LLM must cite [claim_id] after every factual sentence
    if s.strict_render && !claims_rt.is_empty() {
        let (_, matched) = claims_rt.check_answerability_best(&last_user, query_vec.as_deref());
        let render_prompt = claims_rt.format_render_prompt(&matched);
        let persona = format!("{base_persona}\n\n{render_prompt}");
        let assembled = assemble_messages(&persona, composed_rt.skills(), &body.messages, &[]);
        let msgs = match s.lang.as_deref() {
            Some(l) if !l.is_empty() => inject_lang(assembled.messages, l),
            _ => assembled.messages,
        };
        let mut result =
            llm_render_loop_json(&s, msgs, used_model, temperature, max_tokens, &matched).await;
        result["model"] = json!(echo_model);
        return if body.stream {
            json_to_sse_response(result)
        } else {
            Json(result).into_response()
        };
    }

    // ── Normal mode: RAG retrieval ───────────────────────────────────────────
    let (rag_chunks, best_rag_score) = if composed_rt.has_know() {
        if let Some(ref qv) = query_vec {
            let scored = composed_rt.retrieve_scored(qv, 5);
            let best = scored.first().map(|(sc, _)| *sc).unwrap_or(0.0);
            let chunks: Vec<String> = scored.into_iter().map(|(_, text)| text).collect();
            (chunks, best)
        } else {
            (vec![], 0.0)
        }
    } else {
        (vec![], 0.0)
    };

    // ── ANSP: coverage gate after retrieval ──────────────────────────────────
    if !ansp_rt.is_empty() && composed_rt.has_know() {
        match ansp_rt.check_coverage(best_rag_score) {
            AnspDecision::Refuse(msg, _code) => {
                let resp = json!({
                    "id": "chatcmpl-ansp-cov",
                    "object": "chat.completion",
                    "model": echo_model,
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": msg}, "finish_reason": "stop"}],
                    "_aipk": {"mode": "ansp-low-coverage"}
                });
                return if body.stream {
                    json_to_sse_response(resp)
                } else {
                    Json(resp).into_response()
                };
            }
            AnspDecision::AllowPartial(note) => {
                eprintln!("ANSP partial: {note}");
            }
            _ => {}
        }
    }

    let assembled = assemble_messages(
        &base_persona,
        composed_rt.skills(),
        &body.messages,
        &rag_chunks,
    );
    let assembled_msgs = match s.lang.as_deref() {
        Some(l) if !l.is_empty() => inject_lang(assembled.messages, l),
        _ => assembled.messages,
    };

    // ── Streaming path ───────────────────────────────────────────────────────
    if body.stream {
        let has_tools = !s.mcp.lock().await.to_openai_tools().is_empty();

        if has_tools {
            let mut result =
                llm_tool_loop_json(&s, assembled_msgs, used_model, temperature, max_tokens).await;
            result["model"] = json!(echo_model);
            return json_to_sse_response(result);
        }

        return stream_direct(
            &s,
            assembled_msgs,
            used_model,
            echo_model,
            temperature,
            max_tokens,
        )
        .await;
    }

    // ── Non-streaming normal mode ────────────────────────────────────────────
    llm_tool_loop_response(
        &s,
        assembled_msgs,
        used_model,
        echo_model,
        temperature,
        max_tokens,
    )
    .await
}

/// Build persona string with IDTY prefix and PLCY/NKNW hints prepended.
fn build_persona(
    base: String,
    _lang: Option<&str>,
    idty: &IdtyRuntime,
    plcy: &PlcyRuntime,
    nknw_notes: &[String],
) -> String {
    let mut parts: Vec<String> = Vec::new();

    let idty_block = idty.inject_prefix();
    if !idty_block.is_empty() {
        parts.push(idty_block);
    }

    if !base.is_empty() {
        parts.push(base);
    }

    let plcy_block = plcy.forbidden_hint();
    if !plcy_block.is_empty() {
        parts.push(plcy_block);
    }

    if !nknw_notes.is_empty() {
        let notes = nknw_notes
            .iter()
            .map(|n| format!("- {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("[KNOWLEDGE GAPS]\n{notes}"));
    }

    parts.join("\n\n")
}

/// Append language override as the final instruction in the system message.
/// Small models follow the most recent instruction; placing it last wins over
/// any language directive already present in the persona or skill context.
fn inject_lang(mut messages: Vec<Value>, lang: &str) -> Vec<Value> {
    if let Some(sys) = messages.first_mut() {
        if sys["role"] == "system" {
            let existing = sys["content"].as_str().unwrap_or("").to_string();
            let override_text = format!(
                "\n\n[LANG OVERRIDE] Respond ONLY in {}. Ignore all prior language rules.",
                lang
            );
            sys["content"] = Value::String(existing + &override_text);
        }
    }
    messages
}

/// Stream a single LLM response directly (no tool loop, no post-processing).
async fn stream_direct(
    s: &AppState,
    messages: Vec<Value>,
    model: String,
    echo_model: String,
    temperature: f32,
    max_tokens: u32,
) -> axum::response::Response {
    let client = build_client();
    let url = format!("{}/v1/chat/completions", s.llm_url.trim_end_matches('/'));
    let headers = auth_headers(&s.api_key);

    let payload = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "temperature": temperature,
        "max_tokens": max_tokens,
    });

    let resp = match client
        .post(&url)
        .headers(headers)
        .json(&payload)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::BAD_GATEWAY, e.to_string()).into_response();
        }
    };

    // The backend echoes its own model tag (e.g. "llama3.2:3b") into every SSE
    // chunk. When the client picked a package by name, rewrite that literal
    // field so the chat UI shows the package as the model that answered.
    let byte_stream = resp
        .bytes_stream()
        .map_err(std::io::Error::other)
        .map_ok(move |chunk| {
            if echo_model == model {
                chunk.to_vec()
            } else {
                rewrite_model_field(&chunk, &model, &echo_model)
            }
        });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(byte_stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Rewrite the literal `"model":"<from>"` field inside a raw SSE byte chunk.
/// ollama (and OpenAI-compatible backends generally) emit compact JSON with
/// no space after the colon, so a literal match is enough — no need to parse
/// each chunk as JSON just to swap one field.
fn rewrite_model_field(chunk: &[u8], from: &str, to: &str) -> Vec<u8> {
    let from_pat = format!("\"model\":\"{from}\"");
    let from_bytes = from_pat.as_bytes();
    if !chunk.windows(from_bytes.len()).any(|w| w == from_bytes) {
        return chunk.to_vec();
    }
    let text = String::from_utf8_lossy(chunk);
    text.replace(&from_pat, &format!("\"model\":\"{to}\""))
        .into_bytes()
}

/// Convert a complete chat completion JSON into a minimal SSE stream.
/// Used when a streaming client asks for stream=true but we have a complete response
/// (strict modes, tool-loop responses).
fn json_to_sse_response(response: Value) -> axum::response::Response {
    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let model = response["model"].as_str().unwrap_or("unknown").to_string();
    let id = response["id"].as_str().unwrap_or("chatcmpl-0").to_string();

    let chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "delta": {"role": "assistant", "content": content},
            "index": 0,
            "finish_reason": "stop"
        }]
    });

    let aipk_meta = &response["_aipk"];
    let body = if !aipk_meta.is_null() {
        let meta_chunk = json!({
            "id": id,
            "object": "chat.completion.chunk",
            "model": model,
            "_aipk": aipk_meta,
            "choices": [{"delta": {}, "index": 0, "finish_reason": null}]
        });
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            serde_json::to_string(&chunk).unwrap_or_default(),
            serde_json::to_string(&meta_chunk).unwrap_or_default(),
        )
    } else {
        format!(
            "data: {}\n\ndata: [DONE]\n\n",
            serde_json::to_string(&chunk).unwrap_or_default(),
        )
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Run tool loop and return the final JSON value (for non-streaming use or SSE conversion).
async fn llm_tool_loop_json(
    s: &AppState,
    messages: Vec<Value>,
    model: String,
    temperature: f32,
    max_tokens: u32,
) -> Value {
    let client = build_client();
    let url = format!("{}/v1/chat/completions", s.llm_url.trim_end_matches('/'));
    let headers = auth_headers(&s.api_key);
    let mut mcp = s.mcp.lock().await;

    match run_tool_loop(
        &client,
        &url,
        headers,
        &mut mcp,
        messages,
        &model,
        temperature,
        max_tokens,
        false,
    )
    .await
    {
        Ok((_, v)) => v,
        Err(e) => json!({"error": e.to_string()}),
    }
}

/// Run tool loop and return an HTTP response.
async fn llm_tool_loop_response(
    s: &AppState,
    messages: Vec<Value>,
    model: String,
    echo_model: String,
    temperature: f32,
    max_tokens: u32,
) -> axum::response::Response {
    let client = build_client();
    let url = format!("{}/v1/chat/completions", s.llm_url.trim_end_matches('/'));
    let headers = auth_headers(&s.api_key);
    let mut mcp = s.mcp.lock().await;

    match run_tool_loop(
        &client,
        &url,
        headers,
        &mut mcp,
        messages,
        &model,
        temperature,
        max_tokens,
        false,
    )
    .await
    {
        Ok((status, mut result)) => {
            result["model"] = json!(echo_model);
            (
                axum::http::StatusCode::from_u16(status).unwrap_or_default(),
                Json(result),
            )
                .into_response()
        }
        Err(e) => Json(json!({"error": e.to_string()})).into_response(),
    }
}

/// Strict-render: call LLM once, then verify every cited [claim_id] is canonical.
async fn llm_render_loop_json(
    s: &AppState,
    messages: Vec<Value>,
    model: String,
    temperature: f32,
    max_tokens: u32,
    _matched_claims: &[&crate::runtime::Claim],
) -> Value {
    let client = build_client();
    let url = format!("{}/v1/chat/completions", s.llm_url.trim_end_matches('/'));
    let headers = auth_headers(&s.api_key);
    let mut mcp = s.mcp.lock().await;

    let result: Value = match run_tool_loop(
        &client,
        &url,
        headers,
        &mut mcp,
        messages,
        &model,
        temperature,
        max_tokens,
        false,
    )
    .await
    {
        Ok((_, v)) => v,
        Err(e) => return json!({"error": e.to_string()}),
    };
    drop(mcp);

    let response_text = result["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let cited_ids = crate::runtime::ClaimsRuntime::extract_cited_ids(&response_text);

    let mut canonical_claims_used: Vec<String> = cited_ids
        .iter()
        .filter(|id| s.claims.get_canonical(id).is_some())
        .cloned()
        .collect();
    canonical_claims_used.sort();
    canonical_claims_used.dedup();

    let invalid_claim_ids: Vec<String> = cited_ids
        .iter()
        .filter(|id| s.claims.get_canonical(id).is_none())
        .cloned()
        .collect();

    let unsupported_sentences = collect_uncited_sentences(&response_text);
    let total_sentences = count_sentences(&response_text).max(1);
    let supported = total_sentences.saturating_sub(unsupported_sentences.len());
    let coverage = supported as f32 / total_sentences as f32;

    let fully_grounded = invalid_claim_ids.is_empty() && unsupported_sentences.is_empty();

    let mut out = result.clone();
    out["_aipk"] = json!({
        "mode": "strict-render",
        "coverage": (coverage * 100.0).round() / 100.0,
        "canonical_claims_used": canonical_claims_used.len(),
        "canonical_claim_ids": canonical_claims_used,
        "uncited_sentences": unsupported_sentences.len(),
        "invalid_claim_ids": invalid_claim_ids,
        "unsupported_sentences": unsupported_sentences,
        "fully_grounded": fully_grounded,
    });
    apply_enforce(s.enforce, fully_grounded, out)
}

/// Applies `--enforce` to an already-scored strict-render response. Pure and
/// LLM-free by design — the interesting behavior (does `block` actually
/// withhold, does `warn` actually flag, does `observe` leave things alone)
/// is exactly what's under test, and none of that needs a real model call.
///
/// EXPERIMENTAL, deliberately narrow: `verdict` reflects what this gate did
/// (allow/warn/block), not a full grounding contract — it gates on the same
/// single coverage/fully_grounded signal `fully_grounded` already exposed,
/// it does not check claim freshness, semantic entailment, or contradictions.
/// Don't read `verdict: "block"` as "no hallucination is possible here."
pub(crate) fn apply_enforce(
    mode: crate::runtime::EnforceMode,
    fully_grounded: bool,
    mut out: Value,
) -> Value {
    use crate::runtime::EnforceMode;

    let verdict = match mode {
        EnforceMode::Observe => "allow",
        EnforceMode::Warn if !fully_grounded => "warn",
        EnforceMode::Block if !fully_grounded => "block",
        _ => "allow",
    };
    out["_aipk"]["verdict"] = json!(verdict);
    out["_aipk"]["enforce"] = json!(match mode {
        EnforceMode::Observe => "observe",
        EnforceMode::Warn => "warn",
        EnforceMode::Block => "block",
    });

    match verdict {
        "block" => {
            out["choices"][0]["message"]["content"] = json!(
                "This answer could not be fully grounded in canonical claims and was withheld \
                 (--enforce block). See the _aipk field for coverage and uncited-sentence detail."
            );
            out["_aipk"]["mode"] = json!("strict-render-blocked");
        }
        "warn" => {
            out["_aipk"]["flagged"] = json!(true);
        }
        _ => {}
    }
    out
}

pub(crate) fn collect_uncited_sentences(text: &str) -> Vec<String> {
    sentence_units(text)
        .into_iter()
        .filter(|s| !s.contains('['))
        .collect()
}

fn sentence_units(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            if crate::parsers::is_decimal_point(&current, chars.peek()) {
                continue;
            }
            while matches!(chars.peek(), Some(next) if next.is_whitespace()) {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            if matches!(chars.peek(), Some('[')) {
                for next in chars.by_ref() {
                    current.push(next);
                    if next == ']' {
                        break;
                    }
                }
            }

            let s = current.trim().to_string();
            current.clear();
            if s.len() < 10 {
                continue;
            }
            sentences.push(s);
        }
    }
    let tail = current.trim().to_string();
    if tail.len() >= 10 {
        sentences.push(tail);
    }
    sentences
}

pub(crate) fn count_sentences(text: &str) -> usize {
    sentence_units(text).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_units_attach_trailing_claim_citations() {
        let units = sentence_units("The package header is 96 bytes long. [aipk_e2e_doc_0003]");
        assert_eq!(
            units,
            vec!["The package header is 96 bytes long. [aipk_e2e_doc_0003]"]
        );
        assert!(collect_uncited_sentences(
            "The package header is 96 bytes long. [aipk_e2e_doc_0003]"
        )
        .is_empty());
    }

    #[test]
    fn collect_uncited_sentences_keeps_truly_uncited_sentences() {
        let uncited = collect_uncited_sentences(
            "The package header is 96 bytes long. The CLMS section stores claims. [c1]",
        );
        assert_eq!(uncited, vec!["The package header is 96 bytes long."]);
    }

    #[test]
    fn build_persona_returns_base_unchanged() {
        let idty = IdtyRuntime::from_packages(&[]);
        let plcy = PlcyRuntime::from_packages(&[]);
        let p = build_persona("Be helpful.".to_string(), Some("ru"), &idty, &plcy, &[]);
        assert_eq!(p, "Be helpful.");
    }

    #[test]
    fn inject_lang_appends_to_system_message() {
        let msgs = vec![
            json!({"role": "system", "content": "Be helpful."}),
            json!({"role": "user", "content": "Hello"}),
        ];
        let result = inject_lang(msgs, "ru");
        let sys = result[0]["content"].as_str().unwrap();
        assert!(sys.contains("LANG OVERRIDE"));
        assert!(sys.contains("ru"));
        assert!(sys.starts_with("Be helpful."));
    }

    #[test]
    fn build_persona_no_lang_unchanged() {
        let idty = IdtyRuntime::from_packages(&[]);
        let plcy = PlcyRuntime::from_packages(&[]);
        let p = build_persona("Be helpful.".to_string(), None, &idty, &plcy, &[]);
        assert_eq!(p, "Be helpful.");
    }

    fn sample_out() -> Value {
        json!({
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Original answer."}}],
            "_aipk": {"fully_grounded": false}
        })
    }

    #[test]
    fn apply_enforce_observe_never_touches_content() {
        let out = apply_enforce(crate::runtime::EnforceMode::Observe, false, sample_out());
        assert_eq!(out["choices"][0]["message"]["content"], "Original answer.");
        assert_eq!(out["_aipk"]["verdict"], "allow");
    }

    #[test]
    fn apply_enforce_warn_keeps_content_but_flags() {
        let out = apply_enforce(crate::runtime::EnforceMode::Warn, false, sample_out());
        assert_eq!(out["choices"][0]["message"]["content"], "Original answer.");
        assert_eq!(out["_aipk"]["verdict"], "warn");
        assert_eq!(out["_aipk"]["flagged"], true);
    }

    #[test]
    fn apply_enforce_block_withholds_ungrounded_content() {
        let out = apply_enforce(crate::runtime::EnforceMode::Block, false, sample_out());
        assert_ne!(out["choices"][0]["message"]["content"], "Original answer.");
        assert_eq!(out["_aipk"]["verdict"], "block");
        assert_eq!(out["_aipk"]["mode"], "strict-render-blocked");
    }

    #[test]
    fn apply_enforce_block_leaves_grounded_answers_alone() {
        let out = apply_enforce(crate::runtime::EnforceMode::Block, true, sample_out());
        assert_eq!(out["choices"][0]["message"]["content"], "Original answer.");
        assert_eq!(out["_aipk"]["verdict"], "allow");
    }
}
