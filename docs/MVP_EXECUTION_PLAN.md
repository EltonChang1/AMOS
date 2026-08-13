# AMOS MVP execution plan

Status: **proposed execution baseline**

Planning horizon: **16 weeks to a validated MVP, followed by a 6–9 month
repeatability and expansion phase**

This document sequences delivery. `docs/PRODUCT_REQUIREMENTS.md` remains the
canonical product definition, `papers/AMOS_design_proposal.pdf` retains the
full technical architecture and control specifications, and
`docs/PRODUCT_READINESS.md` records implementation evidence. If the documents
conflict on product scope, update the canonical requirements and this plan in
the same reviewed change.

## 1. Objective and definition of complete

AMOS should not attempt to ship a smaller general-purpose Palantir platform as
its first product. The MVP is one complete, customer-deployed analytical
workflow:

> A business user asks a question or invokes a schedule, and AMOS uses one real
> enterprise data source to produce a verified answer, useful charts, an
> editable presentation, a report, and a data appendix. Every material claim is
> connected to its calculation and governing definitions, consequential claims
> remain review-gated, and approved work can be replayed or invalidated when an
> input changes.

The MVP is intentionally narrow:

- one paid design partner initially;
- one recurring executive, finance, revenue, risk, or operations review;
- one department and one reviewer role;
- one production read-only warehouse connector chosen from signed customer
  demand;
- one metric family containing approximately three to ten approved metrics;
- up to three approved query shapes;
- one mandatory chart and support for up to three charts in the deliverable;
- one company-branded report template and one presentation template;
- one approved publication or export destination;
- a local or customer-approved model behind a provider-neutral interface; and
- deployment inside the customer's trust boundary.

There are two completion gates:

1. **MVP release candidate:** a fresh customer-controlled environment can run
   the entire governed workflow and pass the product, security, reliability,
   packaging, and recovery gates in this document.
2. **MVP validated:** at least one paying design partner completes four weekly
   shadow cycles beside its current analyst process, the customer-agreed
   scorecard passes, and the customer makes an explicit renewal, production, or
   expansion decision.

Three design partners and two annual conversions are the next commercial
milestone, not a reason to delay learning from the first partner.

## 2. Required end-to-end user journey

The MVP must implement every step below as one supported workflow.

1. An administrator installs AMOS, connects identity, configures the model,
   registers the approved source, and installs the workflow solution pack.
2. An employee signs in through enterprise identity and asks an ad hoc business
   question, invokes an approved workflow, or schedules its recurrence.
3. AMOS resolves the applicable workflow definition, user permissions, metric
   definitions, schemas, source state, time range, review rules, and artifact
   templates.
4. AMOS filters context by tenant, identity, status, effective time, type, and
   sensitivity before ranking or sending anything to the model.
5. The analyst model proposes a typed plan and bounded tool calls. It receives
   no database credentials and cannot call company systems except through AMOS.
6. AMOS validates the plan against task, policy, schema, metric, time-window,
   relation, column, sensitivity, cost, row, byte, concurrency, and tool rules.
7. Isolated, capability-limited workers execute approved SQL, statistics, and
   chart computations against the production connector.
8. AMOS records the exact input versions, parameters, outputs, hashes, limits,
   latency, policy epoch, and execution identity for every step.
9. The model interprets only verified result objects and proposes a structured
   narrative, chart plan, report plan, and slide plan.
10. AMOS extracts or accepts typed material claims, independently recomputes
    authoritative numbers, validates chart bindings, and rejects unsupported
    prose.
11. Deterministic compilers create the direct answer, executive summary,
    tables, accessible charts, editable PPTX, HTML/PDF report, and XLSX/CSV data
    appendix using versioned company templates.
12. The result presents assumptions, limitations, freshness, unresolved
    questions, citations, sensitivity, intended audience, and required review
    decisions alongside the business narrative.
13. A reviewer inspects claim-level support and approves, rejects, or appends a
    bounded correction without mutating the original evidence.
14. AMOS revalidates policy and source state immediately before publication.
15. AMOS publishes or exports the approved package once, with an idempotent
    destination acknowledgment and recoverable retry behavior.
16. A later metric, schema, policy, source, document, or data-state change
    traverses dependencies and marks affected claims for revalidation,
    invalidation, or replay.
17. A follow-up question reuses the prior permitted context and evidence and
    does not silently change the metric, population, time range, or source
    version.
18. Operators can reconstruct the complete lifecycle, distinguish AMOS failure
    from source or model failure, recover interrupted work, and export the audit
    package.

## 3. Explicit non-goals for the MVP

The following work is deferred until the first workflow is validated:

- a general-purpose enterprise ontology builder;
- dozens of warehouse and business-application connectors;
- unrestricted natural-language access to all company data;
- arbitrary Python or notebook execution;
- general exploratory analysis and automatic causal discovery;
- arbitrary production writes or broad operational actions;
- unreviewed causal, regulated, external, or high-impact conclusions;
- general multi-agent scheduling;
- automatic external communication outside the configured destination;
- multi-region SaaS, international expansion, or broad compliance claims;
- microservice decomposition without measured isolation or scaling pressure;
- Kubernetes as a prerequisite for the first deployment; and
- replacing the customer's warehouse, identity provider, catalog, semantic
  layer, lineage system, or existing BI platform.

## 4. Program principles

1. **Customer value before platform breadth.** The critical path begins with a
   real recurring workflow and ends with measured customer results.
2. **One vertical slice.** Model reasoning, deterministic execution, evidence,
   artifacts, review, publication, and change handling ship together.
3. **Models are replaceable and untrusted.** Models propose; AMOS authorizes,
   executes, calculates, verifies, records, and publishes.
4. **Source systems remain authoritative.** AMOS carries source-native identity
   and restrictions and adds a second fail-closed control layer.
   Customer data, prompts, results, and telemetry do not leave the authorized
   deployment boundary unless an administrator explicitly configures and
   authorizes the destination.
5. **Configuration before copy-and-paste.** Customer-specific behavior lives in
   versioned solution packs rather than branches or hard-coded runtime logic.
