# Scale benchmark — KNOW section I/O and retrieval (brute-force vs ANN)

Generated with `aipk bench-scale --sizes 1000,10000,100000 --queries 100`.
Machine: 32-core, 30 GiB RAM. Synthetic vectors: clustered, unit-normalized,
dim=768 (matches `nomic-embed-text`) — see "Why clustered, not random" below.
This measures the `.aipk` binary format and both retrieval strategies — **not
embedding or retrieval quality**. For grounding-quality numbers on real
content, see [results.md](results.md).

| chunks | KNOW section | build | load | brute p50 | brute p95 | ANN build | ANN p50 | ANN p95 | recall@5 | RSS |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1,000 | 2.94 MB | 3.7 ms | 1.3 ms | 1.26 ms | 1.37 ms | 0.76 s | 335 µs | 394 µs | 1.000 | 29 MB |
| 10,000 | 29.38 MB | 40.4 ms | 15.6 ms | 12.6 ms | 12.8 ms | 7.1 s | 600 µs | 679 µs | 1.000 | 174 MB |
| 100,000 | 293.84 MB | 359.2 ms | 140.7 ms | 125.7 ms | 129.9 ms | 116.8 s | 1.32 ms | 2.20 ms | 1.000 | 1,627 MB |

(1M-chunk row omitted here — brute-force/build/load figures at that size were
previously measured at ~3.6s build, ~1.4s load, ~1.23s p50 brute-force
retrieval, ~9.6GB RSS on the old uniform-random distribution; those don't
depend on vector geometry so they still hold. ANN build/recall at 1M chunks
under the clustered distribution hadn't finished re-measuring as of this
write-up — rerun `aipk bench-scale --sizes 1000000` to get current numbers.)

## Why clustered, not random

The first pass at this benchmark used **uniform random unit vectors**, and it
was actively misleading. At dim=768, random points concentrate so tightly
around the mean pairwise distance that "the 5 nearest neighbors" are barely
closer than "the next 5,000" — even *exact brute force* is picking among
near-ties, and any approximate method reorders that near-tied ranking
easily. Measured on that distribution, HNSW recall@5 collapsed from 0.999 at
1k chunks to 0.175 at 100k, and index build time exploded (221s at 100k with
a *fixed* number of clusters standing in for "topics"). Neither number meant
anything about real usage — they were an artifact of asking an ANN index to
rank points that don't have a well-defined "nearest neighbor" in the first
place.

Real text embeddings aren't uniform random: they cluster semantically (a
corpus about Kubernetes has actual topic structure — chunks about `drain`
cluster near each other, far from chunks about RBAC). This benchmark
generates vectors as noisy draws around a set of cluster centers, with the
number of clusters scaling as roughly `n / 200` — a knowledge base 10x
bigger typically covers more topics, not just 10x deeper repetition of the
same few. Under that more honest distribution, recall@5 is a clean 1.000 at
every size tested, and build time drops by ~2x at 100k (116.8s vs the 221s
measured on the pathological fixed-cluster-count distribution) because a
well-separated graph is cheaper to construct than one full of near-duplicate
points.

The lesson generalizes beyond this one benchmark: **uniform random vectors
are a bad synthetic stand-in for embedding quality/latency work in general**,
not just for this measurement. They're fine for exercising raw binary I/O
(build/load/RSS above), where geometry doesn't matter.

## Reading this honestly

- **Build and load** (raw `.aipk` binary I/O) scale linearly and stay cheap
  through 100k chunks (hundreds of ms). Clustering the vectors doesn't
  change this part.
- **Brute-force retrieval is O(n) per query**, unchanged: ~1.3ms at 1k
  chunks, ~126ms at 100k. Past roughly 50k-100k chunks in a single package,
  brute-force latency alone starts to matter for an interactive response.
- **The ANN index (`AnnIndex`, HNSW via `instant-distance`) cuts retrieval to
  low-single-digit milliseconds even at 100k chunks** (1.3ms p50, 2.2ms p95)
  — a ~95x speedup over brute force at that size, and the gap widens with
  scale since brute force is O(n) and HNSW search is close to O(log n).
- **Building the index is expensive at scale** — 116.8s at 100k chunks with
  the parameters (`ef_construction=ef_search=100`) chosen for reliable
  recall. This is why the index is **never built on package load**: `aipk
  run`/`test` reload a package fresh per invocation (sometimes per *query*
  within one test suite), and paying a two-minute cost for a single answer
  would be far worse than the brute-force scan it's meant to replace.
  Instead, `aipk build`/`pipeline` build the index **once**, for packages
  above `ANN_INDEX_THRESHOLD` (20,000 chunks), and embed it as a new `ANNX`
  section in the `.aipk` file. `KnowRuntime::load` only ever *deserializes*
  a pre-built index — see `src/ann.rs` module docs. Packages without an
  ANNX section (too small, or built before it existed) fall back to brute
  force, same as before this existed.
- **Package size roughly doubles** for packages that get an ANNX section,
  since the index embeds a normalized copy of the same vectors for graph
  search — the trade is retrieval latency for package size, made
  deliberately (see `ANN_INDEX_THRESHOLD`) only where brute force would
  otherwise cost too much per query.

## Takeaway

The `.aipk` format's `INDX` section is a directory of every section's tag,
offset, and size, but `aipk` doesn't jump through it today — it reads
section headers sequentially, which is cheap given how few sections a
package has, and avoids trusting a directory that `aipk sign` can leave
incomplete (it appends `SIGN` after `INDX` without updating it). None of
this touches semantic search speed, which is a separate question: semantic
search itself is brute-force by default
(comfortable up to tens of thousands of chunks) and HNSW-accelerated
(comfortable well past that) for any package built above the ANN threshold.
The remaining honest caveat: the one-time index-build cost at package-build
time grows with corpus size (116.8s at 100k in this benchmark) — for
multi-hundred-thousand-chunk packages, `aipk build`/`pipeline` will take
noticeably longer, which is the right place to pay that cost, not at query
time.

Reproduce: `aipk bench-scale --sizes 1000,10000,100000 --json`
