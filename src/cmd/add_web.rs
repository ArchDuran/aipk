use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Duration;

use crate::cmd::add_docs::chunk_text;
use crate::llm::embed_query;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    project_dir: &Path,
    source: &str,
    embed_model: &str,
    llm_url: &str,
    api_key: &str,
    chunk_size: usize,
    is_sitemap: bool,
    max_pages: usize,
    delay_ms: u64,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent("aipk/1.0 (knowledge import bot)")
        .timeout(Duration::from_secs(30))
        .build()?;

    // ── Collect URLs to fetch ─────────────────────────────────────────────────
    let urls: Vec<String> = if is_sitemap {
        fetch_sitemap(&client, source)
            .await
            .with_context(|| format!("fetching sitemap {source}"))?
            .into_iter()
            .take(max_pages)
            .collect()
    } else {
        vec![source.to_string()]
    };

    if urls.is_empty() {
        println!("No URLs found in sitemap.");
        return Ok(());
    }
    println!("Importing {} page(s) from {}…", urls.len(), source);

    // ── Load existing embedding state ─────────────────────────────────────────
    let cache_dir = project_dir.join(".aipk");
    fs::create_dir_all(&cache_dir)?;

    let state_file = cache_dir.join("state.json");
    let chunks_file = cache_dir.join("chunks.jsonl");
    let vectors_file = cache_dir.join("vectors.bin");

    let mut state: serde_json::Map<String, Value> = if state_file.exists() {
        serde_json::from_str(&fs::read_to_string(&state_file)?)?
    } else {
        serde_json::Map::new()
    };

    if let Some(existing) = state.get("embedding_model").and_then(|v| v.as_str()) {
        if existing != embed_model {
            anyhow::bail!(
                "Embedding model mismatch: project uses '{}', requested '{}'. \
                 Use a consistent model across add-docs calls.",
                existing,
                embed_model
            );
        }
    }

    let mut chunk_id = state
        .get("chunk_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let mut dim: Option<u32> = state
        .get("embedding_dim")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let chunks_f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&chunks_file)?;
    let mut chunks_w = BufWriter::new(chunks_f);

    let vectors_f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&vectors_file)?;
    let mut vectors_w = BufWriter::new(vectors_f);

    // ── Fetch and embed ───────────────────────────────────────────────────────
    let mut added = 0usize;
    let mut failed = 0usize;

    for (i, url) in urls.iter().enumerate() {
        if i > 0 && delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }

        let text = match fetch_text(&client, url).await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  skip {url}: {e}");
                failed += 1;
                continue;
            }
        };

        if text.trim().is_empty() {
            eprintln!("  skip {url}: empty after stripping");
            failed += 1;
            continue;
        }

        let text_chunks = chunk_text(&text, chunk_size);
        if text_chunks.is_empty() {
            failed += 1;
            continue;
        }

        // Source label: use URL path portion for readability
        let source_label = url_to_label(url);
        eprintln!("  {} → {} chunks", source_label, text_chunks.len());

        for chunk in text_chunks {
            let vec = embed_query(llm_url, embed_model, &chunk, api_key)
                .await
                .with_context(|| format!("embedding chunk from {url}"))?;

            if dim.is_none() {
                dim = Some(vec.len() as u32);
            }

            let line = serde_json::to_string(&json!({
                "id": chunk_id,
                "text": chunk,
                "source": source_label,
                "meta": {"url": url, "web_source": source},
            }))?;
            writeln!(chunks_w, "{}", line)?;

            for &v in &vec {
                vectors_w.write_all(&v.to_le_bytes())?;
            }

            chunk_id += 1;
            added += 1;
        }
    }

    chunks_w.flush()?;
    vectors_w.flush()?;

    state.insert("embedding_model".into(), json!(embed_model));
    state.insert("embedding_dim".into(), json!(dim.unwrap_or(0)));
    state.insert("chunk_count".into(), json!(chunk_id));
    fs::write(
        &state_file,
        serde_json::to_string_pretty(&Value::Object(state))?,
    )?;

    println!(
        "✓ web import: {} chunks added, {}/{} page(s) failed (total: {}, dim={})",
        added,
        failed,
        urls.len(),
        chunk_id,
        dim.unwrap_or(0)
    );
    Ok(())
}

/// Fetch a URL and return its text content (HTML stripped).
pub async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().await?;

    if content_type.contains("text/html") || url.ends_with(".html") || url.ends_with(".htm") {
        Ok(strip_html(&body))
    } else {
        Ok(body)
    }
}

/// Fetch a sitemap (XML) and return all `<loc>` URLs.
pub async fn fetch_sitemap(client: &reqwest::Client, url: &str) -> Result<Vec<String>> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let xml = resp.text().await?;
    Ok(extract_loc_tags(&xml))
}

/// Extract `<loc>…</loc>` values from a sitemap XML string.
fn extract_loc_tags(xml: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<loc>") {
        let after_tag = &rest[start + 5..];
        if let Some(end) = after_tag.find("</loc>") {
            let url = after_tag[..end].trim().to_string();
            if !url.is_empty() {
                urls.push(url);
            }
            rest = &after_tag[end + 6..];
        } else {
            break;
        }
    }
    urls
}

