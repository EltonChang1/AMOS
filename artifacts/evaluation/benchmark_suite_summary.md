# AMOS Benchmark Suite Baseline

Generated: 2026-07-11

Command:

```bash
python3 -m amos.evaluation.run_eval --all --samples 3 --scale-items 5000 --run-dir /tmp/amos_eval_baseline
```

Raw output: `artifacts/evaluation/benchmark_suite.json`

## Aggregate Results

- AMOS: 12/12 tasks passed, pass rate 1.0.
- Catalog/lineage/dbt baseline: 6/12 tasks passed, pass rate 0.5.
- Metadata RAG access-control baseline: 3/12 tasks passed, pass rate 0.25.
- Semantic-layer baseline: 3/12 tasks passed, pass rate 0.25.
- Strong long-context baseline: 3/12 tasks passed, pass rate 0.25.
- Structured tool-LLM simulation: 1/12 tasks passed, pass rate 0.083.

## AMOS Runtime Measurements

- Samples: 3.
- Mean task overhead: 0.376 seconds.
- Min task overhead: 0.3002 seconds.
- Max task overhead: 0.4729 seconds.

## Scale Probe

- Memory distractors added: 5000.
- Seed time: 12.3331 seconds.
- Retrieval time at 5000 distractors: 0.8795 seconds.
- Target retrieved: true.
- Target rank: 1.

## Security Checks

- Permission filter: true.
- Prompt-injection-is-evidence-only check: true.
- Memory poisoning blocked: true.