6. **Modular monolith first.** Keep one control-plane application and one
   authoritative transaction database while separating model serving and
   isolated execution workers where their operational boundaries require it.
7. **Every durable effect is idempotent.** API retries, jobs, source events,
   reviews, rendering, and publication cannot create duplicate effects.
8. **Evidence is a product surface.** A correct calculation that a reviewer
   cannot inspect quickly is not complete.
9. **No production-readiness claims from local adapters.** Real infrastructure
   and customer systems require their own conformance evidence.
10. **Every deployment must make the next one easier.** Repeated work becomes a
    connector certification, solution-pack component, template, test, runbook,
    or deployment automation.

## 5. Team ownership and operating cadence

The plan assumes the four current technical founders. Assign named people to
the roles below before starting; one person can own two roles, but no item can
be ownerless.

| Role | Primary ownership | Required operational ownership |
| --- | --- | --- |
| Product and design-partner lead | Workflow discovery, pilot scope, scorecard, user experience, executive readout, commercial decision | Customer communication and scope control |
| Data and connector lead | Production connector, semantic/schema ingestion, solution-pack data mapping, source conformance | Connector incidents, credentials, quotas, and source changes |
| Agent and runtime lead | `ModelProvider`, planning/interpretation loop, task routing, A-TXN orchestration, verification | Model/runtime failures, task recovery, and evaluation regressions |
| Artifacts and platform lead | PPTX/PDF/HTML/XLSX compilers, application surfaces, identity, packaging, deployment automation | Release, backup/restore, observability, and publication incidents |
| Security owner | Threat model, secrets, worker isolation, dependency response, security gates | Incident commander and vulnerability disclosure; may be one of the four roles above |

Operating cadence:

- daily 15-minute critical-path review;
- weekly design-partner session after a partner is selected;
- weekly product scorecard and risk-register review;
- one architecture decision record for every change to a trust boundary,
  durable contract, deployment model, or MVP scope;
- release-candidate demonstration every two weeks using a clean environment;
- no feature enters the MVP without an owner, acceptance test, metric, failure
  behavior, and operator status; and
- no phase closes with an unresolved critical security, permission, data-loss,
  or duplicate-publication defect.

## 6. Critical path

```mermaid
flowchart LR
    A["Paid design partner"] --> B["Frozen workflow and scorecard"]
    B --> C["Versioned solution pack"]
    B --> D["Production connector"]
    C --> E["Model plan and interpretation loop"]
    D --> E
    E --> F["Verified result IR"]
    F --> G["PPTX report and data appendix"]
    G --> H["Review schedule and publication"]
    H --> I["Customer-contained deployment"]
    I --> J["Security reliability and recovery qualification"]
    J --> K["Four weekly shadow cycles"]
    K --> L["Executive readout and commercial decision"]
```

Identity, PostgreSQL, worker isolation, packaging, observability, and customer
discovery run in parallel, but they must converge before customer data enters
the release-candidate environment.

## 7. Phase-by-phase execution plan

### Delivery schedule at a glance

The schedule assumes a paid partner and approved non-production access are
available by the end of week 4. Reusable engineering can continue without
those inputs, but customer-specific connector, identity, template, deployment,
and publication work must remain uncommitted. If no partner is signed, the
customer-validation clock pauses rather than quietly substituting an internal
fixture for customer proof.

| Phase | Target window | Principal dependency | Milestone/exit |
| --- | --- | --- | --- |
| 0. Program reset | Days 1–5 | Plan approval | Repository truth and owned backlog |
| 1. Paid pilot contract | Weeks 1–4 | Qualified customer access | Paid partner, workflow, source, scorecard, decision date |
| 2. Solution-pack contract | Weeks 2–5 | Frozen core contracts | Two packs execute without core specialization |
| 3. Production connector | Weeks 3–7 | Partner source choice and access | Real-service connector certification |
| 4. Analyst model loop | Weeks 4–8 | Pack schema and frozen pilot tasks | Evaluated plan/execute/interpret loop |
| 5. Artifact product | Weeks 5–9 | Verified-result IR | Editable decision package with complete evidence |
| 6. Application workflows | Weeks 6–10 | Async runtime and artifact IR | Analyst, reviewer, and administrator can operate unaided |
| 7. Production foundation | Weeks 5–10 | Customer identity/infrastructure choice | PostgreSQL, identity, secrets, queue, and isolated workers |
| 8. Packaging | Weeks 7–11 | Production topology | Signed installable and upgradable release |
| 9. Qualification | Weeks 9–12 | Integrated release candidate | Security, recovery, performance, and support gates pass |
| 10. Shadow pilot | Weeks 11–15 | Qualified customer deployment | Four consecutive scored weekly cycles |
| 11. Readout | Week 16 | Complete pilot evidence | Explicit production, renewal, expansion, extension, or no-go decision |

Program milestones:

- **M0 — Execution baseline:** Phase 0 passes.
- **M1 — Demand gate:** a paid partner and frozen workflow exist.
- **M2 — Generalization gate:** the core runs two packs and a real connector
  without payment-specific logic or data-access bypasses.
- **M3 — Product gate:** a model-generated plan becomes a verified, editable,
  reviewable decision package.
- **M4 — Pilot release gate:** deployment, security, operations, packaging, and
  recovery qualification pass in a clean environment.
- **M5 — Validated MVP:** four customer shadow cycles and the commercial
  decision are complete.

### Phase 0 — Program reset and repository truth (days 1–5)

Purpose: make the repository a reliable execution baseline rather than a mix of
current product code and historical research claims.

Tasks:

- approve this plan and assign a named owner to every workstream;
- create one issue or tracked work item for every checklist item in this plan;
- freeze the current Rust control contracts and preserve the passing regression
  suite;
- add continuous integration for formatting, clippy with warnings denied,
  debug and release tests, optimized build, documentation, dependency audit,
  and the bounded benchmark;
- add the intended root license after confirming contributor/IP ownership;
- add `SECURITY.md`, a private reporting channel, supported-version policy,
  severity definitions, response targets, and customer-notification policy;
