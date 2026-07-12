# AMOS frozen evaluation results

These are the fixed-workload results reported in the paper. Machine-readable outputs in this directory preserve the benchmark, extended probes, governed retrieval comparisons, and indexed scale studies.

## 1. Analytical-state invariant benchmark

| System | Expected outcomes matched | Rate |
|---|---:|---:|
| **AMOS** | **12/12** | **1.000** |
| Catalog + lineage + dbt agent | 6/12 | 0.500 |
| Metadata-filtered RAG | 3/12 | 0.250 |
| Semantic-layer agent | 3/12 | 0.250 |
| Strong long context | 3/12 | 0.250 |
| Structured tool-using LLM | 1/12 | 0.083 |

An outcome includes the correct governed decision: pass, warning, rejection, repair, downgrade, or `needs_review`.

## 2. Multi-domain capability contract

Twelve seeded variants were executed three times in each domain. A variant passes only when every repeat satisfies task correctness, permission safety, review boundary, claim provenance, and replay.

| System | Payment failure | Subscription churn | Warehouse quality | Provenance / replay |
|---|---:|---:|---:|---:|
| **AMOS** | **12/12** | **12/12** | **12/12** | **1.0 / 1.0** |
| Strong baselines (policy prompt; permission-filtered RAG) | 0/12 | 0/12 | 0/12 | 0.0 / 0.0 |
| OSS-shaped RAG, metrics, and OpenLineage adapters | 0/12 | 0/12 | 0/12 | 0.0 / 0.0 |
| Simple RAG, semantic, catalog, long-context, and agent policies | 0/12 | 0/12 | 0/12 | 0.0 / 0.0 |
| AMOS without verifier | 0/12 | 3/12 | 0/12 | 1.0 / 0.5–0.75 |
| AMOS without permission gate | 0/12 | 0/12 | 0/12 | 1.0 / 1.0 |
| AMOS without provenance | 0/12 | 0/12 | 0/12 | 0.0 / 1.0 |

The metric-axis traces show why full-contract scores differ: several baselines reach 1.0 on task, SQL, metric, schema, permission, and review axes in the payment domain while scoring 0.0 on provenance and replay.

## 3. Component ablations

| Removed component | Benchmark pass rate | Principal affected invariants |
|---|---:|---|
| Verifier | 0.583 | schema, metric, freshness, causal review, poisoning |
| Stream/snapshot memory | 0.750 | late data, replay, integrated analysis |
| Semantic memory | 0.750 | metric drift, integrated analysis, poisoning |
| Schema/catalog memory | 0.833 | schema drift, integrated analysis |
| Provenance memory | 0.833 | replay, causal review |
| Feedback memory | 0.917 | feedback retention |
| Permission filter | 0.917 | permission conflict |

## 4. Indexed scale and concurrency

| Total memories | Serial runs with target at rank 1 | Serial p50 | Serial p95 | 8-reader p95 | Provenance edges | Provenance query p95 |
|---:|---:|---:|---:|---:|---:|---:|
| 10,001 | 30/30 | 0.045 s | 0.047 s | 0.411 s | 10,000 | 0.0009 s |
| 100,001 | 30/30 | 0.145 s | 0.152 s | 0.717 s | 100,000 | 0.0027 s |
| 1,000,001 | 20/20 | 4.997 s | 8.488 s | 4.548 s | 1,000,000 | 0.273 s |

All archived scales keep the FTS index synchronized, observe permission revocation and metric supersession, and complete mixed read/write probes without recorded errors.

## 5. Governed retrieval engines

All candidate generators use the same permission, temporal, authority, and supersession checks after retrieval.

| Distractors | BM25 top-1 | BM25 p95 | MiniLM/HNSW top-1 | HNSW p95 | Hybrid recall@5 | Hybrid p95 |
|---:|---:|---:|---:|---:|---:|---:|
| 1k | 0.667 | 0.267 s | 0.917 | 0.013 s | 0.917 | 0.283 s |
| 10k | 0.667 | 2.036 s | 0.708 | 0.016 s | 0.875 | 1.860 s |
| 100k | 0.667 | 17.326 s | 0.208 | 0.030 s | 0.833 | 16.746 s |

Restricted-item leaks: **0**. Superseded-item leaks: **0**.

## 6. Extended probes

| Probe | Result |
|---|---:|
| Invariant regression variants | 50/50 |
| Noisy retrieval variants | 5/5 |
| Seeded security probes | 6/6 |
| Claim-extraction corpus | 1,000 examples |
| Claim type precision / recall | 0.958 / 1.000 |
| Review-obligation recall | 1.000 |
| Verifier valid-query acceptance | 6/6 |
| Verifier invalid-query rejection | 10/10 |
| Corrected-verifier live end-to-end pilot | 3/3 |
| Full regression suite | 85/85 |

## 7. Integrity checks

```bash
cd artifacts/evaluation/retrieval_engine_comparison_1k && shasum -a 256 -c results.sha256
cd ../retrieval_engine_comparison_10k && shasum -a 256 -c results.sha256
cd ../retrieval_engine_comparison_100k && shasum -a 256 -c results.sha256
cd ../systems_scale_10k && shasum -a 256 -c results.sha256
cd ../systems_scale && shasum -a 256 -c results.sha256
cd ../systems_scale_1m && shasum -a 256 -c results.sha256
```

The preregistered external protocol is versioned separately at [`evaluation_protocols/pvldb_v20_preregistration.json`](../../evaluation_protocols/pvldb_v20_preregistration.json).
