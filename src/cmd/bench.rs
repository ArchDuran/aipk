//! `aipk bench scale` — measures the KNOW section's binary I/O and both
//! retrieval paths (brute-force cosine scan and the HNSW `AnnIndex`) on
//! synthetic vectors, at chunk counts real corpora only reach after heavy
//! use (10k-1M). This does not require an embedding backend.
//!
//! Vectors are generated as noisy draws around a growing set of cluster
//! centers (topic count scales with corpus size), not uniform random —
//! **uniform random high-dimensional vectors are a misleading worst case for
//! both ANN recall and even brute-force "top-k" meaningfulness**: at dim=768,
//! random points concentrate so tightly around the mean pairwise distance
//! that "the nearest 5" are barely closer than "the nearest 5000", so even
//! exact brute force is picking among near-ties. Real text embeddings
//! cluster semantically (a corpus about Kubernetes has real topic
//! structure), so clustered synthetic vectors are the more honest stand-in.
//! This was discovered mid-implementation — see the AIPK devlog/commit
//! history for the investigation that motivated it. Recall numbers here
//! still say nothing about grounding *quality* (see bench/results.md for
//! that) — only about how each search strategy scales and how closely ANN
//! tracks brute force on a semantically-structured distribution.

use crate::ann::AnnIndex;
use crate::format::{build_know_section, parse_know_section, KnowChunk};
use crate::runtime::cosine_similarity;
use anyhow::Result;
use rand::Rng;
use serde_json::json;
use std::collections::HashSet;
use std::time::Instant;

const DIM: usize = 768; // matches nomic-embed-text
const NOISE: f32 = 0.15;

fn random_unit_vector(rng: &mut impl Rng, dim: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    v
}

/// A point near cluster center `c`: nudge every coordinate by ±NOISE, then
/// re-normalize back to unit length.
fn noisy_unit_vector(rng: &mut impl Rng, center: &[f32]) -> Vec<f32> {
    let mut v: Vec<f32> = center
        .iter()
        .map(|x| x + rng.gen_range(-NOISE..NOISE))
        .collect();
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    v
}

/// Cluster centers for a corpus of `n` chunks. Topic count grows with corpus
/// size (roughly one new topic per 200 chunks, floor 10) rather than staying
/// fixed — a knowledge base that's 10x bigger typically covers more ground,
/// not just deeper repetition of the same handful of topics.
fn cluster_centers(rng: &mut impl Rng, n: usize, dim: usize) -> Vec<Vec<f32>> {
    let count = (n / 200).max(10);
    (0..count).map(|_| random_unit_vector(rng, dim)).collect()
}

/// Resident set size of this process, in KB. Linux-only (reads /proc); returns
/// None elsewhere rather than pretending to measure something it can't.
fn rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.trim().trim_end_matches("kB").trim().parse().ok();
        }
    }
    None
}

fn percentile(sorted_asc: &[f64], p: f64) -> f64 {
    if sorted_asc.is_empty() {
        return 0.0;
    }
    let idx = (((sorted_asc.len() - 1) as f64) * p).round() as usize;
    sorted_asc[idx]
}