- label non-executable Python-era scenarios and evaluation commands as archived
  evidence, or restore the missing evaluators and make CI prove they execute;
- separate research results from current product-readiness claims;
- create architecture decision records for the modular monolith, PostgreSQL,
  connector-mediated execution, model-provider boundary, artifact intermediate
  representation, worker isolation, and deployment packaging;
- define development, staging, pilot, and production configuration profiles;
- establish a prioritized risk register covering customer demand, model quality,
  metric correctness, connector behavior, permissions, deployment, support, and
  consulting creep; and
- retain the payment workflow only as a regression fixture, not as the product
  definition.

Exit gate:

- the main branch is green;
- current versus historical evidence is unambiguous;
- every MVP work item has an owner, dependency, and acceptance condition; and
- there is one ordered product backlog tied to this plan.

### Phase 1 — Customer discovery and paid pilot contract (weeks 1–4)

This phase is commercial and technical. It runs in parallel with Phase 0 and
the reusable engineering work, but the selected connector and workflow must
not be guessed before customer evidence exists.

Tasks:

1. Build a list of at least 30 qualified Heads/VPs of Data, Analytics, AI
   Platform, Finance, Revenue Operations, Risk, or Operations.
2. Conduct problem interviews using a concrete recurring-report prompt rather
   than an “AI governance platform” pitch.
3. Observe at least ten real recurring review workflows end to end: request,
   sources, definitions, calculations, charts, narrative, review, delivery,
   corrections, and later updates.
4. Capture for each workflow:
   - business decision and audience;
   - recurrence and current service level;
   - analyst and reviewer time;
   - approved metrics and query patterns;
   - data sources, warehouse, semantic layer, catalog, and identity provider;
   - sensitivity and prohibited data;
   - existing report, slide, and spreadsheet examples;
   - known freshness, schema, and definition failures;
   - publication destination;
   - buyer, technical champion, data owner, reviewer, security stakeholder, and
     procurement path; and
   - value, urgency, budget, decision date, and disqualifying constraints.
5. Score candidates. Prefer a partner that has a warehouse, approved metrics,
   a weekly workflow, a read-only test environment, a named data owner and
   reviewer, a measurable baseline, weekly access to the team, and a short
   procurement path.
6. Select the workflow based on urgency and reuse potential, not on the current
   payment fixture or general warehouse market share.
7. Agree to a fixed-scope paid shadow pilot. A price-discovery range of
   $25,000–$75,000 may be tested, but scope and a decision date matter more than
   the exact first price.
8. Put the following in the pilot statement of work:
   - one source and connector;
   - three to ten metrics and up to three query shapes;
   - one report and presentation template;
   - one reviewer role and one publication destination;
   - read-only operation and prohibited data classes;
   - baseline collection and four weekly shadow cycles;
   - customer responsibilities for access, definitions, review, and feedback;
   - deployment boundary, data handling, retention, deletion, and support;
   - success metrics, acceptable thresholds, decision date, and production or
     renewal option; and
   - separately priced work for material scope changes.
9. Perform a data-flow and security review before receiving credentials or
   customer data.

Exit gate:

- one paid design partner, one named workflow, one real source, one reviewer,
  one data owner, one agreed scorecard, and one commercial decision date;
- a sanitized fixture and sealed reference outputs can be built from the real
  workflow; and
- if 30 qualified interviews produce no workflow access or willingness to pay,
  pause broad engineering and narrow, reposition, integrate into an incumbent,
  or stop.

### Phase 2 — Domain-neutral solution-pack contract (weeks 2–5)

Purpose: remove payment behavior from the core and make customer specialization
installable and versioned.

Tasks:

- define a signed/versioned solution-pack manifest covering:
  - workflow ID, version, effective dates, owner, risk, and status;
  - accepted question families and schedule parameters;
  - required context roles and consistency classes;
  - sources, relations, schemas, sensitivity, and source-version rules;
  - metric definitions, required filters, units, populations, and time grains;
  - allowed plan steps, tools, query shapes, repair classes, and limits;
  - verifier rules and review obligations;
  - claim schemas and evidence requirements;
  - chart, report, presentation, and spreadsheet templates;
  - publication rules and destinations;
  - retention, legal hold, and deletion behavior; and
  - evaluation cases and expected terminal outcomes;
- replace `payment_health_review` selection with explicit workflow routing;
- replace fixed table names, dates, SQL, verifier profiles, claims, chart labels,
  report prose, and replay-template IDs with solution-pack configuration;
- accept and validate explicit time range, comparison period, population,
  metric, and requested output parameters;
- make runtime connector registration tenant- and solution-pack-specific;
- make SQL execution consume the approved connector/driver path instead of
  independently opening the demo database;
- validate packs before activation and require owner approval for governing
  definitions;
- add pack upgrade, rollback, compatibility, and audit behavior;
- convert the payment fixture into a first pack;
- build a second non-payment fixture from configuration without changing core
  runtime code; and
- publish a solution-pack authoring guide and validation command.

Exit gate:

- the payment regression passes through a versioned pack;
- a second workflow pack runs without core-code conditionals;
- the core contains no payment table, metric, date, narrative, or artifact
  assumptions; and
- invalid, unsigned, incompatible, ambiguous, or unauthorized packs fail
  closed.

### Phase 3 — One certified production connector (weeks 3–7)

Purpose: connect to the first customer's actual source without bypassing the
AMOS connector contract or source-native controls.

Tasks:

- implement the connector selected by the paid partner;
- support discovery, observation, bounded reads, revalidation, change events,
  cursor recovery, and health;
- use the customer's source identity or an approved impersonation/delegation
  method where possible;
- preserve source-native row, column, role, classification, and query controls;
- implement credential acquisition and rotation through the selected secret
  manager rather than files or environment dumps;
- establish the private networking or customer-local path to the source;
- implement prepared, query-only execution, cancellation, timeouts, row/byte
  limits, concurrency limits, cost controls, and query tagging;
- record source version, schema version, freshness/watermark, query ID, result
  hash, bytes, rows, latency, and classified failure status;
- map dbt, semantic-layer, or customer metric definitions if they are part of
  the selected workflow;
