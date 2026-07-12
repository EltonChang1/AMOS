# Independent Evidence Intake

This guide converts externally collected tasks, annotations, product runs, and system outputs into auditable AMOS evaluation artifacts. The tools validate evidence; they do not manufacture independence or upgrade synthetic fixtures into human evidence.

## Evidence Classes

| Evidence | Validator or scorer | Strict publication gate |
| --- | --- | --- |
| Independent holdout tasks | `amos.evaluation.independent_task_evidence` | At least 3 domains, 50 non-ambiguous test tasks per domain, all five outcome classes, and at least 3 non-implementer authors |
| Real claim annotations | `amos.evaluation.claim_annotation_evidence` | At least 120 source artifacts, 600 adjudicated claims, all seven artifact kinds, and a held-out test split |
| Deployed product runs | `amos.evaluation.external_product_evidence` | External SaaS or self-hosted deployment, configuration identity, task-level axes, and hash-verified raw evidence |
| Paired system comparison | `amos.evaluation.paired_task_analysis` | Identical independent task IDs and split for both systems |

Structural admissibility and publication-scale completion are separate fields. A small pilot can be structurally valid while `completion_gate_met` remains false.

## Independent Holdout Tasks

The task manifest follows `IndependentTaskStudy` in `amos/evaluation/independent_task_evidence.py`. Each task includes the analyst request, identity and permissions, available sources, a hash-verified sealed reference, independent author/reviewer IDs, two original annotation sets, and a separate adjudication.

Validate the manifest:

```bash
python3 -m amos.evaluation.independent_task_evidence \
  evidence/holdout/manifest.json \
  --output artifacts/evaluation/independent_holdout_validation.json
```

The validator rejects:

- duplicate task IDs;
- implementer authors, reviewers, annotators, or adjudicators;
- author/reviewer identity reuse;
- fewer than two distinct annotators;
- an adjudicator who supplied an original annotation;
- missing, escaping, or hash-mismatched sealed references;
- source groups crossing development, validation, and test splits;
- labels revealed before the task freeze;
- missing preregistration, source-revision, or baseline-configuration hashes.

It reports pre-adjudication outcome Cohen's kappa and set F1 for required evidence, forbidden evidence, and review obligations. Irreducibly ambiguous tasks remain visible but are excluded from primary test scoring.

System predictions use this shape:

```json
{
  "system_id": "system-and-model-version",
  "predictions": [
    {
      "task_id": "task-001",
      "completed": true,
      "observed_outcome_class": "warning",
      "analytical_correctness": true,
      "evidence_used": ["metric-v3", "snapshot-42"],
      "review_obligations_marked": ["causal-claim"],
      "permission_safe": true,
      "unsupported_claim_count": 0,
      "provenance_correctness": 1.0,
      "replay_success": true,
      "latency_seconds": 2.4,
      "token_usage": 830,
      "cost_usd": 0.03
    }
  ]
}
```

Score a system without modifying the sealed task labels:

```bash
python3 -m amos.evaluation.independent_task_evidence \
  evidence/holdout/manifest.json \
  --predictions evidence/runs/amos-model-a.json \
  --split test \
  --output artifacts/evaluation/amos-model-a-task-scores.json
```

Provider errors and rate limits should be omitted from the prediction list. The scorer records them as missing executions and scores task-quality axes conservatively; they are not completed trials.

## Real Claim Annotation Corpus

The claim manifest follows `ClaimAnnotationStudy` in `amos/evaluation/claim_annotation_evidence.py`. Raw UTF-8 text exports remain unchanged except documented redaction. Every artifact has a source group, author group, split, raw SHA-256, two annotations, and separate adjudication.

```bash
python3 -m amos.evaluation.claim_annotation_evidence \
  evidence/claims/manifest.json \
  --output artifacts/evaluation/claim_corpus_validation.json
```

The validator checks exact character spans against raw text, hashes, annotator/adjudicator separation, non-synthetic provenance, and source/author split isolation. It reports exact-span F1, claim-type and review-obligation kappa on matched spans, and evidence-set F1 before adjudication.

Claim extractor predictions use exact character offsets:

```json
{
  "system_id": "structured-output-llm-v1",
  "predictions": [
    {
      "artifact_id": "artifact-001",
      "span_start": 0,
      "span_end": 27,
      "text": "Failure rate rose to 7.4%.",
      "claim_type": "numeric",
      "requires_review": false,
      "evidence_requirements": ["query", "metric", "data_state"]
    }
  ]
}
```

```bash
python3 -m amos.evaluation.claim_annotation_evidence \
  evidence/claims/manifest.json \
  --predictions evidence/claims/predictions/structured-llm.json \
  --split test \
  --output artifacts/evaluation/structured-llm-claim-scores.json
```

Run the same sealed test split for regex, prompted LLM, structured-output LLM, and supervised systems. Do not tune prompts or extraction rules after test labels are revealed. The seven required artifact kinds are report, notebook, slide, chart annotation, table cell, dashboard text, and fragment; forecast and comparison are supported claim types rather than artifact kinds.

## Paired Statistical Comparison

Compare two task-score files only when they contain exactly the same independent task IDs:

```bash
python3 -m amos.evaluation.paired_task_analysis \
  artifacts/evaluation/baseline-task-scores.json \
  artifacts/evaluation/amos-task-scores.json \
  --bootstrap-samples 10000 \
  --seed 20260712 \
  --output artifacts/evaluation/paired-baseline-vs-amos.json
```

Differences are reported as second system minus first system. The analysis includes task-resampled bootstrap intervals, paired standardized effects, and exact McNemar tests for binary axes. Multiplicity correction and hierarchical modeling for stochastic samples must follow the preregistration; this utility does not choose them after seeing results.

## External Product Runs

Use the separate deployment validator:

```bash
python3 -m amos.evaluation.external_product_evidence \
  evidence/products/manifest.json \
  --output artifacts/evaluation/external_product_study.json
```

`local_export_shaped_adapter` runs are rejected when external deployment evidence is required. Preserve native audit, lineage, retrieved-context, SQL/tool, response, cost, and failure records as raw hashed files.

## Archival Sequence

1. Freeze source and baseline configuration.
2. Archive and hash the preregistration.
3. Collect tasks or artifacts without AMOS implementer participation.
4. Preserve both original annotations before adjudication.
5. Validate manifests before system execution.
6. Run every system on the same sealed test IDs and preserve failures.
7. Score systems without changing the manifests.
8. Run paired analyses declared in the preregistration.
9. Add all manifests, raw evidence, scores, and hashes to the immutable release.
10. Update the manuscript only from archived result files.

Until the strict gates pass on actual external inputs, the manuscript must retain its current synthetic/offline evidence boundary.