pub fn run(sizes: Vec<usize>, queries: usize, as_json: bool) -> Result<()> {
    let mut rng = rand::thread_rng();
    let mut rows = Vec::new();

    for &n in &sizes {
        eprintln!("== {n} chunks ==");

        let chunks: Vec<KnowChunk> = (0..n)
            .map(|i| KnowChunk {
                id: i as u32,
                text: format!(
                    "Synthetic chunk #{i}: lorem ipsum dolor sit amet, consectetur \
                     adipiscing elit, sed do eiusmod tempor incididunt ut labore et \
                     dolore magna aliqua, chunk index {i} of {n}."
                ),
                source: format!("synthetic_{}.md", i / 100),
            })
            .collect();
        let centers = cluster_centers(&mut rng, n, DIM);
        let vectors: Vec<Vec<f32>> = (0..n)
            .map(|_| {
                let c = &centers[rng.gen_range(0..centers.len())];
                noisy_unit_vector(&mut rng, c)
            })
            .collect();

        let build_start = Instant::now();
        let section = build_know_section(&chunks, &vectors, DIM as u32)?;
        let build_ms = build_start.elapsed().as_secs_f64() * 1000.0;
        let section_bytes = section.len();

        let load_start = Instant::now();
        let (parsed_chunks, parsed_vectors, _dim) = parse_know_section(&section)?;
        let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
        anyhow::ensure!(parsed_chunks.len() == n, "roundtrip lost chunks");

        // Queries drawn from the same cluster distribution as the corpus —
        // a real question tends to land near existing topic content, not at
        // a uniformly random point in embedding space.
        let queries_vecs: Vec<Vec<f32>> = (0..queries)
            .map(|_| {
                let c = &centers[rng.gen_range(0..centers.len())];
                noisy_unit_vector(&mut rng, c)
            })
            .collect();

        let mut brute_latencies_us: Vec<f64> = Vec::with_capacity(queries);
        let mut ground_truth: Vec<HashSet<usize>> = Vec::with_capacity(queries);
        for q in &queries_vecs {
            let t0 = Instant::now();
            let mut scored: Vec<(f32, usize)> = parsed_vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (cosine_similarity(q, v), i))
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            brute_latencies_us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
            ground_truth.push(scored.into_iter().take(5).map(|(_, i)| i).collect());
        }
        brute_latencies_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let brute_p50_us = percentile(&brute_latencies_us, 0.50);
        let brute_p95_us = percentile(&brute_latencies_us, 0.95);

        let ann_build_start = Instant::now();
        let ann = AnnIndex::build(&parsed_vectors);
        let ann_build_ms = ann_build_start.elapsed().as_secs_f64() * 1000.0;

        let (ann_p50_us, ann_p95_us, recall_at_5) = if let Some(ann) = &ann {
            let mut ann_latencies_us: Vec<f64> = Vec::with_capacity(queries);
            let mut total_recall = 0.0;
            for (q, truth) in queries_vecs.iter().zip(ground_truth.iter()) {
                let t0 = Instant::now();
                let results = ann.search(q, 5);
                ann_latencies_us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                let hits = results.iter().filter(|(i, _)| truth.contains(i)).count();
                total_recall += hits as f64 / 5.0;
            }
            ann_latencies_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
            (
                percentile(&ann_latencies_us, 0.50),
                percentile(&ann_latencies_us, 0.95),
                total_recall / queries as f64,
            )
        } else {
            (0.0, 0.0, 1.0)
        };

        let rss = rss_kb();

        rows.push(json!({
            "chunks": n,
            "know_section_mb": section_bytes as f64 / (1024.0 * 1024.0),
            "build_ms": build_ms,
            "load_ms": load_ms,
            "brute_retrieval_p50_us": brute_p50_us,
            "brute_retrieval_p95_us": brute_p95_us,
            "ann_build_ms": ann_build_ms,
            "ann_retrieval_p50_us": ann_p50_us,
            "ann_retrieval_p95_us": ann_p95_us,
            "ann_recall_at_5": recall_at_5,
            "rss_mb": rss.map(|kb| kb as f64 / 1024.0),
        }));
    }

    if as_json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!(
            "\n{:>10} {:>12} {:>9} {:>9} {:>12} {:>12} {:>11} {:>12} {:>12} {:>9} {:>9}",
            "chunks",
            "KNOW (MB)",
            "build ms",
            "load ms",
            "brute p50µs",
            "brute p95µs",
            "ann build ms",
            "ann p50µs",
            "ann p95µs",
            "recall@5",
            "RSS (MB)"
        );
        for r in &rows {
            println!(
                "{:>10} {:>12.2} {:>9.1} {:>9.1} {:>12.1} {:>12.1} {:>11.1} {:>12.1} {:>12.1} {:>9.3} {:>9}",
                r["chunks"],
                r["know_section_mb"].as_f64().unwrap_or(0.0),
                r["build_ms"].as_f64().unwrap_or(0.0),
                r["load_ms"].as_f64().unwrap_or(0.0),
                r["brute_retrieval_p50_us"].as_f64().unwrap_or(0.0),
                r["brute_retrieval_p95_us"].as_f64().unwrap_or(0.0),
                r["ann_build_ms"].as_f64().unwrap_or(0.0),
                r["ann_retrieval_p50_us"].as_f64().unwrap_or(0.0),
                r["ann_retrieval_p95_us"].as_f64().unwrap_or(0.0),
                r["ann_recall_at_5"].as_f64().unwrap_or(0.0),
                r["rss_mb"]
                    .as_f64()
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "n/a".into()),
            );
        }
        println!(
            "\nSynthetic vectors (clustered around ~n/200 topics, unit-normalized, dim={DIM} \
             — see module docs on why not uniform random) — measures the .aipk binary format \
             and both retrieval strategies, not embedding/retrieval quality. recall@5 compares \
             the HNSW AnnIndex's top-5 against brute-force ground truth on this same synthetic \
             distribution. ann_build_ms reflects a one-time cost paid at `aipk build`/`pipeline` \
             time, not on every load — see README for details. See bench/results.md for the \
             grounding-quality benchmark on real content."
        );
    }

    Ok(())
}