- map catalog/lineage metadata only when the customer already operates those
  systems; do not build replacements;
- implement retry and backoff without retrying unsafe or non-idempotent work;
- make revocation or source permission changes visible before execution and
  before publication;
- implement source deletion and schema/metric change events for invalidation;
- add structured, redacted connector logs and customer-visible health;
- run a conformance suite for authentication, discovery, pagination,
  permissions, schema drift, freshness, rate limits, quotas, timeout, outage,
  restart, credential rotation, revocation, duplicate events, and deletion; and
- document supported versions, privileges, network paths, quotas, failure
  modes, and operator runbooks.

Exit gate:

- every source read travels through the connector-mediated, capability-checked
  path;
- the connector conformance suite passes against a real non-production service;
- no ambient warehouse credential reaches the API, model, artifact, log, or
  unisolated worker; and
- revocation and schema changes block or invalidate work as designed.

### Phase 4 — Real analyst model loop (weeks 4–8)

Purpose: replace the deterministic hard-coded planner with a bounded,
evaluated, provider-neutral analyst.

Tasks:

- define a `ModelProvider` port supporting health, structured generation,
  cancellation, timeout, model metadata, and usage records;
- implement one local or customer-approved model adapter. Gemma 4 is the
  preferred local starting family, but customer requirements and frozen task
  evaluation determine the release model;
- keep model weights and inference serving as separately installable
  components;
- define hardware profiles for development, lower-resource, and standard
  server inference;
- record provider, exact model, weight and quantization identity, serving
  version, prompt-template version, input-manifest hash, output hash, latency,
  token counts, decoding settings, retries, and failure status;
- build a typed planning schema containing purpose, inputs, source, tool,
  parameters, expected output, limits, verifier profile, and repair classes;
- validate all model output structurally and semantically before execution;
- reject, request bounded repair, or require review when a plan is invalid;
- send only permission-filtered and purpose-limited context to the model;
- ensure prompts and retrieved documents are treated as data with explicit
  provenance, not as authority to change policy or tools;
- perform authoritative computation only in deterministic workers;
- return only verified result objects to the interpretation pass;
- define structured narrative, claim, chart, report, and slide-plan outputs;
- independently verify all material claims before rendering;
- implement a bounded plan/execute/interpret loop with maximum steps, repairs,
  tokens, time, and cost;
- implement graceful model outage, malformed output, timeout, and cancellation
  behavior;
- implement follow-up questions as child tasks bound to the prior task's
  permitted context, definitions, time range, population, and evidence;
- add model and prompt evaluations for correctness, policy adherence,
  unsupported claims, prompt injection, ambiguity, repair behavior, and
  variance across repeated runs; and
- prove that a second provider can be substituted without changing memory,
  transaction, connector, evidence, or artifact contracts.

Exit gate:

- at least 90% safe-and-correct completion on the frozen pilot task set;
- zero execution of seeded critical policy, destructive-query, blocked-column,
  or permission violations;
- every authoritative number is independently recomputed outside the model;
- invalid model outputs fail visibly and closed; and
- model or provider replacement requires adapter/configuration work, not a core
  contract rewrite.

### Phase 5 — Verified result IR and deterministic artifacts (weeks 5–9)

Purpose: deliver the work product the customer buys, not merely runtime JSON.

Tasks:

- define a versioned verified-result intermediate representation containing:
  - executive answer, findings, claims, limitations, and open questions;
  - typed tables, values, units, populations, time ranges, and formats;
  - chart specifications bound to exact verified result sets;
  - citations to metric, schema, source state, execution, verification, and
    review records;
  - narrative, slide, and appendix plans;
  - audience, sensitivity, retention, and publication metadata; and
  - deterministic template and rendering configuration IDs;
- make the model produce a plan for this IR rather than final authoritative
  files or numbers;
- implement deterministic accessible chart rendering with titles, labels,
  units, legends, alt text, and stable data bindings;
- implement an editable PPTX compiler with company theme, titles, graphs,
  tables, conclusions, citations, and speaker notes;
- implement an HTML report and deterministic PDF rendering path;
- implement XLSX and CSV export with values, units, filters, time ranges, and
  source references;
- add a machine-readable JSON export for integrations and independent checking;
- preserve editable charts/tables where the target format permits it;
- include claim-level evidence links in every supported artifact;
- render sensitivity and intended audience in the package;
- hash every input, template, generated file, and package manifest;
- support regeneration from unchanged verified results without rerunning SQL;
- reject any narrative claim that is absent from the verified IR;
- compare rendered chart values and slide numbers back to the IR;
- add golden-file and visual regression tests for the selected customer
  templates; and
- provide one downloadable decision package containing PPTX, PDF/HTML,
  XLSX/CSV, JSON, and evidence manifest.

Exit gate:

- the customer's reference report and deck can be reproduced in an editable,
  recognizable form;
- 100% of displayed material numbers match the verified IR;
- 100% of charts and slide conclusions link to current evidence;
- regeneration is deterministic within the declared renderer contract; and
- a reviewer can locate the support for a material number or claim without
  engineering assistance.

### Phase 6 — Product workflows and application surfaces (weeks 6–10)

Purpose: turn the control endpoints into a coherent analyst, reviewer, and
administrator product.

Tasks:

- add enterprise sign-in and a real browser session flow;
- build an analyst workspace for questions, workflow selection, parameters,
  schedules, progress, results, follow-ups, downloads, and task history;
- make task admission asynchronous: return a task ID, expose durable progress,
  and allow safe cancellation rather than holding one HTTP request open;
- add a schedule editor with time zone, recurrence, owner, output, and
  notification configuration;
- show selected definitions, freshness, limitations, assumptions, and
  permission-visible evidence in a usable evidence drawer;
- upgrade the review queue to support claim selection, evidence inspection,
  diffing, correction, explicit confirmation, and publication state;
- show invalidation and replay differences at the claim and artifact level;
- build an administration surface for identity/group mapping, sources,
  credentials, network rules, model profile, solution packs, resource limits,
  review/publication rules, retention, schedules, and notification destinations;
