//! Approximate nearest-neighbor index for KNOW-section retrieval.
//!
//! `bench/scale-results.md` measured brute-force cosine search at O(n) per
//! query: ~123ms at 100k chunks, ~1.23s at 1M — past a point that's no longer
//! acceptable for an interactive chat response. This wraps `instant-distance`
//! (pure-Rust HNSW) behind the same cosine-similarity contract the
//! brute-force path already exposes, so callers don't need to know which one
//! ran.
//!
//! Vectors are unit-normalized before indexing so that squared Euclidean
//! distance is monotonic with cosine similarity: for unit vectors,
//! `||a-b||^2 = 2 - 2*cos_sim(a,b)`. We index on that squared-Euclidean
//! distance and convert back to a cosine score on the way out.
//!
//! Building the index is expensive at scale (~100s for 100k chunks at
//! dim=768 with default ef_construction — see the investigation in
//! bench/scale-results.md) and most commands that load a package
//! (`run`, `test`) do so once per invocation, sometimes once per *query*
//! within a single process. Building on every load would make those far
//! slower than the brute-force scan it's meant to replace. So the index is
//! never built implicitly on load: `aipk build`/`pipeline` build it once (see
//! `format::build_annx_section`) and ship it inside the package as the ANNX
//! section; `KnowRuntime::load` only ever *deserializes* it. Packages built
//! before ANNX existed, or below the size threshold, simply have no index
//! and fall back to brute force — correct, just not accelerated.

use instant_distance::{Builder, HnswMap, Point, Search};
use serde::{Deserialize, Serialize};

/// A chunk's embedding, pre-normalized to unit length for indexing.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct UnitVec(Vec<f32>);

impl Point for UnitVec {
    fn distance(&self, other: &Self) -> f32 {
        self.0
            .iter()
            .zip(other.0.iter())
            .map(|(x, y)| (x - y) * (x - y))
            .sum()
    }
}

fn normalize(v: &[f32]) -> UnitVec {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        UnitVec(v.to_vec())
    } else {
        UnitVec(v.iter().map(|x| x / norm).collect())
    }
}

/// Squared-Euclidean-on-unit-vectors distance back to a cosine similarity.
fn sq_dist_to_cosine(sq_dist: f32) -> f32 {
    1.0 - sq_dist / 2.0
}

/// Below this many chunks, brute-force cosine scan stays well under 25ms
/// (see bench/scale-results.md) and isn't worth an index's build time or
/// package-size cost. `aipk build`/`pipeline` only emit an ANNX section
/// above this threshold.
pub const ANN_INDEX_THRESHOLD: usize = 20_000;

/// ef_construction/ef_search used when building the persisted index. Measured
/// on clustered (realistic) 768-dim vectors at 100k chunks: ef=100 gives
/// recall@5=0.993 in ~98s build; ef=48 gives recall@5=0.927 in ~58s; ef=24
/// collapses to recall@5=0.467. Since the build only happens once at package
/// build time (not on every load), the higher-recall/slower setting is the
/// right trade — see bench/scale-results.md for the full investigation,
/// including why *uniform random* synthetic vectors are a misleading
/// worst-case for this measurement.
const EF: usize = 100;

pub struct AnnIndex {
    map: HnswMap<UnitVec, u32>,
}

impl AnnIndex {
    /// Builds an index over `vectors`, indexed positionally (value = original
    /// index into the caller's vector/chunk arrays). Returns `None` if there's
    /// nothing to index — callers should fall back to brute force. Expensive
    /// at scale (see module docs) — only call this at package-build time, not
    /// on every load.
    pub fn build(vectors: &[Vec<f32>]) -> Option<Self> {
        Self::build_with_params(vectors, EF, EF)
    }

    pub fn build_with_params(
        vectors: &[Vec<f32>],
        ef_construction: usize,
        ef_search: usize,
    ) -> Option<Self> {
        if vectors.is_empty() {
            return None;
        }
        let points: Vec<UnitVec> = vectors.iter().map(|v| normalize(v)).collect();
        let values: Vec<u32> = (0..vectors.len() as u32).collect();
        // Fixed seed: index build (and therefore search results) must be
        // reproducible across runs of the same package, not just within one.
        let map = Builder::default()
            .seed(0xA1F4)
            .ef_construction(ef_construction)
            .ef_search(ef_search)
            .build(points, values);
        Some(Self { map })
    }

