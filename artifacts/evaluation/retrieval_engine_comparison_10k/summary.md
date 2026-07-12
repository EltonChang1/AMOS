# Governed Retrieval-Engine Comparison

- Distractors: 10000
- Queries: 24
- Vector model revision: `1110a243fdf4706b3f48f1d95db1a4f5529b4d41`

| Engine | Top-1 | Recall@5 | MRR | p50 (s) | p95 (s) | Permission leaks | Superseded leaks |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| bm25_governed | 0.666667 | 0.75 | 0.708333 | 0.004874 | 2.035807 | 0 | 0 |
| minilm_hnsw_governed | 0.708333 | 0.708333 | 0.708333 | 0.011748 | 0.016194 | 0 | 0 |
| rrf_hybrid_governed | 0.666667 | 0.875 | 0.751389 | 0.018118 | 1.859767 | 0 | 0 |

Evidence boundary: Internally authored synthetic relevance cases with templated distractors on one machine. This compares local candidate engines and governed output behavior; it is not independent retrieval evaluation, a distributed-store benchmark, or deployed-product evidence.