- build an operations surface for task health, model/source status, queues,
  jobs, outbox, audits, invalidations, publications, usage, latency, and support
  bundle generation;
- implement accessible, responsive loading, empty, denied, partial, warning,
  retry, and failure states;
- keep restricted evidence, object existence, logs, and operational metrics
  invisible to unauthorized users; and
- instrument the complete user journey for the MVP scorecard without placing
  customer data in product telemetry.

Exit gate:

- an analyst can run and follow up on the workflow without a CLI;
- a reviewer can resolve an artifact without database or engineering access;
- an administrator can configure the pilot without source-code changes; and
- every long-running action survives browser disconnect and process restart.

### Phase 7 — Production state, identity, secrets, and isolated execution (weeks 5–10)

Purpose: replace local proof adapters with a defensible pilot deployment.

Tasks:

#### State and queues

- port the authoritative control store from SQLite to PostgreSQL while keeping
  SQLite only for local development and deterministic tests;
- add a bounded connection pool, online forward migrations, composite tenant
  ownership, forced row-level security, and database-role separation;
- configure encryption, high availability, point-in-time recovery, backup
  retention, and restore automation;
- move large artifact bodies to versioned object storage while retaining hashes
  and metadata in PostgreSQL;
- introduce a durable queue for asynchronous tasks, scheduled work, replay,
  invalidation, rendering, publication, and outbox dispatch;
- separate interactive and scheduled workloads into independent queues;
- preserve database-owned CAS transitions, fences, leases, checkpoints, and
  post-commit queue acknowledgment; and
- add controller election or single-active-controller behavior appropriate to
  the deployment.

#### Identity and authorization

- integrate the customer's OIDC provider first; add SAML only if required;
- validate issuer, audience, signature, time, nonce/state, and JWKS rotation;
- map users, groups, tenant, roles, source permissions, policy attributes, and
  policy epoch;
- implement secure cookies, session expiry/revocation, CSRF controls, logout,
  and administrative break-glass auditing;
- propagate source-native identity or approved delegation to the connector; and
- verify tenant, permission, row/column, sensitivity, task, review, and
  publication policy at retrieval, execution, commit, read, and publication.

#### Secrets and execution isolation

- store connector, model, signing, and publication secrets in the selected
  secret manager;
- use KMS/HSM-backed signing or envelope encryption where available;
- implement key versioning, rotation, revocation, least-privilege access,
  audit, and recovery;
- run SQL/statistics/chart/render workers in isolated containers or VMs with a
  separate workload identity;
- issue short-lived, audience-bound, single-task capabilities to workers;
- mount no ambient credentials and apply a default-deny egress allowlist;
- set CPU, memory, time, process, filesystem, row, byte, concurrency, and cost
  limits;
- scan worker images and dependencies and run them read-only where practical;
- terminate compromised or runaway workers without corrupting transaction
  history; and
- keep the model server isolated from warehouse credentials and direct source
  routes.

Exit gate:

- cross-tenant and cross-role tests fail closed at API, repository, RLS,
  connector, cache, object path, capability, worker, and artifact layers;
- database, objects, queue state, keys, and connector configuration can be
  restored together within the declared pilot RPO/RTO;
- worker compromise exercises expose no long-lived credentials; and
- a policy or identity revocation before publication prevents publication.

### Phase 8 — Executable packaging and release engineering (weeks 7–11)

Purpose: create a supported enterprise product distribution. The existing Rust
binary remains a useful component but is not sufficient by itself.

Tasks:

- retain the `amos` server/runtime binary and add production configuration
  rather than weakening its current fail-closed demo behavior;
- create an `amosctl` executable supporting:
  - preflight and hardware checks;
  - install and bootstrap;
  - configuration validation;
  - source, identity, model, and destination tests;
  - database migration and status;
  - health and diagnostics;
  - support-bundle export with redaction;
  - backup and restore;
  - upgrade, rollback, and release-channel selection;
  - tenant audit/data export; and
  - uninstall/offboarding and deletion verification;
- publish a signed Linux OCI image or VM appliance as the primary production
  distribution;
- provide a simple container-compose deployment for the paid pilot;
- add a Kubernetes package only when a customer environment or measured scale
  requires it;
- keep model weights and inference runtime in a separate, licensed model
  package with explicit hardware profiles;
- produce a signed offline installation bundle with mirrored dependencies and
  documentation for disconnected deployments, at minimum validating that the
  architecture can support it; make full air-gap qualification conditional on
  customer need;
- build reproducible versioned releases for supported Linux architectures;
- generate and publish an SBOM, dependency and container vulnerability scan,
  checksums, signatures, provenance/attestation, licenses, notices, and
  compatibility manifest;
- define stable, candidate, and recalled release channels;
- test upgrade, failed upgrade, rollback, schema compatibility, and model/pack
  compatibility;
- support development, staging, pilot, and production configurations without
  rebuilding binaries;
- document ports, storage, compute/GPU, network, DNS/TLS, egress, identity,
  backup, log, and monitoring requirements; and
- run the installation guide from a clean customer-like VM with no developer
  workstation dependencies.

Exit gate:

- a new environment can be installed, configured, diagnosed, upgraded,
  rolled back, backed up, restored, and exported through supported commands;
- the release is signed, scanned, reproducible, versioned, and accompanied by
  its SBOM and notices;
- no model weights are embedded in the Rust executable; and
- target customer onboarding completes in fewer than ten business days.

### Phase 9 — Observability, reliability, support, and security qualification (weeks 9–12)

Purpose: prove the whole product boundary under failure, not only the happy
path.

Tasks:

#### Observability and service objectives

- propagate request, trace, tenant, task/A-TXN, plan, execution, job, artifact,
  review, publication, and source IDs;
- export metrics, structured redacted logs, distributed traces, and immutable
  audit events to the selected backend;
- monitor task outcomes, context coverage, verifier rules, repair counts,
  capability denials, query cost, model latency/tokens/cost, source health,
  queue age, review backlog, invalidation lag/fan-out, publication
  acknowledgment, replay success, storage growth, and customer scorecard
  metrics;
