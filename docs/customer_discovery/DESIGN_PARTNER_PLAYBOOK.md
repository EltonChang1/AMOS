# AMOS design-partner playbook

Status: **working hypothesis; validate through interviews**

## Phase 1 outcome

Secure one fixed-scope paid shadow pilot with:

- one named recurring analytical workflow;
- one real read-only source;
- three to ten approved metrics and no more than three query shapes;
- one data owner and one reviewer;
- one agreed report and presentation template;
- one publication or export destination;
- a frozen baseline and success scorecard; and
- an explicit commercial decision date after four weekly shadow cycles.

Do not select a connector, department, or workflow from the current payment
fixture or general market share. Customer evidence makes that choice.

## Initial customer hypothesis

The starting hypothesis is a company with at least 500 employees that already
has a data warehouse, approved metrics, recurring analyst work, and business
teams waiting for reviewed reports or presentations.

Likely economic buyer: one accountable VP/Head of Data, Analytics, or AI
Platform. Finance, Revenue Operations, Risk, or Operations leadership may be
the buyer only when that role owns the budget and deployment decision for the
observed workflow.

Likely participants:

| Role | Evidence required |
| --- | --- |
| Economic buyer | Owns budget, urgency, procurement path, and decision date |
| Technical champion | Will coordinate architecture, access, and weekly working sessions |
| Data owner | Can approve source, schema, metric definitions, and sanitized fixtures |
| Reviewer | Currently judges the report and can assess correctness and review time |
| Security stakeholder | Can approve the deployment/data boundary or give a concrete path to approval |
| Daily user | Performs or coordinates the current analysis and artifact preparation |

## Hard qualification gates

A candidate cannot advance to a pilot proposal until all answers are yes:

1. Is there a real recurring review at least weekly or with equivalent urgency?
2. Does it use a warehouse or another bounded read-only analytical source?
3. Are the governing metrics and a reviewer identifiable?
4. Can the customer provide a read-only non-production path or approved shadow
   access without prohibited data?
5. Can current analyst time, reviewer time, correction rate, or another useful
   baseline be measured?
6. Will the data owner, reviewer, and champion attend weekly sessions?
7. Is there a plausible paid procurement path and a named decision date?
8. Can the workflow remain inside one source, one department, and one approved
   destination for the pilot?

Disqualify or defer strategy workshops without workflow access, requests for
arbitrary Python or production writes, workflows dominated by prohibited or
regulated raw data without an approved safety case, and opportunities that
require a new core architecture.

## Three-conversation motion

### Conversation 1: problem and priority

Goal: determine whether the problem is current, costly, and owned. Do not demo
the platform or lead with “AI governance.”

Opening prompt:

> Walk me through the last recurring business review where an analyst had to
> collect data, calculate changes, explain what happened, build charts or
> slides, and get the output approved.

Follow with concrete questions:

- What decision did the deliverable support, who received it, and what happened
  after it was reviewed?
- How often is it produced, and when was the last late or corrected version?
- Who performed each step and how much active/waiting time did it take?
- Which step is hardest to delegate or verify?
- What metric, freshness, schema, permission, or explanation failure has
  occurred recently? What did it cost in rework or decision delay?
- Which existing warehouse, BI, semantic, catalog, lineage, ticket, and slide
  tools are involved?
- Who owns the budget and who could veto a pilot?
- What priority would displace this work this quarter?
- If the workflow were materially faster and every important number remained
  reviewable, what would the organization do differently?

End by requesting a workflow-observation session, not a sale.

### Conversation 2: observe the real workflow

Goal: watch a recent or live instance from request through later corrections.
Use the [workflow record](WORKFLOW_RECORD_TEMPLATE.md). Ask the participant to
show existing artifacts with sensitive values redacted when necessary.

Observe rather than infer:

1. request and schedule trigger;
2. source, identity, and access path;
3. definitions and time/population choices;
4. query and calculation sequence;
5. chart, narrative, slide, and appendix construction;
6. reviewer questions and corrections;
7. delivery destination and acknowledgment; and
8. what happens after data, schema, or definitions change.

End by confirming the baseline measurements, hard gates, stakeholders, and
whether a paid shadow pilot is worth scoping.

### Conversation 3: fixed-scope pilot

Goal: agree on the exact workflow, scorecard, access process, fee, and decision
date. Use the SOW, scorecard, and security-review templates in this directory.
Do not expand scope to win verbal enthusiasm.

## Candidate scoring rubric

Score only from observed or explicitly confirmed evidence. Unknown is zero,
not an optimistic estimate.

| Criterion | Weight | Full-credit evidence |
| --- | ---: | --- |
| Business urgency | 15 | Named decision is impaired now; buyer commits a decision date |
| Recurrent and bounded workflow | 10 | Same reviewed package runs weekly with stable boundaries |
| Measurable baseline | 10 | Current active time, elapsed time, correction/error data, and samples exist |
| Source readiness | 10 | One identified source with an approved read-only test/shadow path |
| Definition readiness | 10 | Three to ten owned metrics and up to three query shapes are identifiable |
| Reviewer/data-owner engagement | 10 | Named people agree to weekly participation and acceptance duties |
| Security feasibility | 10 | Trust boundary and prohibited data are known; approval path is credible |
| Workflow access | 10 | Customer permits end-to-end observation and sanitized fixture creation |
| Commercial path | 10 | Budget owner, procurement steps, pilot fee range, and decision date are known |
| Reuse potential | 5 | Workflow can become configuration, template, connector certification, or test |

Advancement rule: all hard gates pass and the score is at least 70/100. The
score ranks candidates; it does not override a failed security, access, review,
or commercial gate.

## Evidence and note-taking rules

- Record facts, direct customer language, and observed behavior separately from
  AMOS interpretations.
- Record “unknown” instead of filling gaps from public research or assumptions.
- Do not count advisors, repository users, targets, or informal conversations as
  qualified interviews unless the buyer/workflow criteria are met.
- Store live names and contact details in an approved private system. Use
  sanitized account and participant IDs in public repository artifacts.
- Obtain permission before recording a call or retaining customer artifacts.
- Never request credentials, raw sensitive data, or proprietary exports during
  discovery. The security gate comes first.

## Weekly review

Review counts and evidence every week:

- qualified interviews completed;
- workflow observations completed;
- candidates above 70 with all hard gates passed;
- pilot proposals sent, won, lost, and reason;
- elapsed time from first call to signed pilot;
- buyer, champion, data-owner, reviewer, and security engagement;
- most frequent existing alternative and objection; and
- whether evidence supports narrowing, repositioning, integrating, or stopping.

After 30 qualified interviews, absence of urgent workflow access or willingness
to pay triggers the stop/narrow decision in the execution plan.
