use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, TimeZone, Utc};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

// ─── constants ───────────────────────────────────────────────────────────────

pub const MAGIC: &[u8; 4] = b"AIPK";
pub const VERSION: u32 = 1;
pub const HEADER_SIZE: usize = 96;
pub const SECTION_HEADER_SIZE: usize = 16; // type(4) + flags(4) + size(8)

// Package header flags (bytes 84..88).
// 0x04 was historically shared by SIGNED and ENCRYPTED; SIGNED keeps it,
// ENCRYPTED moved to 0x08 (runtime detection uses per-section flags, so
// packages written before the split still load fine).
pub const PKG_FLAG_SIGNED: u32 = 0x04;
pub const PKG_FLAG_ENCRYPTED: u32 = 0x08;
pub const PKG_FLAG_SEALED: u32 = 0x10;

/// Read the package header flags without a full parse.
pub fn header_flags(raw: &[u8]) -> u32 {
    if raw.len() < HEADER_SIZE || &raw[..4] != MAGIC {
        return 0;
    }
    u32::from_le_bytes(raw[84..88].try_into().unwrap_or_default())
}

// ─── types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SectionHeader {
    pub tag: String,
    #[allow(dead_code)]
    pub flags: u32,
    pub size: usize,
    pub offset: usize, // byte offset where DATA starts (after section header)
}

#[derive(Debug)]
pub struct AipkPackage {
    pub name: String,
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub sections: Vec<SectionHeader>,
    pub raw: Vec<u8>, // full file bytes for lazy reading
}

impl AipkPackage {
    pub fn section(&self, tag: &str) -> Option<&SectionHeader> {
        self.sections.iter().find(|s| s.tag == tag)
    }

    pub fn section_data(&self, sec: &SectionHeader) -> &[u8] {
        &self.raw[sec.offset..sec.offset + sec.size]
    }

    pub fn meta(&self) -> Result<toml::Value> {
        let sec = self
            .section("META")
            .ok_or_else(|| anyhow!("no META section"))?;
        let text = std::str::from_utf8(self.section_data(sec))?;
        Ok(toml::from_str(text)?)
    }

    pub fn persona(&self) -> Option<String> {
        let sec = self.section("PERS")?;
        String::from_utf8(self.section_data(sec).to_vec()).ok()
    }

    pub fn tools_json(&self) -> Option<Value> {
        let sec = self.section("TOOL")?;
        serde_json::from_slice(self.section_data(sec)).ok()
    }

    /// Returns list of (name, trigger, content) for each skill
    pub fn skills(&self) -> Vec<SkillEntry> {
        let sec = match self.section("SKIL") {
            Some(s) => s,
            None => return vec![],
        };
        parse_skil_section(self.section_data(sec)).unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub trigger: String,
    pub content: String,
}

// ─── parser ──────────────────────────────────────────────────────────────────

pub fn parse(path: &Path) -> Result<AipkPackage> {
    let raw = std::fs::read(path)?;
    parse_bytes(raw)
}

pub fn parse_bytes(raw: Vec<u8>) -> Result<AipkPackage> {
    if raw.len() < HEADER_SIZE {
        bail!("file too small");
    }
    if &raw[..4] != MAGIC {
        bail!("invalid magic: expected AIPK");
    }

    let version = u32::from_le_bytes(raw[4..8].try_into()?);
    if version == 0 || version > VERSION {
        bail!("unsupported version {version}");
    }

    let name = {
        let name_bytes = &raw[8..72];
        let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(64);
        String::from_utf8_lossy(&name_bytes[..end]).into_owned()
    };

    let created_ts = u64::from_le_bytes(raw[72..80].try_into()?);
    let created_at = Utc
        .timestamp_opt(created_ts as i64, 0)
        .single()
        .unwrap_or_else(Utc::now);

    let _section_count = u32::from_le_bytes(raw[80..84].try_into()?) as usize;
    let _flags = u32::from_le_bytes(raw[84..88].try_into()?);
    // index_offset at [88..96] is deliberately unused: `aipk sign` (cmd::sign::sign_bytes)
    // appends the SIGN section after INDX without updating its directory, so INDX
    // isn't guaranteed complete for signed packages. Sequential parsing sidesteps
    // that gap and is cheap anyway given how few sections a package has.

    let mut sections = Vec::new();
    let mut pos = HEADER_SIZE;
    let mut seen: std::collections::HashSet<String> = Default::default();

    while pos + SECTION_HEADER_SIZE <= raw.len() {
        let tag = String::from_utf8_lossy(&raw[pos..pos + 4])
            .trim_end_matches('\0')
            .to_string();
        let flags = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into()?);
        let size = u64::from_le_bytes(raw[pos + 8..pos + 16].try_into()?) as usize;
        let data_offset = pos + SECTION_HEADER_SIZE;

        if data_offset + size > raw.len() {
            bail!("section '{tag}' size {size} exceeds file bounds");
        }

        if !seen.contains(&tag) || tag == "INDX" {
            sections.push(SectionHeader {
                tag: tag.clone(),
                flags,
                size,
                offset: data_offset,
            });
            seen.insert(tag);
        }

        pos = data_offset + size;
    }