- distinguish AMOS, model, source, identity, destination, and operator failure;
- implement dashboards and alerts for paid-pilot objectives:
  - 99.5% API availability;
  - 99% successful scheduled runs excluding declared source outages;
  - p95 task admission/context compilation below two seconds at the declared
    scale;
  - zero confirmed cross-tenant or permission leaks; and
  - 100% of final artifacts with complete claim/evidence manifests.

#### Failure and recovery

- test process and worker death at every durable boundary;
- test model outage, source timeout, identity outage, queue backlog, database
  failover, object mismatch, duplicate events, stale leases, invalidation
  storms, replay differences, and lost publication acknowledgments;
- prove no duplicate publication across retries or acknowledgment loss;
- set the paid-pilot control-plane targets to an RPO of five minutes and RTO of
  one hour unless the contract specifies stricter values;
- restore database, objects, queues, keys, configuration, and connector state
  together in staging; and
- document customer-facing degradation and recovery behavior.

#### Runbooks and support

- write and rehearse runbooks for source authentication failure, permission
  leak suspicion, policy mismatch, stuck transaction, worker compromise,
  runaway query, queue backlog, database failover, object mismatch,
  invalidation storm, replay mismatch, model outage, publication failure,
  retention/legal hold, and erasure;
- define support channel, hours, severity, response targets, escalation,
  evidence preservation, incident communication, and postmortem process;
- define customer admin, AMOS, cloud/infrastructure, model, and source-system
  responsibilities; and
- publish a truthful security and architecture page listing implemented and
  unimplemented controls without claiming SOC 2, ISO, legal, or regulatory
  compliance.

#### Qualification suites

- run unit/property tests for state, policy, time, authority, supersession,
  hashing, CAS, fencing, and tenant ownership;
- run connector conformance against the real service;
- run context tests under permissions, ambiguity, conflicts, budget pressure,
  supersession, and policy churn;
- run verifier corpora for valid/invalid SQL, metrics, freshness, schema drift,
  numeric claims, chart binding, and unsupported narrative;
- run end-to-end tests for request, schedule, review, correction, publication,
  feedback reuse, follow-up, replay, invalidation, retry, and export;
- run security tests for cross-tenant access, prompt injection, memory
  poisoning, capability replay, secret leakage, provenance inference, SSRF,
  unsafe files, and denial of service;
- run performance tests at declared pilot capacity, not aspirational scale;
- measure p50/p95/p99, peak RSS, pool/lock wait, queue age, rows/bytes,
  concurrency, model usage, and invalidation fan-out; and
- conduct a human-review usability study in which qualified reviewers locate
  support, identify seeded defects, and submit corrections without engineering
  help.

Exit gate:

- no open critical security or data-integrity finding;
- all failure paths preserve an inspectable, idempotently recoverable audit
  trail;
- the release SLOs pass at declared pilot capacity with margin;
- the restore drill and lost-acknowledgment drill pass; and
- a reviewer can complete the supported workflow unaided.

### Phase 10 — Customer onboarding and four-cycle shadow pilot (weeks 11–15)

Purpose: validate the product against the customer's current work without
prematurely replacing it.

Tasks:

1. Deploy the signed release into the agreed customer-controlled environment.
2. Complete identity, network, source, model, metric, policy, template, review,
   publication, retention, monitoring, backup, and support configuration.
3. Confirm prohibited sources, columns, identities, destinations, and actions.
4. Import only approved definitions and sanitized template assets.
5. Freeze a representative task set and sealed reference calculations with the
   customer's analysts.
6. Record the current analyst process baseline before AMOS results are shown:
   analyst minutes, reviewer minutes, time to deliverable, manual repairs,
   recurring failures, and current audit/reconstruction effort.
7. Rehearse request, schedule, cancellation, review, correction, publication,
   replay, invalidation, source outage, model outage, and recovery.
8. Run AMOS beside the existing analyst for four weekly cycles. Do not replace
   the official deliverable during shadow mode.
9. For every cycle:
   - execute the scheduled and agreed ad hoc tasks;
   - preserve raw attempts and failures;
   - compare material numbers, methods, charts, claims, limitations, and files
     with the sealed reference and analyst output;
   - have the named reviewer use the product UI;
   - record review time, corrections, unsupported claims, caught errors,
     escaped errors, replay, and publication behavior;
   - classify product, model, source, definition, or operator causes; and
   - turn every reproducible defect into a regression test or solution-pack
     validation rule.
10. Hold a weekly customer session covering results, defects, support burden,
    requested scope, and decision risks.
11. Refuse unplanned arbitrary Python, database writes, new departments, and
    extra connectors unless they replace lower-priority scope through a signed
    change.
12. Maintain a deployment journal separating reusable product work from
    customer-specific services.

Exit gate:

- four consecutive weekly shadow cycles complete;
- the agreed correctness, security, evidence, delivery-time, and review-time
  thresholds pass;
- zero confirmed critical permission or sensitive-data leak occurs;
- all final artifacts contain complete evidence manifests;
- source changes successfully identify affected work and replay produces a
  recorded exact/equivalent/different result;
- support effort and bespoke steps are known; and
- the customer is ready for an executive production/renewal decision.

### Phase 11 — MVP readout and commercial decision (week 16)

Tasks:

- produce a joint before/after readout using the scorecard in Section 8;
- report every failed run, manual repair, unsupported claim, security finding,
  source outage, model failure, and excluded case rather than reporting only
  successful outputs;
- calculate analyst and reviewer hours avoided using the customer's actual
  baseline;
- present deployment time, recurring support time, and projected gross-margin
  inputs;
- ask for one explicit outcome:
  - annual production contract for the first workflow;
  - paid extension with specified remaining gates;
  - expansion to another metric/report using the same source; or
  - a documented no-go with reasons;
- obtain permission before using customer names, metrics, quotes, or results;
- update the product requirements, threat model, pricing assumptions, solution
  pack, connector certification, runbooks, and roadmap from the evidence;
- publish an independently run evaluation when valid external evidence is
  available; and
- do not claim analyst replacement until accuracy, coverage, review burden,
  reliability, cost, and customer adoption support that statement.

