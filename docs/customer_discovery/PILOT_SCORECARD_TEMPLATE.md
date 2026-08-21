# Paid shadow-pilot scorecard

Status: **template; freeze with the customer before evaluated outputs are opened**

## Scorecard control

| Field | Agreed value |
| --- | --- |
| Account/workflow ID | TBD |
| Baseline period | TBD |
| Four shadow-cycle dates | TBD |
| Customer scorecard owner | TBD |
| AMOS scorecard owner | TBD |
| Freeze date and version | TBD |
| Commercial decision date | TBD |
| Allowed exclusions/source outages | TBD |

## Product and safety measures

| Metric | Definition | Baseline | Pilot threshold | Evidence source | Cycle 1 | Cycle 2 | Cycle 3 | Cycle 4 | Final |
| --- | --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| Representative task completion | Supported tasks reaching correct terminal outcome with complete deliverable / attempted supported tasks | TBD | ≥90% | Frozen task records and package manifests | — | — | — | — | — |
| Material-number accuracy | Correct material numbers / all material numbers | TBD | 100% for published artifacts | Sealed references and verified-result IR | — | — | — | — | — |
| Unsupported-claim rate | Material claims without valid required support / all material claims | TBD | 0% published | Claim/evidence manifests | — | — | — | — | — |
| Permission/sensitive-data violations | Confirmed unauthorized retrieval, inference, execution, display, log, or publication | TBD | 0 | Security and audit review | — | — | — | — | — |
| Critical seeded-error detection | Critical SQL/schema/metric/freshness/policy/unsafe cases caught / seeded critical cases | TBD | 100% | Frozen fault-injection set | — | — | — | — | — |
| Chart-to-data correctness | Correct displayed values and labels / checked values and labels | TBD | 100% | Chart binding validation | — | — | — | — | — |
| Slide-to-evidence correctness | Material slide claims resolving to current evidence / material slide claims | TBD | 100% | Slide and evidence manifests | — | — | — | — | — |
| Replay success | Eligible tested changes with durable comparison / eligible tested changes | TBD | 100% | Replay records | — | — | — | — | — |
| Evidence completeness | Final artifacts with complete manifests / final artifacts | TBD | 100% | Package manifest | — | — | — | — | — |

## Customer-value and operating measures

| Metric | Measurement method | Baseline | Agreed threshold | Cycle 1 | Cycle 2 | Cycle 3 | Cycle 4 | Final |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Request/schedule to review-ready package | Median elapsed hours, excluding declared source outage | TBD | TBD improvement | — | — | — | — | — |
| Analyst active time | Minutes per run measured by participant log | TBD | TBD improvement | — | — | — | — | — |
| Reviewer active time | Minutes per run without reducing defect detection | TBD | TBD improvement | — | — | — | — | — |
| Manual repair rate | Supported runs needing analyst/engineer repair / supported runs | TBD | TBD | — | — | — | — | — |
| Scheduled-run success | Eligible scheduled runs completed / eligible scheduled runs | TBD | Track; ≥99% at pilot maturity | — | — | — | — | — |
| Invalidation coverage and lag | Seeded affected claims found; minutes to classification | TBD | 100%; lag TBD | — | — | — | — | — |
| Deployment/onboarding time | Approved access to first accepted run in business days | — | <10 business days | — | — | — | — | — |
| AMOS support burden | Founder/engineer hours and incidents per run | — | Downward trend | — | — | — | — | — |

## Cycle decision record

For each cycle, record source/model incidents, AMOS incidents, exclusions,
manual interventions, reviewer corrections, scorecard deviations, and scope
requests. Never delete failed runs from the denominator unless the frozen
exclusion rule applies.

Final customer decision: `Production` / `Renew` / `Expand` / `Extend` / `Stop`

Decision rationale, signed by the buyer and scorecard owners: TBD