/// Strip HTML tags and decode common entities, returning plain text.
pub fn strip_html(html: &str) -> String {
    // Remove <script>…</script> and <style>…</style> blocks entirely.
    let s = remove_block_tags(html, "script");
    let s = remove_block_tags(&s, "style");
    let s = remove_block_tags(&s, "head");

    // Replace block-level closing tags with newlines for readability.
    let block_closers = [
        "</p>",
        "</div>",
        "</section>",
        "</article>",
        "</h1>",
        "</h2>",
        "</h3>",
        "</h4>",
        "</h5>",
        "</h6>",
        "</li>",
        "</tr>",
        "</td>",
        "</th>",
        "</blockquote>",
        "<br>",
        "<br/>",
        "<br />",
    ];
    let mut s = s;
    for tag in &block_closers {
        s = s.replace(tag, "\n");
    }

    // Strip all remaining tags.
    let s = strip_tags(&s);

    // Decode HTML entities.
    let s = decode_entities(&s);

    // Collapse excessive whitespace.
    collapse_whitespace(&s)
}

fn remove_block_tags(html: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut result = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.to_lowercase().find(&open) {
        result.push_str(&rest[..start]);
        let after_open = &rest[start..];
        if let Some(end) = after_open.to_lowercase().find(&close) {
            rest = &after_open[end + close.len()..];
        } else {
            break;
        }
    }
    result.push_str(rest);
    result
}

fn strip_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

pub fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…")
}

fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_newline = false;
    let mut newline_count = 0usize;

    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            newline_count += 1;
            if newline_count <= 2 {
                result.push('\n');
            }
            prev_newline = true;
        } else {
            // Collapse runs of internal whitespace to single space.
            let normalized = collapse_spaces(trimmed);
            if prev_newline && !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&normalized);
            result.push('\n');
            prev_newline = false;
            newline_count = 0;
        }
    }
    result.trim().to_string()
}

fn collapse_spaces(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch == ' ' || ch == '\t' {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(ch);
            prev_space = false;
        }
    }
    result
}

/// Convert a URL to a short human-readable label (domain + path).
pub fn url_to_label(url: &str) -> String {
    // Strip protocol
    let s = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    // Truncate long paths for display
    if s.len() > 80 {
        format!("{}…", &s[..77])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_removes_script_blocks() {
        let html = "<html><head><script>alert(1)</script></head><body><p>Hello</p></body></html>";
        let text = strip_html(html);
        assert!(!text.contains("alert"));
        assert!(text.contains("Hello"));
    }

    #[test]
    fn strip_html_handles_block_tags_as_newlines() {
        let html = "<p>First paragraph</p><p>Second paragraph</p>";
        let text = strip_html(html);
        assert!(text.contains("First paragraph"));
        assert!(text.contains("Second paragraph"));
        // Should have newline separation
        assert!(text.contains('\n'));
    }

    #[test]
    fn strip_html_decodes_entities() {
        let html = "<p>a &amp; b &lt;c&gt; &quot;d&quot; &nbsp;e</p>";
        let text = strip_html(html);
        assert!(text.contains("a & b <c> \"d\""));
    }

    #[test]
    fn strip_html_collapses_whitespace() {
        let html = "<p>   lots   of   spaces   </p>";
        let text = strip_html(html);
        // Should not have multiple internal spaces on same content after strip
        assert!(!text.contains("   "));
    }

    #[test]
    fn extract_loc_tags_parses_sitemap() {
        let xml = r#"<?xml version="1.0"?>
<urlset>
  <url><loc>https://example.com/</loc></url>
  <url><loc>https://example.com/about</loc></url>
  <url><loc>https://example.com/docs</loc></url>
</urlset>"#;
        let urls = extract_loc_tags(xml);
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0], "https://example.com/");
        assert_eq!(urls[2], "https://example.com/docs");
    }

    #[test]
    fn extract_loc_tags_returns_empty_for_invalid_xml() {
        let xml = "not a sitemap";
        let urls = extract_loc_tags(xml);
        assert!(urls.is_empty());
    }

    #[test]
    fn url_to_label_strips_protocol() {
        assert_eq!(url_to_label("https://example.com/page"), "example.com/page");
        assert_eq!(
            url_to_label("http://docs.example.com/api"),
            "docs.example.com/api"
        );
    }

    #[test]
    fn url_to_label_truncates_long_urls() {
        let long_url = format!("https://example.com/{}", "a".repeat(100));
        let label = url_to_label(&long_url);
        assert!(label.len() <= 80);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn remove_block_tags_strips_script() {
        let s = remove_block_tags("before<script>evil()</script>after", "script");
        assert_eq!(s, "beforeafter");
    }

    #[test]
    fn remove_block_tags_case_insensitive() {
        let s = remove_block_tags("a<SCRIPT>x</SCRIPT>b", "script");
        assert_eq!(s, "ab");
    }
}