MVP validated gate:

- the release-candidate and shadow-pilot gates pass;
- the customer completes the agreed decision process; and
- the team can state exactly what is reusable, what remains bespoke, what the
  workflow costs to operate, and what would falsify expansion.

## 8. MVP scorecard and instrumentation

The pilot contract should set numerical thresholds before opening evaluated
outputs. At minimum, report every metric below.

| Metric | Definition | MVP release expectation |
| --- | --- | --- |
| Representative task completion | Tasks reaching the correct terminal outcome and complete deliverable divided by attempted supported tasks | At least 90% on the frozen supported set |
| Material-number accuracy | Correct material numbers divided by all material numbers | 100% for published artifacts |
| Unsupported-claim rate | Material claims lacking valid required support divided by all material claims | 0% in published artifacts |
| Permission/sensitive-data violations | Confirmed unauthorized retrieval, inference, execution, display, log, or publication | Zero |
| Error-detection performance | Seeded SQL, schema, metric, freshness, policy, and unsafe-query errors correctly blocked or downgraded | 100% of critical seeded cases; publish per-class results |
| Chart-to-data correctness | Displayed values/labels matching the verified result IR | 100% |
| Slide-to-evidence correctness | Material slide claims resolving to current evidence | 100% |
| Time to completed artifact | Request/schedule to review-ready package, with component latency | Customer-agreed improvement over baseline |
| Human review minutes | Active reviewer time per artifact | Customer-agreed improvement without lower defect detection |
| Manual repair rate | Supported recurring runs needing engineer or analyst repair | Track each cycle; target should be agreed before pilot |
| Scheduled-run success | Eligible runs completed excluding declared source outages | At least 99% at paid-pilot maturity |
| Replay success | Eligible source-change replays producing a durable comparison | 100% of tested supported cases |
| Invalidation coverage/lag | Affected known claims found and reclassified within the target time | 100% of seeded dependents; lag target agreed by workflow |
| Evidence completeness | Final artifacts with complete claim/evidence manifest | 100% |
| Deployment/onboarding time | Start of approved access to first accepted workflow run | Fewer than ten business days |
| Analyst hours avoided | Baseline analyst time minus retained AMOS-assisted analyst time | Report observed value; do not estimate generic incident savings |
| Support burden | Founder/engineer hours per customer per week and incidents per run | Must trend down across cycles and customers |
| Customer decision | Renew, deploy, expand, extend, or stop | Explicit at the agreed date |

Also track commercial discovery weekly:

- qualified interviews and observed workflows;
- paid pilots proposed, won, lost, and reason;
- time from first call to pilot;
- buyer, champion, reviewer, and security engagement;
- weekly governed runs and active reviewers;
- requested repeats, additions, and expansions; and
- actual contract value, implementation cost, model/hosting cost, support cost,
  and emerging gross margin.

## 9. Paid-pilot release checklist

Every answer must be yes before AMOS handles the scoped production-like pilot
workflow.

### Product and evidence

- [ ] A real user request and schedule both produce a complete decision package.
- [ ] The local/customer-approved model proposes the typed plan and structured narrative/slide plan.
- [ ] Every required context role is present or a visible non-pass outcome occurs.
- [ ] Every material number resolves to query, result hash, source version, metric, schema, and verification.
- [ ] Every chart and slide conclusion resolves to the verified result IR.
- [ ] The PPTX is editable and the report/data appendix open in supported tools.
- [ ] Assumptions, limitations, freshness, sensitivity, audience, and review requirements are visible.
- [ ] Reviewer corrections affect the next applicable task while originals remain immutable.
- [ ] Follow-ups preserve the governing analytical scope unless the user explicitly changes it.

### Authorization and security

- [ ] Enterprise identity, groups, session lifecycle, and policy epoch work.
- [ ] Permission filtering occurs before ranking and is rechecked at execution, read, commit, and publication.
- [ ] Source-native row/column controls remain effective.
- [ ] Model and API components have no warehouse credentials.
- [ ] Workers have isolated identity, short-lived capabilities, resource limits, and default-deny egress.
- [ ] Secret rotation and revocation have been rehearsed.
- [ ] Cross-tenant, prompt-injection, poisoning, capability-replay, SSRF, secret-leak, and DoS tests pass.
- [ ] There are zero open critical findings.

### Durability and change handling

- [ ] Duplicate requests, reviews, events, jobs, renders, and retries do not duplicate effects.
- [ ] Permission revocation between retrieval and publication prevents commit.
- [ ] Metric, schema, policy, source, and data-state changes identify dependent claims.
- [ ] Replay creates a new transaction and exact/equivalent/different comparisons without modifying the original.
- [ ] Lost publication acknowledgment retries with the same stable publication ID.
- [ ] Worker/process loss at every durable boundary recovers with correct fencing.
- [ ] Retention, legal hold, erasure, customer export, and offboarding behave as documented.

### Deployment and operations

- [ ] The signed OCI/VM release installs in a clean customer-like environment.
- [ ] `amosctl` validates, migrates, diagnoses, upgrades, rolls back, backs up, restores, and exports.
- [ ] PostgreSQL RLS, object storage, durable queues, KMS/secrets, and isolated workers are enabled.
- [ ] SBOM, licenses/notices, vulnerability scan, signatures, checksums, and provenance accompany the release.
- [ ] Dashboards, alerts, support routing, customer-visible status, and audit export work.
- [ ] Restore, source outage, model outage, queue backlog, failover, invalidation storm, and publication recovery drills pass.
- [ ] The declared availability, scheduled-run, latency, RPO, and RTO objectives pass at supported capacity.
- [ ] Onboarding is documented and takes fewer than ten business days.

### Customer value

- [ ] The baseline and evaluation set were frozen before scored runs.
- [ ] Four weekly shadow cycles completed with failures included.
- [ ] The customer-agreed correctness, time, review, and evidence targets passed.
- [ ] Support hours and bespoke implementation steps are recorded.
- [ ] The buyer completed an explicit production, renewal, expansion, extension, or no-go decision.

## 10. Architecture at MVP scale

The supported initial topology is:

