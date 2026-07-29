use crate::agent::{auth_headers, build_client, run_tool_loop, DEFAULT_MAX_TOKENS};
use crate::crypto::load_package;
use crate::llm::embed_query;
use crate::mcp_client::McpManager;
use crate::runtime::{assemble_messages, KnowRuntime};
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::Path;

const ASK_TOOL: &str = r#"{
  "name": "ask",
  "description": "Ask this agent a question and get an expert answer.",
  "inputSchema": {
    "type": "object",
    "properties": {"question": {"type": "string", "description": "The question to ask"}},
    "required": ["question"]
  }
}"#;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    pkg_path: &Path,
    llm_url: &str,
    model: &str,
    api_key: &str,
    embed_model: &str,
    trust_tools: bool,
    allow: &[String],
    allow_tool: &[String],
) -> Result<()> {
    let pkg = load_package(pkg_path, None)?;
    let pkg_name = pkg.name.clone();
    let know = KnowRuntime::load(&pkg);
    let mut mcp = McpManager::new()
        .load_from_pkg(&pkg)
        .with_allowed_tools(allow_tool);
    if !mcp.confirm_launch(trust_tools, allow) {
        anyhow::bail!("MCP server launch declined");
    }
    mcp.start_all();

    let ask_tool: Value = serde_json::from_str(ASK_TOOL)?;
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                let out = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}});
                writeln!(stdout, "{}", serde_json::to_string(&out)?)?;
                continue;
            }
        };

        let id = msg["id"].clone();
        let method = msg["method"].as_str().unwrap_or("");

        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": pkg_name, "version": "0.1"}
                }
            }),
            "notifications/initialized" => continue,
            "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"tools": [ask_tool]}
            }),
            "tools/call" => {
                let tool_name = msg["params"]["name"].as_str().unwrap_or("");
                if tool_name != "ask" {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unknown tool"}})
                } else {
                    let question = msg["params"]["arguments"]["question"]
                        .as_str()
                        .unwrap_or("");
                    let answer = do_ask(
                        &pkg,
                        &mut mcp,
                        &know,
                        question,
                        llm_url,
                        model,
                        api_key,
                        embed_model,
                    )
                    .await;
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"content": [{"type": "text", "text": answer}]}
                    })
                }
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("Method not found: {method}")}
            }),
        };

        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_ask(
    pkg: &crate::format::AipkPackage,
    mcp: &mut McpManager,
    know: &KnowRuntime,
    question: &str,
    llm_url: &str,
    model: &str,
    api_key: &str,
    embed_model: &str,
) -> String {
    let rag_chunks = if !know.is_empty() && !embed_model.is_empty() {
        match embed_query(llm_url, embed_model, question, api_key).await {
            Ok(qvec) => know.retrieve(&qvec, 5),
            Err(_) => vec![],
        }
    } else {
        vec![]
    };

    let persona = pkg.persona().unwrap_or_default();
    let skills = pkg.skills();
    let user_msg = json!({"role": "user", "content": question});
    let assembled = assemble_messages(&persona, &skills, &[user_msg], &rag_chunks);

    let client = build_client();
    let url = format!("{}/v1/chat/completions", llm_url.trim_end_matches('/'));
    let headers = auth_headers(api_key);

    match run_tool_loop(
        &client,
        &url,
        headers,
        mcp,
        assembled.messages,
        model,
        1.0,
        DEFAULT_MAX_TOKENS,
        false,
    )
    .await
    {
        Ok((_, result)) => result["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        Err(e) => format!("[LLM error: {e}]"),
    }
}