    Ok(AipkPackage {
        name,
        version,
        created_at,
        sections,
        raw,
    })
}

// ─── SKIL section parser ──────────────────────────────────────────────────────

fn parse_skil_section(data: &[u8]) -> Result<Vec<SkillEntry>> {
    if data.len() < 16 {
        return Ok(vec![]);
    }
    let manifest_offset = u64::from_le_bytes(data[0..8].try_into()?) as usize;
    let files_offset = u64::from_le_bytes(data[8..16].try_into()?) as usize;

    // Parse manifest JSON
    let manifest: Vec<Value> = serde_json::from_slice(&data[manifest_offset..files_offset])?;
    let triggers: HashMap<String, String> = manifest
        .iter()
        .filter_map(|e| {
            let name = e["name"].as_str()?.to_string();
            let trigger = e["trigger"].as_str().unwrap_or("").to_string();
            Some((name, trigger))
        })
        .collect();

    // Parse skill files
    let mut skills = Vec::new();
    let mut pos = files_offset;
    while pos + 2 <= data.len() {
        let fname_len = u16::from_le_bytes(data[pos..pos + 2].try_into()?) as usize;
        pos += 2;
        if pos + fname_len > data.len() {
            break;
        }
        let filename = String::from_utf8_lossy(&data[pos..pos + fname_len]).into_owned();
        pos += fname_len;
        if pos + 4 > data.len() {
            break;
        }
        let content_len = u32::from_le_bytes(data[pos..pos + 4].try_into()?) as usize;
        pos += 4;
        if pos + content_len > data.len() {
            break;
        }
        let content = String::from_utf8_lossy(&data[pos..pos + content_len]).into_owned();
        pos += content_len;

        // Extract name from frontmatter or filename
        let name = parse_frontmatter_name(&content).unwrap_or_else(|| {
            Path::new(&filename)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let trigger = triggers.get(&name).cloned().unwrap_or_default();
        skills.push(SkillEntry {
            name,
            trigger,
            content,
        });
    }
    Ok(skills)
}

fn parse_frontmatter_name(content: &str) -> Option<String> {
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("---")? + 3;
    for line in content[3..end].lines() {
        if let Some(val) = line.strip_prefix("name:") {
            return Some(val.trim().to_string());
        }
    }
    None
}

pub fn parse_frontmatter_trigger(content: &str) -> String {
    if !content.starts_with("---") {
        return String::new();
    }
    let end = match content[3..].find("---") {
        Some(e) => e + 3,
        None => return String::new(),
    };
    for line in content[3..end].lines() {
        if let Some(val) = line.strip_prefix("trigger:") {
            return val.trim().to_string();
        }
    }
    String::new()
}

// ─── KNOW section ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct KnowChunk {
    pub id: u32,
    pub text: String,
    pub source: String,
}

/// Parse KNOW section → (chunks, vectors, dim)
/// Binary layout (24-byte header):
///   [0..8]  chunks_offset: u64  (always 24)
///   [8..16] vectors_offset: u64 (24 + len(gzip))
///   [16..20] chunk_count: u32
///   [20..24] dim: u32
///   [24..vectors_offset] gzip-compressed JSONL chunks
///   [vectors_offset..] raw f32 LE vectors
pub fn parse_know_section(data: &[u8]) -> Result<(Vec<KnowChunk>, Vec<Vec<f32>>, u32)> {
    if data.len() < 24 {
        bail!("KNOW section too small");
    }
    let chunks_offset = u64::from_le_bytes(data[0..8].try_into()?) as usize;
    let vectors_offset = u64::from_le_bytes(data[8..16].try_into()?) as usize;
    let chunk_count = u32::from_le_bytes(data[16..20].try_into()?) as usize;
    let dim = u32::from_le_bytes(data[20..24].try_into()?);

    let mut gz = GzDecoder::new(&data[chunks_offset..vectors_offset]);
    let mut jsonl = String::new();
    gz.read_to_string(&mut jsonl)?;

    let chunks: Vec<KnowChunk> = jsonl
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let v: Value = serde_json::from_str(line).ok()?;
            Some(KnowChunk {
                id: v["id"].as_u64().unwrap_or(0) as u32,
                text: v["text"].as_str().unwrap_or("").to_string(),
                source: v["source"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect();

    let vec_data = &data[vectors_offset..];
    let expected = chunk_count * dim as usize * 4;
    if vec_data.len() < expected {
        bail!("vectors data too small: {} < {}", vec_data.len(), expected);
    }
    let vectors: Vec<Vec<f32>> = (0..chunk_count)
        .map(|i| {
            let base = i * dim as usize * 4;
            (0..dim as usize)
                .map(|j| {
                    let off = base + j * 4;
                    f32::from_le_bytes(vec_data[off..off + 4].try_into().unwrap())
                })
                .collect()
        })
        .collect();

    Ok((chunks, vectors, dim))
}

pub fn build_know_section(chunks: &[KnowChunk], vectors: &[Vec<f32>], dim: u32) -> Result<Vec<u8>> {
    let jsonl: String = chunks
        .iter()
        .map(|c| {
            serde_json::to_string(&serde_json::json!({
                "id": c.id, "text": c.text, "source": c.source, "meta": {}
            }))
            .unwrap()
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(jsonl.as_bytes())?;
    let chunks_gz = gz.finish()?;

    let mut vec_bytes: Vec<u8> = Vec::with_capacity(vectors.len() * dim as usize * 4);
    for vec in vectors {
        for &v in vec {
            vec_bytes.extend_from_slice(&v.to_le_bytes());
        }
    }

    let header_size: u64 = 24;
    let chunks_offset: u64 = header_size;
    let vectors_offset: u64 = header_size + chunks_gz.len() as u64;

    let mut out = Vec::new();
    out.extend_from_slice(&chunks_offset.to_le_bytes());
    out.extend_from_slice(&vectors_offset.to_le_bytes());
    out.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
    out.extend_from_slice(&dim.to_le_bytes());
    out.extend_from_slice(&chunks_gz);
    out.extend_from_slice(&vec_bytes);
    Ok(out)
}

// ─── builder ─────────────────────────────────────────────────────────────────

fn write_section(tag: &str, data: &[u8]) -> Vec<u8> {
    let mut tag_bytes = [0u8; 4];
    for (i, b) in tag.bytes().take(4).enumerate() {
        tag_bytes[i] = b;
    }
    let mut out = Vec::with_capacity(SECTION_HEADER_SIZE + data.len());
    out.extend_from_slice(&tag_bytes);
    out.extend_from_slice(&0u32.to_le_bytes()); // flags
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(data);
    out
}

pub struct AipkBuilder {
    pub name: String,
    sections: Vec<(String, Vec<u8>)>,
}

impl AipkBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sections: Vec::new(),
        }
    }

    pub fn add(&mut self, tag: impl Into<String>, data: Vec<u8>) -> &mut Self {
        self.sections.push((tag.into(), data));
        self
    }

    pub fn build(self) -> Vec<u8> {
        let mut pos = HEADER_SIZE;
        let mut indx_entries: Vec<(String, usize, usize)> = Vec::new();
        let mut content: Vec<u8> = Vec::new();

        for (tag, data) in &self.sections {
            let data_offset = pos + SECTION_HEADER_SIZE;
            indx_entries.push((tag.clone(), data_offset, data.len()));
            content.extend(write_section(tag, data));
            pos += SECTION_HEADER_SIZE + data.len();
        }

        let index_section_offset = pos;

        // Build INDX data
        let mut indx_data = (indx_entries.len() as u32).to_le_bytes().to_vec();
        for (tag, offset, size) in &indx_entries {
            let mut tag_bytes = [0u8; 4];
            for (i, b) in tag.bytes().take(4).enumerate() {
                tag_bytes[i] = b;
            }
            indx_data.extend_from_slice(&tag_bytes);
            indx_data.extend_from_slice(&(*offset as u64).to_le_bytes());
            indx_data.extend_from_slice(&(*size as u64).to_le_bytes());
        }
        content.extend(write_section("INDX", &indx_data));

        // Header
        let section_count = self.sections.len() + 1; // +1 for INDX
        let mut name_bytes = [0u8; 64];
        for (i, b) in self.name.bytes().take(63).enumerate() {
            name_bytes[i] = b;
        }

        let mut header = Vec::with_capacity(HEADER_SIZE);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&VERSION.to_le_bytes());
        header.extend_from_slice(&name_bytes);
        header.extend_from_slice(
            &(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs())
            .to_le_bytes(),
        );
        header.extend_from_slice(&(section_count as u32).to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes()); // flags
        header.extend_from_slice(&(index_section_offset as u64).to_le_bytes());
        assert_eq!(header.len(), HEADER_SIZE);

        [header, content].concat()
    }
}

/// Build SKIL section bytes from skill files on disk
pub fn build_skil_section(skills: &[SkillDef]) -> Vec<u8> {
    let manifest: Vec<serde_json::Value> = skills
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "file": format!("skills/{}", s.filename),
                "trigger": s.trigger,
            })
        })
        .collect();
    let manifest_json = serde_json::to_vec(&manifest).unwrap();

    let mut files_data: Vec<u8> = Vec::new();
    for s in skills {
        let filename = format!("skills/{}", s.filename);
        let fname_bytes = filename.as_bytes();
        let content_bytes = s.content.as_bytes();
        files_data.extend_from_slice(&(fname_bytes.len() as u16).to_le_bytes());
        files_data.extend_from_slice(fname_bytes);
        files_data.extend_from_slice(&(content_bytes.len() as u32).to_le_bytes());
        files_data.extend_from_slice(content_bytes);
    }

    let header_size: u64 = 16;
    let manifest_offset: u64 = header_size;
    let files_offset: u64 = header_size + manifest_json.len() as u64;

    let mut out = Vec::new();
    out.extend_from_slice(&manifest_offset.to_le_bytes());
    out.extend_from_slice(&files_offset.to_le_bytes());
    out.extend_from_slice(&manifest_json);
    out.extend_from_slice(&files_data);
    out
}

pub struct SkillDef {
    pub name: String,
    pub filename: String,
    pub trigger: String,
    pub content: String,
}