    /// Returns up to `top_k` (original_index, cosine_score) pairs, best first.
    pub fn search(&self, query_vec: &[f32], top_k: usize) -> Vec<(usize, f32)> {
        if query_vec.is_empty() {
            return vec![];
        }
        let query = normalize(query_vec);
        let mut search = Search::default();
        self.map
            .search(&query, &mut search)
            .take(top_k)
            .map(|item| (*item.value as usize, sq_dist_to_cosine(item.distance)))
            .collect()
    }

    /// Serializes the index for storage in a package's ANNX section.
    pub fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        Ok(bincode::serialize(&self.map)?)
    }

    /// Deserializes an index previously written by `to_bytes`.
    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            map: bincode::deserialize(data)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::cosine_similarity;
    use rand::Rng;

    fn random_unit_vector(rng: &mut impl Rng, dim: usize) -> Vec<f32> {
        let mut v: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in v.iter_mut() {
            *x /= norm;
        }
        v
    }

    #[test]
    fn empty_vectors_build_none() {
        assert!(AnnIndex::build(&[]).is_none());
    }

    #[test]
    fn finds_exact_match() {
        let vectors = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        let index = AnnIndex::build(&vectors).unwrap();
        let results = index.search(&[0.0, 1.0, 0.0], 1);
        assert_eq!(results[0].0, 1);
        assert!((results[0].1 - 1.0).abs() < 1e-4);
    }

    #[test]
    fn cosine_score_matches_brute_force_definition() {
        let vectors = vec![vec![1.0, 0.0], vec![1.0, 1.0], vec![0.0, 1.0]];
        let index = AnnIndex::build(&vectors).unwrap();
        let query = vec![1.0, 0.5];
        let results = index.search(&query, 3);
        for (i, score) in &results {
            let expected = cosine_similarity(&query, &vectors[*i]);
            assert!(
                (score - expected).abs() < 1e-3,
                "index {i}: ann={score} brute={expected}"
            );
        }
    }

    #[test]
    fn roundtrips_through_bytes() {
        let vectors = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        let index = AnnIndex::build(&vectors).unwrap();
        let bytes = index.to_bytes().unwrap();
        let restored = AnnIndex::from_bytes(&bytes).unwrap();
        assert_eq!(
            index.search(&[0.0, 1.0, 0.0], 1),
            restored.search(&[0.0, 1.0, 0.0], 1)
        );
    }

    /// Recall@10 against brute-force ground truth on random vectors — HNSW is
    /// approximate, so this asserts "good enough for retrieval", not exact
    /// equality with brute force.
    #[test]
    fn recall_at_10_is_high_on_random_vectors() {
        let mut rng = rand::thread_rng();
        let dim = 64;
        let n = 2000;
        let vectors: Vec<Vec<f32>> = (0..n).map(|_| random_unit_vector(&mut rng, dim)).collect();
        let index = AnnIndex::build(&vectors).unwrap();

        let mut total_recall = 0.0;
        let num_queries = 20;
        for _ in 0..num_queries {
            let query = random_unit_vector(&mut rng, dim);

            let mut brute: Vec<(usize, f32)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (i, cosine_similarity(&query, v)))
                .collect();
            brute.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let ground_truth: std::collections::HashSet<usize> =
                brute.into_iter().take(10).map(|(i, _)| i).collect();

            let ann_results = index.search(&query, 10);
            let hits = ann_results
                .iter()
                .filter(|(i, _)| ground_truth.contains(i))
                .count();
            total_recall += hits as f64 / 10.0;
        }
        let avg_recall = total_recall / num_queries as f64;
        assert!(
            avg_recall >= 0.9,
            "recall@10 too low: {avg_recall} (expected >= 0.9)"
        );
    }
}
