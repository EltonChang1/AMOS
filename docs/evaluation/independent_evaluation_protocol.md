# AMOS Independent Evaluation Protocol

## Purpose

This protocol defines the evidence required before AMOS results may be described as independently evaluated, statistically generalizable, or representative of deployed products. The current seeded benchmark does not meet those thresholds and remains engineering evidence.

## Preregistration Boundary

Before collecting holdout tasks:

1. freeze the AMOS implementation and record its source hash;
2. freeze all baseline configurations, prompts, tool permissions, and retry budgets;
3. publish the task schema and scoring rubric without publishing task answers;
4. declare primary and secondary metrics;
5. declare exclusion, retry, timeout, and malformed-output rules;
6. declare the statistical unit and planned tests;
7. archive the preregistration timestamp and hash.

No task-specific verifier, retrieval, prompt, or scoring changes may be made after holdout answers are revealed. Corrections discovered during evaluation must be reported and evaluated on a new holdout set.

## Independent Task Authors

- Recruit at least three analysts who did not implement AMOS.
- Each analyst should author tasks in a domain they understand.
- Authors receive the scenario data dictionary and business context, but not AMOS internals.
- At least one different analyst reviews each task for realism and ambiguity.
- AMOS implementers may reject tasks only for documented protocol violations before labels are revealed.

## Target Workload

- At least 50 independently authored tasks per domain.
- At least three domains with materially different schemas and failure modes.
- Include multi-turn tasks and tasks that require clarification or refusal.
- Include normal tasks, not only cases targeting AMOS invariants.
- Include schema evolution, metric changes, late data, permission conflicts, ambiguous definitions, stale analyses, feedback, and unsupported causal pressure.
- Include tasks for which the correct outcome is `pass`, `warning`, `repair`, `reject`, or `needs_review`.

## Task Record

Every task must contain:

```json
{
  "task_id": "stable identifier",
  "domain": "domain name",
  "request": "analyst request",
  "data_state": "snapshot or stream-state reference",
  "user_identity": "evaluation principal",
  "permissions": ["permission labels"],
  "available_sources": ["source identifiers"],
  "expected_outcome_class": "pass|warning|repair|reject|needs_review",
  "required_evidence": ["evidence categories"],
  "forbidden_evidence": ["restricted identifiers"],
  "reference_sql_or_method": "sealed reference",
  "claim_labels": ["sealed claim and review labels"],
  "author_id": "pseudonymous author",
  "reviewer_id": "pseudonymous reviewer"
}
```

## Annotation and Adjudication

- Two annotators independently label outcome class, required evidence, forbidden evidence, and claim-review obligations.
- Measure agreement before adjudication.
- Report Cohen's kappa for categorical labels and agreement/F1 for evidence sets.
- A third adjudicator resolves disagreements without seeing system identity or outputs.
- Preserve original labels, disagreement records, adjudicated labels, and adjudication rationale.
- Exclude irreducibly ambiguous tasks from primary scoring but report them separately.

## Systems and Fairness

All systems receive:

- identical task text and user identity;
- equivalent access to the same permissible data and metadata;
- the same model when isolating runtime architecture;
- the same tool timeout and retry budget;
- documented native integrations for semantic definitions, catalogs, policies, lineage, provenance, and replay.

Report both:

1. **matched-agent comparisons**, where the model and tools are fixed and only the runtime changes; and
2. **native-system comparisons**, where each external system uses its documented production configuration.

Do not score a system as analytically incorrect merely because it lacks an AMOS-specific feature. Report independent metric axes and a clearly labeled contract-coverage score.

## Required Baselines

- Tool-using LLM with current schema only.
- Permission-filtered RAG agent.
- Semantic-layer-backed agent.
- Catalog/lineage-backed agent.
- Long-context agent with all permissible context.
- AMOS with the same model and tools.
- AMOS component ablations.
- At least one deployed external product or production-faithful open-source stack.

## Live Models

- Evaluate at least three model families.
- Use at least three preregistered prompt paraphrases per task family.
- Repeat stochastic configurations sufficiently to estimate within-task variance.
- Preserve provider, model/version, temperature, seed where available, prompts, responses, tool calls, errors, latency, tokens, and monetary cost.
- Provider failures and rate limits are missing executions, not completed trials.

## Primary Metrics

- End-to-end task completion.
- Analytical correctness.
- Valid-query acceptance and invalid-query rejection.
- Permission safety.
- Unsupported-claim rate.
- Review-obligation recall and precision.
- Provenance coverage and provenance correctness.
- Replay success.
- Latency, token use, and cost.

