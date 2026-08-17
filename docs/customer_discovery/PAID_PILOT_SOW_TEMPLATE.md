# AMOS paid shadow-pilot statement of work

Status: **commercial template; requires founder, customer, and legal review**

This template defines delivery scope and acceptance. It is not legal advice and
does not replace an executed agreement, data-processing terms, security terms,
or any required industry-specific review.

## 1. Parties and control

| Field | Agreed value |
| --- | --- |
| Customer legal entity | TBD |
| AMOS legal entity | TBD |
| Effective date | TBD |
| SOW owner for each party | TBD |
| Pilot fee and payment schedule | TBD; price-discovery hypothesis is $25,000–$75,000 |
| Approved start date | TBD; after access/security prerequisites |
| Four shadow-cycle dates | TBD |
| Executive readout date | TBD |
| Commercial decision date | TBD |

## 2. Objective

AMOS will configure and evaluate one read-only recurring analytical workflow in
shadow mode beside the customer's existing process. The purpose is to measure
whether AMOS produces a correct, evidence-complete, reviewable decision package
with less analyst/reviewer effort and acceptable operational burden. AMOS will
not replace the current production process during the shadow period.

## 3. Included scope

| Boundary | Agreed pilot scope |
| --- | --- |
| Department and workflow | One: TBD |
| Business decision and audience | TBD |
| Trigger | One recurring schedule plus approved ad hoc invocations |
| Source and connector | One read-only source: TBD |
| Metric family | Three to ten owner-approved metrics: TBD |
| Query shapes | Up to three approved shapes: TBD |
| Time/population parameters | TBD |
| Charts | One mandatory; no more than three total: TBD |
| Artifact templates | One company report and one presentation template |
| Data appendix | One XLSX/CSV appendix generated from verified results |
| Reviewer | One role and named acceptance owner: TBD |
| Publication/export | One approved destination: TBD |
| Model | One local or customer-approved provider behind the AMOS interface: TBD |
| Deployment boundary | One customer-approved environment: TBD |
| Retention/deletion | TBD |
| Support channel/hours | TBD |

Expected decision package: direct answer, executive summary, tables, accessible
charts, editable presentation, HTML/PDF report, data appendix, assumptions,
limitations, freshness, review decisions, and claim/evidence manifest.

## 4. Explicitly excluded scope

- production writes or actions;
- more than one source, department, reviewer role, or publication destination;
- arbitrary SQL, Python, notebooks, plugins, tools, or exploratory analysis;
- causal, regulated, external, or high-impact conclusions without agreed human
  review;
- prohibited data classes listed below;
- custom ontology, catalog, semantic layer, lineage system, warehouse, or BI
  replacement;
- new identity, connector, model, artifact, or deployment targets not named in
  this SOW;
- automatic external communications beyond the approved destination;
- general availability, multi-region SaaS, Kubernetes, certification, or
  compliance commitments not explicitly stated here; and
- production cutover during the four shadow cycles.

## 5. Deliverables and acceptance

| Deliverable | Acceptance evidence | Customer acceptance owner |
| --- | --- | --- |
| Frozen workflow definition and solution-pack inputs | Approved metrics, schemas, parameters, rules, templates, and evaluation cases | Data owner |
| Certified source path | Agreed read-only conformance tests pass in approved environment | Data owner/security stakeholder |
| Configured shadow workflow | Request and schedule produce the scoped review-ready package | Daily user/reviewer |
| Four weekly shadow packages | Each run and failure is preserved and scored against the frozen scorecard | Reviewer |
| Audit/replay package | Inputs, versions, evidence, review, replay, and publication state can be reconstructed | Reviewer/security stakeholder |
| Executive readout | Joint baseline/after results, deviations, incidents, support effort, and commercial options | Economic buyer |

Acceptance does not require every run to succeed. It requires truthful
measurement against the frozen scorecard, preservation of failures, resolution
of critical security/data-loss defects, and delivery of the agreed evidence.

## 6. Customer responsibilities

The customer will:

1. provide named buyer, champion, data owner, reviewer, security, and
   procurement contacts with decision authority;
2. provide timely access to a customer-approved environment, identity path,
   read-only source, network route, templates, and current-process samples;
3. approve exact metrics, filters, populations, time ranges, query shapes,
   sensitivity, prohibited data, and reference outputs;
