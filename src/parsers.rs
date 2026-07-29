//! Text extraction from user documents. One entry point — `extract_text` —
//! turns a file into plain text ready for chunking, or `None` when the
//! format is not supported.

use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;

use crate::cmd::add_web::{decode_entities, strip_html};

/// Extensions read verbatim as UTF-8 text.
const PLAIN_TEXT_EXTS: &[&str] = &[
    "md", "mdx", "txt", "rst", "adoc", "org", "tex", "log", "json", "jsonl", "yaml", "yml", "toml",
    "ini", "cfg",
];

/// Extract plain text from `path` based on its extension.
/// Returns `Ok(None)` for unsupported formats so callers can skip silently.
pub fn extract_text(path: &Path) -> Result<Option<String>> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let text = match ext.as_str() {
        e if PLAIN_TEXT_EXTS.contains(&e) => {
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
        }
        "html" | "htm" | "xhtml" => {
            let html = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            strip_html(&html)
        }
        "pdf" => extract_pdf(path)?,
        "docx" => extract_docx(path)?,
        "csv" => extract_csv(path, b',')?,
        "tsv" => extract_csv(path, b'\t')?,
        _ => return Ok(None),
    };
    Ok(Some(text))
}

/// True when `extract_text` knows how to handle this file.
pub fn is_supported(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    PLAIN_TEXT_EXTS.contains(&ext.as_str())
        || matches!(
            ext.as_str(),
            "html" | "htm" | "xhtml" | "pdf" | "docx" | "csv" | "tsv"
        )
}

// ─── PDF ─────────────────────────────────────────────────────────────────────

fn extract_pdf(path: &Path) -> Result<String> {
    // pdf-extract can panic on malformed files — contain it.
    let owned = path.to_path_buf();
    let result = std::panic::catch_unwind(|| pdf_extract::extract_text(&owned));
    match result {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => anyhow::bail!("PDF extraction failed for {}: {e}", path.display()),
        Err(_) => anyhow::bail!("PDF parser crashed on {}", path.display()),
    }
}

// ─── DOCX ────────────────────────────────────────────────────────────────────

/// A .docx is a zip archive; the text lives in word/document.xml.
/// Paragraph closes (`</w:p>`) become blank lines so the chunker sees
/// paragraph boundaries; all other tags are stripped.
fn extract_docx(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).with_context(|| format!("not a zip: {}", path.display()))?;
    let mut doc = archive.by_name("word/document.xml").map_err(|_| {
        anyhow::anyhow!(
            "{}: word/document.xml not found — not a DOCX?",
            path.display()
        )
    })?;
    let mut xml = String::new();
    doc.read_to_string(&mut xml)?;
    Ok(docx_xml_to_text(&xml))
}

fn docx_xml_to_text(xml: &str) -> String {
    let with_breaks = xml
        .replace("</w:p>", "\n\n")
        .replace("<w:br/>", "\n")
        .replace("<w:tab/>", " ");
    let mut out = String::with_capacity(with_breaks.len() / 4);
    let mut in_tag = false;
    for ch in with_breaks.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = decode_entities(&out);
    // Collapse runs of 3+ newlines left by empty paragraphs
    let mut result = String::with_capacity(decoded.len());
    let mut newlines = 0;
    for ch in decoded.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines <= 2 {
                result.push(ch);
            }
        } else {
            newlines = 0;
            result.push(ch);
        }
    }
    result.trim().to_string()
}

// ─── CSV / TSV ───────────────────────────────────────────────────────────────

/// Each row becomes one paragraph: `header1: v1 | header2: v2`.
/// Paragraphs are what the chunker splits on, so a row never gets torn apart.
fn extract_csv(path: &Path, delimiter: u8) -> Result<String> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_path(path)
        .with_context(|| format!("reading {}", path.display()))?;

    let headers: Vec<String> = reader
        .headers()
        .map(|h| h.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();

    let mut paragraphs: Vec<String> = Vec::new();
    for record in reader.records() {
        let record = match record {
            Ok(r) => r,
            Err(_) => continue,
        };
        let fields: Vec<String> = record
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.trim().is_empty())
            .map(|(i, v)| match headers.get(i) {
                Some(h) if !h.trim().is_empty() => format!("{}: {}", h.trim(), v.trim()),
                _ => v.trim().to_string(),
            })
            .collect();
        if !fields.is_empty() {
            paragraphs.push(fields.join(" | "));
        }
    }
    Ok(paragraphs.join("\n\n"))
}

/// True when the '.' just pushed onto `current` sits between two digits
/// ("8.4 kWh", "v3.1") — a decimal point, not a sentence boundary.
pub fn is_decimal_point(current: &str, next: Option<&char>) -> bool {
    current.ends_with('.')
        && current
            .chars()
            .rev()
            .nth(1)
            .is_some_and(|c| c.is_ascii_digit())
        && next.is_some_and(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn plain_text_passthrough() {
        let dir = std::env::temp_dir().join("aipk_parsers_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("note.md");
        std::fs::write(&p, "# Hello\n\nWorld").unwrap();
        assert_eq!(extract_text(&p).unwrap().unwrap(), "# Hello\n\nWorld");
    }

    #[test]
    fn unsupported_extension_is_none() {
        assert!(extract_text(Path::new("binary.exe")).unwrap().is_none());
        assert!(!is_supported(Path::new("photo.jpg")));
        assert!(is_supported(Path::new("doc.pdf")));
        assert!(is_supported(Path::new("data.csv")));
    }

    #[test]
    fn csv_rows_become_labeled_paragraphs() {
        let dir = std::env::temp_dir().join("aipk_parsers_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("faq.csv");
        std::fs::write(&p, "question,answer\nWhat is AIPK?,A package format\n,\n").unwrap();
        let text = extract_text(&p).unwrap().unwrap();
        assert_eq!(text, "question: What is AIPK? | answer: A package format");
    }

    #[test]
    fn docx_xml_paragraphs_and_entities() {
        let xml = r#"<w:document><w:p><w:r><w:t>First &amp; foremost</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:document>"#;
        let text = docx_xml_to_text(xml);
        assert_eq!(text, "First & foremost\n\nSecond");
    }

    #[test]
    fn docx_roundtrip_via_zip() {
        let dir = std::env::temp_dir().join("aipk_parsers_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("mini.docx");
        let f = std::fs::File::create(&p).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        zw.start_file::<_, ()>("word/document.xml", Default::default())
            .unwrap();
        zw.write_all(b"<w:document><w:p><w:t>Hello docx</w:t></w:p></w:document>")
            .unwrap();
        zw.finish().unwrap();
        assert_eq!(extract_text(&p).unwrap().unwrap(), "Hello docx");
    }
}