- one modular Rust control-plane application with stateless API/runtime replicas;
- PostgreSQL as the authoritative state-transition, metadata, evidence, audit,
  job, and outbox database;
- versioned object storage for rendered files and large retained bodies;
- a durable queue with separate interactive and scheduled lanes;
- isolated worker pools partitioned by tool/risk;
- a separately deployed model server with no direct source credentials;
- enterprise identity and source-native authorization mapping;
- a secret manager and KMS/HSM integration;
- a private network path to the customer source;
- a telemetry backend for metrics, logs, traces, alerts, and audit export; and
- separate development, staging, pilot, and production configuration.

Do not split the control plane into microservices during the MVP. Extract a
component only after one of these conditions is measured:

- worker isolation cannot be enforced in the current boundary;
- model/GPU serving requires an independent lifecycle or scaling profile;
- connector traffic scales independently;
- context indexes exceed PostgreSQL capacity;
- invalidation fan-out damages interactive latency;
- an enterprise deployment requires separate control and data planes; or
- team ownership requires an independent release cycle.

When extraction occurs, preserve the existing typed contracts, tenant scope,
idempotency, fencing, hashes, audit, and capability rules.

## 11. Six-to-nine-month repeatability and scaling plan

Scaling begins only after the validated MVP produces customer evidence.

### Commercial repeatability

1. Close two additional paid design partners in the same workflow family.
2. Run the same baseline and four-cycle shadow protocol for each.
3. Convert at least two pilots to annual contracts, producing at least two
   annual contracts from the first three paid design partners.
4. Require the second and third deployment to use the same core runtime without
   customer-specific core branches.
5. Track time-to-onboard, support hours, bespoke steps, run volume, renewal,
   expansion, gross retention, net retention, and gross margin.
6. Publish customer evidence or an anonymized independent evaluation only with
   permission and complete failure reporting.

### Productization of field work

Turn every repeated deployment task into one of:

- a certified connector feature or conformance case;
- a solution-pack schema, validation, or reusable component;
- a metric/semantic mapping adapter;
- a verifier rule pack;
- a prompt/model evaluation;
- a company artifact template primitive;
- a deployment preflight or `amosctl` automation;
- a support diagnostic, alert, or runbook; or
- documentation and a customer-admin workflow.

The primary scaling metric is:

> Does each deployment reduce onboarding time and bespoke engineering for the
> next deployment while maintaining correctness and isolation?

If the answer does not improve across three customers, AMOS is behaving like a
consulting engagement rather than a repeatable product.

### Land-and-expand order

Expand inside a customer in this sequence:

1. complete one recurring report and editable slide deck for one team;
2. add more approved questions and metric families using the same source;
3. automate more recurring reports for the same team;
4. add one additional approved source in the same environment;
5. expand the proven packs to another team or business unit; and
6. add a controlled higher-risk action only with its own policy, approval,
   idempotency, rollback, monitoring, kill switch, and safety evidence.

### Connector and deployment breadth

- build one connector at a time in response to signed demand;
- do not call a connector supported until its real-service certification
  passes authentication, discovery, read, permission, cursor, freshness,
  outage, rotation, quota, retry, revocation, and deletion cases;
- add Kubernetes, dedicated tenancy, regional placement, customer-managed keys,
  or complete air-gap qualification only when contracted requirements justify
  their operating cost;
- defer international expansion until one deployment and support model is
  repeatable; and
- use warehouse, semantic-layer, catalog, identity, cloud-marketplace, and
  audit partners as distribution/integration leverage, not as substitutes for
  customer demand.

### Capacity scaling

- scale persistent memory independently from model context;
- keep structured tenant, policy, status, type, time, and label filters in the
  authoritative store;
- move blobs to object storage;
- add dedicated lexical/vector candidate search behind the memory interface
  only when measured recall or latency requires it;
- shard by tenant before sharding within a tenant;
- preserve reverse dependency indexes for invalidation;
- cache only version-addressed, permission-scoped results and evict on policy
  epoch changes; and
- track active tenants, memory objects, version rate, dependency edges,
  schedules, tool concurrency, result bytes, model tokens, p50/p95/p99 latency,
  queue age, invalidation fan-out, RSS, and noisy-neighbor impact.

## 12. Stop, narrow, or pivot conditions

The team should not continue broad platform construction merely because the
control kernel is technically strong. Reassess if any of these occur:

- 30 qualified interviews yield no urgent workflow, workflow access, or paid
  pilot;
- existing warehouse, catalog, BI, or agent platforms already satisfy the
  customer's requirement at acceptable cost;
- four shadow cycles do not improve time, review burden, reconstruction, or
  error escape rate;
- human review makes the workflow slower without a compensating quality or
  risk benefit;
- each customer requires a different core architecture rather than a new pack
  or connector;
- integration and support cost make the proposed annual contract uneconomic;
- customers will not place AMOS in the trusted path even with customer-local
  deployment;
- critical permissions or data-handling failures cannot be resolved within the
  pilot boundary; or
- the team cannot identify a buyer with budget and authority.

Possible responses are to narrow the workflow, sell an integration into an
incumbent platform, focus on the claim/evidence transaction as a component, or
stop. The repository and synthetic evaluation alone are not reasons to ignore
negative customer evidence.

## 13. Immediate next ten actions

1. Assign the five ownership roles and approve the MVP/non-goal boundary.
2. Create the work-item tracker from this document and mark the critical path.
3. Add CI, licensing, security policy, ADRs, and accurate archive labels.
4. Begin the 30-account interview campaign and schedule ten workflow
   observations.
5. Draft the paid shadow-pilot statement of work and scorecard.
6. Design the solution-pack and verified-result IR contracts.
7. Refactor the payment runtime behind pack, connector, planner, and renderer
   interfaces without weakening the current tests.
8. Prototype the provider-neutral model loop and evaluate it on frozen tasks.
9. Prototype editable PPTX plus report and data-appendix generation from the
   verified-result IR.
10. Once a paid partner selects the environment, implement that connector,
    identity, deployment, and publication path and discard guessed integrations
    from the MVP backlog.
