# AMOS solution-pack authoring and trust

Status: **signed contract and validation implemented; runtime routing and
activation persistence remain in progress**

A solution pack is the only supported home for workflow-specific definitions.
The AMOS core owns authorization, transaction state, execution, evidence,
review, replay, invalidation, and publication. A pack may narrow those services
for one approved workflow; it cannot grant a user permission, introduce a
write-capable source, bypass a connector, make a model authoritative, or remove
human review from the banking MVP.

The Rust contract is `amos.solution_pack.v1` in `src/solution_pack.rs`. Unknown
JSON fields are rejected. Activation validation checks the full contract before
signature verification so malformed definitions never become trusted merely
because they carry a valid signature.

## Contract sections

Every pack declares:

- stable pack and workflow IDs, semantic version, AMOS core version range,
  effective interval, status, risk class, tenant allowlist, and owner approval;
- accepted question families, required explicit parameters, and bounded
  recurrence/time-zone scheduling;
- required context roles, memory types, consistency classes, and minimum
  authority;
- read-only connector/source identity, relations, schemas, sensitivity, and
  source-version rules;
- metrics, formulas, units, populations, time grains, required filters,
  tolerances, owner approval, and optional limits, warnings, and materiality;
- optional bank metadata for institution boundary, legal entity, jurisdiction,
  hierarchy, currency, cutoff, horizon, scenario/model versions,
  reconciliation, and funding/collateral classes;
- allowed tools, plan bounds, read-only SQL templates, operation limits,
  verifier profiles, bounded repairs, and required context;
- claim schemas, evidence requirements, review obligations, and segregation of
  duties;
- deterministic answer/report/presentation/spreadsheet/data/evidence template
  identities;
- review-gated publication destinations and acknowledgments;
- retention, legal-hold, export, and deletion behavior; and
- frozen evaluation cases with explicit parameters, outputs, and terminal
  outcomes.

All question families and evaluation cases must explicitly bind
`window_start`, `window_end`, `comparison_start`, `comparison_end`,
`population`, `metric_id`, and `requested_outputs`. SQL templates may reference
only declared parameters using `{{parameter_id}}`. This syntax declares a bind;
runtime integration must still bind typed values through the connector driver
and must never interpolate untrusted text into SQL.

## Author, approve, and sign

Start from a reviewed JSON document whose `signatures` array is empty. The
owner approval reference must identify the customer or fixture change-control
decision; a synthetic label is not valid customer approval.

Generate and custody an Ed25519 signing key in the deployment's approved secret
manager or offline signing system. Never pass a private key as a command-line
argument, place it in a pack, trust store, shell history, environment variable,
container image, support bundle, model context, or source control.

The signing command reads exactly 32 raw key bytes encoded as 64 hexadecimal
characters from standard input and writes a signed copy:

```bash
approved-secret-command | amosctl solution-packs sign \
  --pack bank-weekly-liquidity.v1.unsigned.json \
  --output bank-weekly-liquidity.v1.json \
  --key-id customer-pack-publisher-2026
```

The command prints the public key, pack identity, and manifest hash. It never
prints the private key. AMOS signs the deterministic serialization of the typed
manifest, so all publishers must use the AMOS signing command for this schema
version rather than inventing a different JSON canonicalization scheme.

Create a separate trust store inside the customer's controlled configuration:

```json
{
  "schema_version": "amos.solution_pack.trust.v1",
  "publishers": [
    {
      "key_id": "customer-pack-publisher-2026",
      "public_key_hex": "64 hexadecimal public-key characters",
      "tenant_allowlist": ["customer_tenant_id"]
    }
  ]
}
```

The publisher key and the pack must independently authorize the target tenant.
An unknown key, wrong tenant, absent signature, modified manifest, unsupported
schema/core version, non-approved or ineffective pack, write-capable source,
ambiguous question family, unknown relation/metric/tool reference, missing
evidence rule, or unreviewed publication contract fails closed.

## Validate without activation

Validation is read-only and does not register a connector, change active
workflow state, or grant access:

```bash
amosctl solution-packs validate \
  --pack solution-packs/bank-weekly-liquidity.v1.json \
  --trust-store solution-packs/trust/development-fixtures.json \
  --tenant tenant_bank_fixture \
  --core-version 0.2.0
```

Multiple `--pack` arguments may be supplied when they target the same tenant.
The output records each verified manifest hash and trusted publisher key ID.

The checked-in development trust store and fixture publisher must never be
copied into staging, pilot, or production. They exist only to make signature,
tamper, compatibility, and tenant-boundary behavior reproducible in CI.

## Change, upgrade, and rollback rules

Treat every changed manifest as a new semantic version and new signature. Do
not edit a signed active version in place. Before promotion:

1. freeze the source mapping, metrics, limits, scenarios, evaluation cases,
   artifact templates, and expected outcomes;
2. run contract, signature, tool-catalog, verifier, golden-output, replay, and
   migration compatibility tests;
3. record the data/metric owner approval and independent review decision;
4. preserve the prior pack, trust decision, manifest hash, and activation audit
   record for replay; and
5. stage rollback by selecting the last compatible signed version, never by
   weakening validation or changing published history.

The current repository implements immutable signed documents, trust
verification, strict validation, and in-memory ambiguity protection. Durable
tenant activation, upgrade/rollback state, audit events, connector registration,
and execution routing are the next runtime slice and must not be represented as
complete yet.

## Banking fixture boundary

`solution-packs/bank-weekly-liquidity.v1.json` contains aggregate synthetic
definitions, synthetic thresholds, and explicit synthetic approvals. It moves no
money, initiates no funding or collateral action, changes no limit, makes no
customer or trading decision, files no report, and establishes no regulatory
compliance. A customer deployment must replace every fixture mapping,
definition, threshold, scenario, owner, applicability decision, and destination
through its own governed approval process.
