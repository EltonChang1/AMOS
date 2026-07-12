# AMOS Artifact Evaluation Guide

This guide regenerates the evidence bundle used by the AMOS paper. The harness is offline-capable, isolates generated data from the source checkout, records failures per stage, and writes SHA-256 hashes for generated artifacts.

## Full reproduction

From the repository root:

```bash
python3 -m amos.evaluation.paper_bundle --all \
  --run-dir artifacts/evaluation/final_paper_run \
  --variants 12 \
  --samples 3 \
  --generated-tasks 120 \
  --variant-seed 20260711 \
  --scale-items 5000 \
  --claim-items 1000 \
  --concurrency 8 \
  --systems-scale-sizes 10000,100000,1000000 \
  --live-pilot-dir artifacts/evaluation/live_llm_pilot \
  --provider-mode auto
```

The command runs the unit tests, writes versioned independent-evidence schemas, loads all scenario fixtures, evaluates scenario-pack contracts and generated tasks, evaluates all product systems and ablations in all three domains, runs the deterministic benchmark and extended experiments, reruns the indexed 10k/100k/1m memory-and-provenance curve, and generates the consolidated paper report.

After updating the manuscript only from that frozen bundle, attach and hash the exact submitted files:

```bash
python3 -m amos.evaluation.finalize_submission \
  --run-dir artifacts/evaluation/final_paper_run \
  --tex AMOS_revised_professional.tex \
  --pdf AMOS_revised_professional.pdf \
  --supporting-doc PUBLICATION_READINESS_STATUS.md \
  --supporting-doc INDEPENDENT_EVALUATION_PROTOCOL.md \
  --supporting-doc EVIDENCE_INTAKE.md
```

`source_manifest.json` hashes evaluation code, tests, scenarios, and dependencies before execution. `submission/submission_manifest.json` separately hashes the post-result manuscript, PDF, and supporting documents, avoiding a circular source/manuscript hash.

`--provider-mode auto` uses the configured live provider only when credentials are present. Without credentials, the complete offline bundle is generated and every product result records `provider: offline`; this does not support a live-provider robustness claim. No secret values are written to environment metadata—only credential-presence booleans.

For a live experiment on a workstation authenticated through Codex, explicitly opt in to the read-only ephemeral CLI transport:

```bash
AMOS_LIVE_AGENT_PROVIDER=codex_cli \
python3 -m amos.evaluation.run_eval --extended \
  --scale-items 5000 --claim-items 1000 --concurrency 8 --llm-samples 24
```

The CLI transport runs outside the repository with `--ephemeral`, `--sandbox read-only`, `--ignore-user-config`, and `--ignore-rules`. It records the resolved model, final response, latency, and reported token total, but not authentication material. Do not label fewer than 24 policy prompts (three paraphrase rounds across eight categories) as a robustness study.

## Fast harness check

This checks orchestration and artifact contracts without producing paper-scale evidence:

```bash
python3 -m amos.evaluation.paper_bundle --all \
  --run-dir /tmp/amos_paper_bundle_check \
  --variants 1 --samples 1 \
  --generated-tasks 6 \
  --scale-items 10 --concurrency 2 \
  --benchmark-samples 1 \
  --provider-mode offline \
  --skip-tests
```

Fast-check numbers must not be copied into the paper.

## Required top-level evidence

Under `<run-dir>/artifacts/evaluation/` inspect:

| Artifact | Purpose |
| --- | --- |
| `bundle_manifest.json` | Stage status, duration, exact parameters, and overall pass/fail |
| `environment.json` | Python, platform, package versions, seeds, systems, and credential presence |
| `artifact_manifest.json` | Relative path, byte size, and SHA-256 for every generated artifact |
| `PAPER_RESULTS.md` | Consolidated tables, supported claims, and unsupported claim boundaries |
| `paper_artifact_index.json` | Mapping from report sections to deterministic source artifacts |
| `product_eval*/results.json` | Raw per-run records and aggregate statistics for each domain |
| `product_eval*/metric_axis_summary.csv` | Correctness, schema, permission, provenance, replay, and review axes |
| `product_eval*/system_contracts.json` | Exact access and guarantee contract for every compared system |
| `product_eval*/failure_modes.csv` | Failure counts by system and guarantee axis |
| `product_eval*/provenance_overhead.csv` | Matched provenance-on/off latency, tokens, evidence bytes, and replay time |
| `benchmark_suite.json` | Deterministic invariant benchmark results |
| `extended_experiments.json` | Retrieval, concurrency, security, and conditional provider experiments |
| `systems_scale*/results.json` | Indexed 10k/100k/1m memory, concurrency, update-visibility, and provenance-growth measurements |
| `systems_scale*/results.sha256` | Per-run integrity digest for the systems result |

Every product-evaluation row includes a `raw_path`. AMOS and the matched no-provenance ablation also include raw trace paths. Representative failures in `failures.md` link back to these records.

## Integrity and claim rules

1. Confirm `bundle_manifest.json` reports `status: pass`. A skipped unit-test stage is acceptable only for a fast harness check, never for a final bundle.
2. Confirm every stage is `pass`, and inspect `logs/pytest.log`.
3. Confirm `raw_evidence_existing == raw_evidence_count` and `raw_trace_existing == raw_trace_count` in `paper_artifact_index.json`.
4. Recompute SHA-256 hashes before archival if files have been moved or modified.
5. Cite full-contract pass rates separately from metric-axis correctness. Strong baselines can be analytically correct while lacking claim provenance and replay.
6. Do not cite offline-provider runs as live-provider results.
7. Preserve failures, malformed provider outputs, and retries; do not delete adverse raw records from a final bundle.
8. Verify each systems-scale digest from inside its directory with `shasum -a 256 -c results.sha256`; compare the 10k, 100k, and 1m runs without treating repeated operations as independent samples.

## Provider-backed extension

With `OPENAI_API_KEY` set, rerun the full command with `--provider-mode auto`. Archive the resulting bundle separately from offline results. Provider-backed evidence is incomplete until repeated trials cover all three domains, raw responses and failures exist, and provider/model identifiers are recorded. Population confidence intervals may be reported only over independently authored tasks or another justified statistical unit—not deterministic reruns of the same seeded variant.

## External-product evidence

Fixture-backed adapters must remain labeled as local export-shaped adapters. Real external runs should follow `INDEPENDENT_EVALUATION_PROTOCOL.md` and can be checked with:

```bash
python3 -m amos.evaluation.external_product_evidence path/to/manifest.json \
  --output artifacts/evaluation/external_product_study.json
```

The validator requires product/version/configuration identity, user permissions, raw evidence paths and SHA-256 hashes, task-level metric axes, and an external SaaS or self-hosted deployment mode. It rejects local fixture adapters when external deployment evidence is required.
