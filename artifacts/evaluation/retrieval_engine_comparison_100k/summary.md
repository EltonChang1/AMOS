# Governed Retrieval-Engine Comparison

- Distractors: 100000
- Queries: 24
- Vector model revision: `1110a243fdf4706b3f48f1d95db1a4f5529b4d41`

| Engine | Top-1 | Recall@5 | MRR | p50 (s) | p95 (s) | Permission leaks | Superseded leaks |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| bm25_governed | 0.666667 | 0.75 | 0.708333 | 0.010884 | 17.32559 | 0 | 0 |
| minilm_hnsw_governed | 0.208333 | 0.208333 | 0.208333 | 0.015742 | 0.030328 | 0 | 0 |
| rrf_hybrid_governed | 0.625 | 0.833333 | 0.704861 | 0.020914 | 16.746061 | 0 | 0 |

Evidence boundary: Internally authored synthetic relevance cases with templated distractors on one machine. This compares local candidate engines and governed output behavior; it is not independent retrieval evaluation, a distributed-store benchmark, or deployed-product evidence.
