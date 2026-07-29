use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;

use crate::mcp_client::McpManager;

/// A generation that never hits a stop token (small local models do this
/// occasionally) would otherwise block the caller forever, since the
/// backend request has no server-side length limit unless we send one.
/// This is the belt to `max_tokens`'s suspenders: even if a caller forgets
/// to bound the response, the HTTP call itself gives up eventually.
const REQUEST_TIMEOUT_SECS: u64 = 300;

/// Fallback response length cap for callers that don't expose their own
/// `--max-tokens` flag (MCP tool answers, base-model tests, init smoke tests).
pub const DEFAULT_MAX_TOKENS: u32 = 1024;

/// Build an HTTP client for LLM backend calls with a bounded request timeout.
pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .unwrap_or_default()
}

/// Run a single-shot or tool-augmented LLM conversation loop.
///
/// Returns `(http_status, completion_json)`.  On transport error returns Err.
/// Callers format the result as needed (HTTP response, stdout, string).
#[allow(clippy::too_many_arguments)]
pub async fn run_tool_loop(
    client: &reqwest::Client,
    url: &str,
    headers: reqwest::header::HeaderMap,
    mcp: &mut McpManager,
    mut messages: Vec<Value>,
    model: &str,
    temperature: f32,
    max_tokens: u32,
    verbose: bool,
) -> Result<(u16, Value)> {
    let tools = mcp.to_openai_tools();
    let has_tools = !tools.is_empty();

    for _ in 0..10usize {
        let mut payload = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });
        if has_tools {
            payload["tools"] = json!(tools);
            payload["tool_choice"] = json!("auto");
        }

        let resp = client
            .post(url)
            .headers(headers.clone())
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow!("LLM request error: {e}"))?;

        let status = resp.status().as_u16();

        let result: Value = resp
            .json()
            .await
            .map_err(|e| anyhow!("LLM response parse error: {e}"))?;

        if !(200..300).contains(&status) {
            let detail = result["error"]["message"]
                .as_str()
                .or_else(|| result["error"].as_str())
                .unwrap_or("unknown error");
            return Err(anyhow!("LLM error {status}: {detail}"));
        }

        let choice = &result["choices"][0];
        let finish = choice["finish_reason"].as_str().unwrap_or("stop");
        let msg = &choice["message"];

        if finish != "tool_calls" || msg["tool_calls"].is_null() {
            return Ok((status, result));
        }

        messages.push(msg.clone());

        if let Some(tool_calls) = msg["tool_calls"].as_array() {
            for tc in tool_calls {
                let fn_name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let args: Value = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|a| serde_json::from_str(a).ok())
                    .unwrap_or(json!({}));
                let call_id = tc["id"].as_str().unwrap_or("").to_string();

                if verbose {
                    eprintln!("→ tool: {fn_name}({args})");
                }

                let tool_result = mcp.call_tool(&fn_name, &args);

                if verbose {
                    eprintln!("← {}", &tool_result[..tool_result.len().min(100)]);
                }

                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": tool_result,
                }));
            }
        }
    }

    Ok((200, json!({"error": "max tool iterations reached"})))
}

/// Build Authorization header map for a given API key (empty key = no header).
pub fn auth_headers(api_key: &str) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if !api_key.is_empty() {
        if let Ok(v) = format!("Bearer {api_key}").parse() {
            headers.insert(reqwest::header::AUTHORIZATION, v);
        }
    }
    headers
}
