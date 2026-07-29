use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[allow(dead_code)]
pub struct LlmClient {
    client: reqwest::Client,
    base_url: String,
    model: String,
    headers: reqwest::header::HeaderMap,
}

impl LlmClient {
    #[allow(dead_code)]
    pub fn new(base_url: &str, model: &str, api_key: &str) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if !api_key.is_empty() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {api_key}").parse().unwrap(),
            );
        }
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            headers,
        }
    }

    #[allow(dead_code)]
    pub async fn complete(&self, messages: Vec<Value>) -> Result<String> {
        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .headers(self.headers.clone())
            .json(&json!({
                "model": self.model,
                "messages": messages,
                "stream": false,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("LLM error {status}: {body}");
        }

        let result: Value = resp.json().await?;
        Ok(result["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }
}

/// Which embedding API a base URL speaks. Detected once per URL, then cached.
#[derive(Clone, Copy, PartialEq, Debug)]
enum EmbedApi {
    Ollama, // POST /api/embed        → {"embeddings": [[..]]}
    OpenAi, // POST /v1/embeddings    → {"data": [{"embedding": [..]}]}
}

fn embed_api_cache() -> &'static Mutex<HashMap<String, EmbedApi>> {
    static CACHE: OnceLock<Mutex<HashMap<String, EmbedApi>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse_ollama_embedding(result: &Value) -> Result<Vec<f32>> {
    let vec: Vec<f32> = result["embeddings"][0]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no embeddings in ollama response"))?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
        .collect();
    Ok(vec)
}

fn parse_openai_embedding(result: &Value) -> Result<Vec<f32>> {
    let vec: Vec<f32> = result["data"][0]["embedding"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no embedding in OpenAI-format response"))?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
        .collect();
    Ok(vec)
}

async fn embed_via(
    api: EmbedApi,
    base_url: &str,
    model: &str,
    text: &str,
    api_key: &str,
) -> Result<Vec<f32>> {
    let path = match api {
        EmbedApi::Ollama => "/api/embed",
        EmbedApi::OpenAi => "/v1/embeddings",
    };
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{}{}", base_url.trim_end_matches('/'), path))
        .json(&json!({"model": model, "input": text}));
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("embed error {status}: {body}");
    }
    let result: Value = resp.json().await?;
    match api {
        EmbedApi::Ollama => parse_ollama_embedding(&result),
        EmbedApi::OpenAi => parse_openai_embedding(&result),
    }
}

/// Embed `text` against `base_url`. Speaks both the ollama API (`/api/embed`)
/// and the OpenAI-compatible API (`/v1/embeddings` — vLLM, SGLang, llama.cpp,
/// llamafile). The working API for each base URL is detected on first call
/// and cached for the rest of the process.
pub async fn embed_query(
    base_url: &str,
    model: &str,
    text: &str,
    api_key: &str,
) -> Result<Vec<f32>> {
    let cached = embed_api_cache()
        .lock()
        .ok()
        .and_then(|c| c.get(base_url).copied());

    if let Some(api) = cached {
        return embed_via(api, base_url, model, text, api_key).await;
    }

    // Detect: try ollama first (its error messages are clearer for missing
    // models), then fall back to the OpenAI-compatible endpoint.
    match embed_via(EmbedApi::Ollama, base_url, model, text, api_key).await {
        Ok(vec) => {
            if let Ok(mut c) = embed_api_cache().lock() {
                c.insert(base_url.to_string(), EmbedApi::Ollama);
            }
            Ok(vec)
        }
        Err(ollama_err) => match embed_via(EmbedApi::OpenAi, base_url, model, text, api_key).await
        {
            Ok(vec) => {
                if let Ok(mut c) = embed_api_cache().lock() {
                    c.insert(base_url.to_string(), EmbedApi::OpenAi);
                }
                Ok(vec)
            }
            Err(openai_err) => bail!(
                "embedding failed on both APIs\n  ollama /api/embed: {ollama_err}\n  openai /v1/embeddings: {openai_err}"
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ollama_embed_format() {
        let v = json!({"embeddings": [[0.1, 0.2, 0.3]]});
        assert_eq!(parse_ollama_embedding(&v).unwrap(), vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn parses_openai_embed_format() {
        let v = json!({"data": [{"embedding": [1.0, -0.5]}], "model": "bge"});
        assert_eq!(parse_openai_embedding(&v).unwrap(), vec![1.0, -0.5]);
    }

    #[test]
    fn wrong_format_is_an_error() {
        let openai = json!({"data": [{"embedding": [1.0]}]});
        assert!(parse_ollama_embedding(&openai).is_err());
        let ollama = json!({"embeddings": [[1.0]]});
        assert!(parse_openai_embedding(&ollama).is_err());
    }
}
