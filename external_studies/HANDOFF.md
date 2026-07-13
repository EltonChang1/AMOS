# AMOS External Study Handoff

This packet is for collaborators who did not implement AMOS. It is deliberately separate from the current synthetic fixtures. Do not place real participant identities, secrets, proprietary product exports, or unredacted sensitive data in the public repository.

## Before Anyone Authors or Labels Data

1. Read `docs/evaluation/independent_evaluation_protocol.md` and `evaluation_protocols/pvldb_v20_preregistration.json`.
2. Verify `evaluation_protocols/pvldb_v20_preregistration.sha256`.
3. Archive the protocol JSON and sidecar in a timestamped, write-once location controlled by someone outside the implementation team.
4. Assign pseudonymous participant IDs and preserve the private identity key separately.
5. Confirm ethics, consent, privacy, compensation, data-use, and product-export permissions.
6. Freeze exact model IDs, provider parameters, baseline configurations, timeouts, retry budgets, and prices before any test labels or system outputs are opened.

## Required Independent Roles

| Role | Minimum | Independence rule |
| --- | ---: | --- |
| Task authors | 3 | Must not have implemented AMOS; each authors tasks in a domain they understand. |
| Task reviewers | 3 | Must differ from the author of every reviewed task. |
| Task annotators | 2 per task | Label independently before discussing disagreements. |
| Task adjudicator | 1 or more | Must not have supplied either original annotation for the task. |
| Claim annotators | 2 per artifact | Label exact spans independently. |
| Claim adjudicator | 1 or more | Must not be an original annotator for the artifact. |
| Red-team testers | 3 | Must not receive hidden expected attack outcomes from implementers. |
| Audit-study participants | 36 | Must be independent of implementation and technically qualified for the assigned review task. |

An individual may fill roles across different items only when the manifest validator permits it and the preregistered independence boundary remains intact. AMOS implementers may answer procedural questions but may not rewrite tasks, labels, rationales, or sealed references after seeing outputs.

## Task Author Package

For every task, provide:

- a realistic analyst request that is not written to reward AMOS terminology;
- a user identity and permissions;
- the exact available sources and data-state reference;
- a sealed reference answer, SQL, or method in a separate file;
- an expected outcome among `pass`, `warning`, `repair`, `reject`, or `needs_review`;
- required evidence, forbidden evidence, and review obligations;
- an author ID, different reviewer ID, two annotator IDs, and separate adjudicator ID;
- a source-group ID that never crosses development, validation, and test splits.

Use `templates/holdout_manifest.template.json` only as a field guide. Replace every placeholder and hash. The final test manifest must contain 50 non-ambiguous tasks in each of the three domains and cover all five outcome classes.

## Claim Corpus Package

- Preserve raw UTF-8 text after only documented redaction.
- Collect at least 120 source artifacts and 600 adjudicated claims.
- Include all seven artifact kinds accepted by the validator.
- Preserve exact character offsets and text for every label.
- Keep source and author groups entirely within one split.
- Preserve both original annotations and the adjudication rationale.
- Include numeric, causal, recommendation, context, forecast, and comparison claims, including hedging and fragments.

Use `templates/claim_manifest.template.json` only as a field guide.

## External Product Package

- Enable the product's native catalog, lineage, policy, memory, audit, citation, and replay features where available.
- Record the exact product/version/deployment/configuration, task, identity, permissions, retrieved context, SQL/tool calls, raw response, latency, tokens, cost, and failure status.
- Hash the configuration and each raw evidence file.
- Do not use `local_export_shaped_adapter` for the publication comparison.
- Do not import AMOS verifier or provenance components into the competing system.

Use `templates/external_product_manifest.template.json` only as a field guide.

## Validation Commands

```bash
shasum -a 256 -c evaluation_protocols/pvldb_v20_preregistration.sha256

python3 -m amos.evaluation.independent_task_evidence \
  evidence/holdout/manifest.json \
  --output artifacts/evaluation/independent_holdout_validation.json

python3 -m amos.evaluation.claim_annotation_evidence \
  evidence/claims/manifest.json \
  --output artifacts/evaluation/claim_corpus_validation.json

python3 -m amos.evaluation.external_product_evidence \
  evidence/products/manifest.json \
  --output artifacts/evaluation/external_product_study.json
```

Structural validation is not permission to run the held-out test. A designated study custodian should release test access only after configurations are frozen and development/validation work is complete.

## Required Return to the Implementation Team

- archived preregistration location and timestamp;
- pseudonymous role/independence roster;
- hash-verified manifests and sealed-reference files;
- annotation guidelines and pre-adjudication agreement data;
- exact provider/product configuration manifests;
- raw attempts, failures, outputs, costs, and hashes;
- documented deviations and exclusions;
- ethics/privacy approval or documented determination where applicable.

The implementation team may integrate these artifacts only after validators pass. Failed gates remain results and must not be repaired by editing the held-out evidence.