Secondary metrics may include retrieval quality, repair success, clarification quality, reviewer audit time, storage growth, and attack success rate.

## Statistical Analysis

- The independent task is the primary statistical unit.
- Deterministic reruns of one task are not independent observations.
- Use paired comparisons because systems evaluate the same tasks.
- Report effect sizes and uncertainty intervals over independent tasks.
- Use hierarchical or mixed-effects analysis when repeated model samples are nested within tasks.
- Correct for multiple comparisons when testing many systems or axes.
- Publish per-task outcomes so aggregate conclusions can be audited.

## Verifier Study

Create a separately labeled SQL corpus containing valid and invalid queries across:

- aliases and derived columns;
- CTEs and nested subqueries;
- joins and correlated subqueries;
- window functions;
- grouping sets and conditional aggregates;
- dialect differences;
- stale and blocked columns;
- missing or malformed metric predicates;
- comments, strings, and obfuscation attacks;
- writes, DDL, export, and unsafe functions.

Report valid acceptance, invalid rejection, false positives, false negatives, repair success, and error category. The existing 16-case corpus is a development regression set and must not be reused as the independent test set.

## Claim-Extraction Study

- Use non-templated reports, notebooks, slides, chart annotations, and table cells.
- Redact sensitive information without normalizing language into templates.
- Double-annotate claim spans, types, evidence requirements, and review obligations.
- Split data by source artifact or author, not by sentence, to prevent leakage.
- Compare regex, prompted LLM, structured-output LLM, and supervised approaches.
- Evaluate both claim extraction and correctness of evidence links.

## Security Study

Define attacker capabilities, protected assets, policy assumptions, and success criteria. Include direct and indirect prompt injection, cross-user leakage, provenance expansion, malicious high-authority sources, permission revocation, multi-turn poisoning, and inference leakage. Report attack success together with normal-task utility.

## External Product Evidence

An external run is admissible only when it records:

- product name, version, deployment mode, and configuration hash;
- connector and model versions;
- task, identity, permissions, and available sources;
- raw product request/response or export;
- retrieved context identifiers;
- SQL/tool calls and execution results;
- native lineage/provenance/audit outputs;
- latency, tokens, and cost where available;
- SHA-256 hashes for all raw evidence.

Fixture-backed adapters must remain labeled `local_export_shaped_adapter` and may not be relabeled as external product runs.

## Release Requirements

The final submission artifact must contain:

- immutable source revision;
- preregistration hash;
- task and annotation protocol;
- anonymized holdout task release where permitted;
- raw outputs and failures;
- environment and dependency manifests;
- artifact hashes;
- scripts that regenerate every table and figure;
- explicit mapping from each paper claim to source artifacts.

## Repository Intake and Scoring Tools

The repository implements the collection boundary described above:

- `python3 -m amos.evaluation.independent_task_evidence <manifest>` validates frozen holdout tasks, participant independence, sealed-reference hashes, split leakage, original annotations, adjudication, and strict task/domain gates;
- the same command with `--predictions` scores task outcomes, evidence use, permissions, review obligations, provenance, replay, missing executions, latency, tokens, and cost over independent task units;
- `python3 -m amos.evaluation.claim_annotation_evidence <manifest>` validates non-synthetic raw claim artifacts, exact spans, double annotation, separate adjudication, hashes, source/author split isolation, agreement, and strict corpus-size/type gates;
- the same command with `--predictions` scores exact-span extraction, claim type, review obligation, and evidence-link correctness by source artifact;
- `python3 -m amos.evaluation.paired_task_analysis <first> <second>` verifies identical task IDs and reports paired task-resampled intervals and exact McNemar tests;
- `python3 -m amos.evaluation.external_product_evidence <manifest>` validates external deployment identity and raw evidence.

See `docs/evaluation/evidence_intake.md` for manifest fields, prediction shapes, commands, and archival order. Passing repository unit tests proves the intake contract, not the existence or independence of the required human evidence.

## Completion Gate

The paper may claim independent evaluation only after all of the following are true:

- independent task authors and annotators participated;
- holdout tasks were frozen before system tuning;
- at least three live models completed the study;
- verifier false-positive and false-negative rates were measured independently;
- at least one real external system was evaluated;
- population statistics use independent tasks as their unit;
- every headline number maps to immutable raw evidence.