4. preserve its existing workflow as the authoritative process during shadow
   mode and identify source outages or upstream changes;
5. participate in baseline collection, weekly review sessions, incident
   decisions, and the final executive readout;
6. review or reject packages within the agreed service window and record
   corrections without altering source evidence; and
7. obtain internal approvals and ensure it has the rights to provide all data,
   templates, definitions, and system access used in the pilot.

Delays in these prerequisites adjust the schedule without silently reducing
qualification or safety gates.

## 7. AMOS responsibilities

AMOS will:

1. keep operation read-only and within the approved source, tenant, workflow,
   tool, model, destination, and data boundary;
2. prevent the model from receiving source credentials or directly invoking
   company systems;
3. independently validate approved plans, calculations, claims, chart bindings,
   review obligations, and publication state;
4. preserve versioned inputs, outputs, hashes, audit events, failures,
   corrections, and replay/invalidation results;
5. notify the customer through the agreed channel of incidents, blocked runs,
   material scorecard deviations, and required security decisions;
6. delete or return customer data and credentials according to the agreed
   retention and offboarding procedure; and
7. record separately all reusable product work and customer-specific work.

## 8. Data handling and security schedule

The parties must approve the separate
[data-flow and security review](DATA_FLOW_SECURITY_REVIEW_TEMPLATE.md) before
AMOS receives credentials or customer data. At minimum, record:

- authorized data classes and explicitly prohibited data;
- deployment, model, telemetry, storage, backup, support, and publication
  boundaries;
- source-native identity and least-privilege read-only access;
- encryption, secrets, key rotation, worker isolation, and network controls;
- retention, legal hold, deletion, export, and termination handling;
- logging/redaction, audit access, incident contacts, and notification targets;
- approved subprocessors or an explicit statement that none apply; and
- customer responsibilities and residual risks.

No source access is authorized merely by signing this scope template.

## 9. Frozen success scorecard

Attach a completed [pilot scorecard](PILOT_SCORECARD_TEMPLATE.md) before opening
evaluated AMOS outputs. Required hard thresholds are:

- 100% material-number accuracy for published artifacts;
- 0% unsupported material claims in published artifacts;
- zero confirmed permission or sensitive-data violations;
- 100% chart-to-data and slide-to-evidence correctness;
- 100% evidence completeness and eligible tested replay success; and
- 100% of critical seeded error cases blocked or downgraded correctly.

Customer-value targets for analyst time, reviewer time, elapsed time, manual
repair, and invalidation lag must be set from the measured workflow baseline,
not generic ROI estimates.

## 10. Scope and change control

A material change includes another source, department, workflow family, metric
family, query shape beyond the agreed limit, reviewer role, destination, model,
environment, data class, regulated use, production write, artifact type, or
support obligation.

Material changes require a written change order stating:

- scope removed and added;
- security/data-flow impact;
- acceptance and scorecard impact;
- schedule and decision-date impact;
- separate implementation and recurring fee; and
- whether the change invalidates prior qualification evidence.

Material scope changes and customer-specific implementation work are
separately priced from the fixed pilot fee.

Silence, a chat request, or engineering effort does not amend scope.

## 11. Incidents, suspension, and termination

Either party may suspend access for suspected unauthorized access, data
exposure, destructive behavior, loss of required review control, or violation
of the approved boundary. AMOS must stop affected processing and preserve the
minimum authorized evidence for investigation.

Define before start:

- security and support contacts;
- severity definitions and response/notification targets;
- source outage versus AMOS incident classification;
- credential revocation procedure;
- data return/deletion and deletion-evidence procedure;
- treatment of backups, logs, support artifacts, and legal holds; and
- fees, work product, and obligations upon early termination.

## 12. Commercial decision

On the agreed decision date, the economic buyer records one outcome:

- production deployment under a new agreement;
- pilot renewal;
- expansion through a separately scoped workflow or environment;
- time-bounded extension with explicit unmet gates; or
- stop and offboard.

An extension is not an implicit success. It must name the unresolved evidence,
new date, cost, and stop condition.

## 13. Sign-off

| Approval | Name/title | Signature/date |
| --- | --- | --- |
| Customer economic buyer | TBD | TBD |
| Customer data owner | TBD | TBD |
| Customer security approver | TBD | TBD |
| AMOS authorized signer | TBD | TBD |
| Legal/privacy review, if required | TBD | TBD |
