# AMOS Frequently Asked Questions

This document explains AMOS in plain language. It separates the long-term
product idea from what the current repository actually implements.

## Quick explanation

### What is AMOS?

AMOS is a governed operating layer for AI data-analysis agents. It sits between
an agent and a company's databases and analytical tools. It controls what the
agent may see and do, verifies proposed work, records evidence, and supports
human review and reproducible results.

In one sentence:

> The agent proposes the analysis; AMOS authorizes, executes, and records it.

### What problem does AMOS solve?

Companies want to use AI with internal data, but they may not trust an AI agent
with unrestricted database credentials or sensitive information. They also
need to know how an AI-generated conclusion was produced and whether it is
still valid.

AMOS addresses this by adding permissions, policy checks, safe execution,
evidence, audit history, review, replay, and invalidation between the agent and
the data systems.

### Is AMOS an operating system?

Not in the same sense as Linux, Windows, or macOS. "Operating layer" is an
architectural description: AMOS coordinates governed memory, permissions,
tools, execution, evidence, and lifecycle state for analytical agents.

### Is AMOS an AI model or chatbot?

No. AMOS is not a large language model, and the current repository does not
embed a general-purpose AI agent. An external agent or application can connect
to AMOS through its API.

## Architecture

### Where does AMOS sit?

The basic architecture is:

```text
User
  -> AI agent
  -> AMOS control layer
  -> sandboxed analytical tools
  -> databases, warehouses, and files
```

The model can reason about the request and propose a plan. AMOS controls access
to data and tools and records what actually happened.

### Does the AI agent sit inside AMOS?

It does not have to. The recommended design keeps the agent logically above
AMOS. This allows a company to change models or agent frameworks without
rebuilding the governance and execution layer.

An application may package the agent and AMOS together for deployment, but
their responsibilities should remain separate.

### Should analytical tools be inside AMOS?

AMOS should own the tool registry, authorization, input validation, capability
issuance, resource limits, and execution records. The actual tool workers can
run as separate processes, containers, or services.

This separation keeps the core small and makes risky tools, such as Python
execution, easier to isolate.

### Who chooses which tool to use?

The agent can propose and sequence tools based on the user's question. AMOS
makes the final authorization decision.

For example, an agent may request SQL extraction followed by PCA and charting.
AMOS would independently check whether the user may access the data, whether
each tool is allowed, and whether the requested resources and outputs comply
with policy.

### Can an agent call several tools in one analysis?

Conceptually, yes. Each tool call should be independently authorized and
recorded. Later steps should explicitly depend on the outputs of earlier steps
so that the complete analysis can be reviewed and replayed.

## Data access and security

### Why not let the agent interact with the database directly?

Direct access is simple, but it can expose excessive permissions, sensitive
columns, database credentials, or destructive and expensive operations. It
also makes evidence, replay, and accountability harder.

AMOS provides a controlled path in which the database still enforces its own
permissions and AMOS adds a second layer of policy and execution controls.

### Does AMOS replace database security?

No. Database permissions, network controls, row-level security, column masking,
and identity management should remain in place. AMOS is an additional control
and evidence layer, not a substitute for the database's security boundary.

### Does AMOS give the model raw database credentials?

It should not. Credentials should remain in trusted infrastructure. AMOS can
issue narrowly scoped, short-lived capabilities to approved workers instead
of exposing long-lived credentials to the model.

### Does company data have to leave the company?

Not necessarily. AMOS can be paired with different model deployments:

- a fully self-hosted model inside the company's environment;
- a private cloud model or private endpoint;
- an external model that receives only approved, minimized context.

The current local MVP does not by itself provide every production isolation,
identity, and cloud integration required for these deployment modes.

### Does using a self-hosted model remove the need for AMOS?

No. A self-hosted model can still run an incorrect query, access information
the user should not see, use the wrong business definition, or produce an
unsupported conclusion. Model hosting addresses where computation happens;
AMOS addresses control, evidence, and accountability.

### Can AMOS prevent all data leaks or incorrect answers?

No system can make that guarantee. AMOS can reduce risk through least-privilege
access, validation, bounded execution, audit records, and review. Correct
deployment, database security, policies, worker isolation, and operational
monitoring are still required.

## Context and memory

### What does "memory" mean in AMOS?

Memory means governed organizational knowledge that an agent may need for an
analysis, such as metric definitions, schemas, policies, approved documents,
past decisions, and source versions. It is typed, versioned, permission-aware,
and traceable.

It is not simply an unrestricted transcript or a vector database containing
everything the organization knows.

### How does AMOS give context to an agent?

AMOS first filters information according to tenant, identity, object type,
status, time, labels, and policy. It then selects a bounded set of relevant
items and records a context manifest describing what was included and omitted.

### Why is permission filtering performed before ranking?

If unauthorized items enter retrieval before filtering, their existence or
content can influence the result. Filtering first helps prevent accidental
disclosure and makes the resulting context easier to audit.

### Does AMOS learn automatically from every interaction?

Not without control. Feedback and new memory should be governed, versioned,
and attributable. The current implementation supports reviewer approval,
correction, and append-only feedback records rather than silently treating
every model output as truth.

## Analysis and tools

### Does AMOS perform data analysis itself?

AMOS coordinates and governs analysis. Tool workers perform the actual SQL,
statistics, or chart computation. AMOS verifies inputs and plans, constrains
execution, and records outputs and evidence.

### Does AMOS currently support exploratory data analysis (EDA)?

Not as a general-purpose feature. The current repository implements a focused
local production slice with a read-only warehouse connector, a payment-failure
metric family, deterministic statistics, and charting.

General EDA across arbitrary datasets would require additional governed tools,
data profiling, sampling controls, output limits, and user interfaces.

### Does AMOS currently perform PCA?

No. The present implementation does not include Principal Component Analysis.
PCA could be added later as a versioned, sandboxed analytical tool with a
defined input contract and recorded parameters and outputs.

### Could AMOS support Python, notebooks, forecasting, or machine learning?

Yes as a future extension, but these are intentionally outside the current MVP.
Arbitrary Python and notebook execution introduce significant sandboxing,
dependency, network, reproducibility, and data-exfiltration risks. A production
implementation should begin with narrowly defined tools instead of unrestricted
code execution.

### How should a PCA tool work through AMOS?

A governed PCA operation should record at least:

- the exact input dataset and version;
- selected columns, filters, and sampling;
- missing-value handling and scaling method;
- algorithm and library version;
- component count and random seed, when applicable;
- execution limits and worker identity;
- explained variance, loadings, and generated artifacts;
- claims derived from the result and their review state.

### Can AMOS write to production databases?

The current implementation uses a read-only warehouse connector and does not
provide arbitrary production writes. Write support would require stronger
approval, transaction, rollback, and destination-specific controls.

### Can AMOS communicate or publish externally by itself?

The current MVP supports governed local publication. Autonomous external
communication and production cloud publication destinations are not included.

## Trust, evidence, and review

### What is a claim in AMOS?

A claim is a typed material statement in an analytical result, such as
"payment failures increased during this period." A claim can be connected to
the data, query, calculation, evidence, review status, and dependencies that
support it.

### What does "evidence-backed" mean?

It means a material conclusion is not stored only as prose. AMOS records the
computations, source versions, dependencies, and execution metadata needed to
inspect how the conclusion was produced.

### What is replay?

Replay runs a previously recorded analysis again using durable execution
metadata and creates new comparison evidence. It helps determine whether a
result can be reproduced and whether changed data produces a different answer.

Replay does not mean that old and new executions are guaranteed to be
identical when their inputs have changed.

### What is invalidation?

If a source table, schema, metric definition, policy, or upstream result
changes, claims that depend on it may no longer be reliable. AMOS can propagate
that change through dependency relationships and mark affected validity
dimensions accordingly.

### Does AMOS guarantee that a conclusion is true?

No. AMOS can verify permissions, schemas, supported query structure,
provenance, execution, and declared dependencies. It cannot automatically
guarantee that an analyst or model chose the correct method or interpreted the
result correctly. Important conclusions may still require domain review.

### Why is human review included?

Business conclusions can involve ambiguity, causal interpretation, and domain
knowledge that mechanical validation cannot settle. Reviewers can approve,
reject, or correct results, and AMOS records that decision without overwriting
the original history.

## Product and market

### Is AMOS a replacement for a data warehouse?

No. It connects to warehouses and governed data systems. It does not replace
their storage and query engines.

### Is AMOS a replacement for BI tools?

Not necessarily. BI tools remain useful for dashboards and established
reporting. AMOS can govern agent-driven analysis and potentially publish
approved outputs to BI destinations.

### Is AMOS similar to Databricks?

There is significant overlap across several Databricks products. Unity Catalog
provides governance, access controls, audit, and lineage; AI/BI Genie provides
natural-language data questions; and MLflow provides agent tracing,
evaluation, monitoring, and feedback.

AMOS aims to combine these concerns around a model- and warehouse-neutral
analytical transaction, with particular emphasis on claim-level evidence,
review state, dependency invalidation, and replay. AMOS must demonstrate that
this combined workflow is sufficiently valuable and easier to adopt than
assembling existing platform features.

### What is AMOS's strongest potential differentiation?

Its strongest potential distinction is claim-level lifecycle management: not
only recording that a query touched a table, but tracking which business
conclusion depends on which exact data, computation, metric definition, and
policy—and warning users when that conclusion becomes unreliable.

### Who might buy AMOS?

Likely buyers include enterprise data-platform, AI-platform, security,
governance, risk, and compliance teams. Initial users could be analysts and
reviewers producing important recurring reports in regulated or high-risk
environments.

### What should the first commercial product be?

A focused starting point is a governed gateway for AI-generated business
reports that makes each important conclusion auditable, reviewable,
reproducible, and invalidatable.

Trying to become a universal operating system for every data agent immediately
would create an excessive integration and product scope.

### What are the main benefits?

- Safer access to sensitive organizational data
- Consistent use of approved metrics and policies
- Evidence and provenance for important conclusions
- Human review and accountable corrections
- Reproducible analyses and durable audit history
- Detection of conclusions made stale by upstream changes
- Independence from a single model, agent framework, or warehouse

### What are the main disadvantages and risks?

- Additional latency and infrastructure between agents and data
- A large integration surface across warehouses, identity systems, and tools
- Complex policy configuration and operational responsibility
- Competition from Databricks, Snowflake, Microsoft, and governance vendors
- Difficulty proving correctness beyond execution and provenance
- Enterprise adoption friction from introducing another control layer
- Risk of an unclear buyer if the product is positioned too broadly

### How might AMOS be priced?

Possible models include per governed execution, per active user, per connected
data environment, or an annual enterprise platform license. Pricing should
reflect reduced governance and review cost rather than model-token usage alone.

### What would demonstrate product-market fit?

Useful signals include customers running recurring production analyses through
AMOS, shorter review time, fewer policy or metric errors, successful audit
evidence, detection of stale published claims, and expansion to additional
teams or data sources.

## Current repository

### What does the current MVP include?

The repository provides a Rust-based local implementation with:

- one configured tenant;
- one read-only SQLite warehouse connector;
- a payment-failure reference analysis;
- governed, typed, and versioned memory;
- permission-first context construction;
- SQL, schema, metric, and policy verification;
- bounded deterministic statistics and charting;
- capability-limited execution workers;
- typed claims, evidence, dependencies, and review;
- transitive invalidation and durable replay;
- crash recovery, audit records, retention, and local publication;
- an HTTP API, command-line interface, and server-rendered product surfaces.

### What is intentionally not included today?

- A built-in general-purpose LLM or autonomous agent
- General PCA or unrestricted EDA
- Arbitrary Python or notebook execution
- Arbitrary production database writes
- General multi-agent scheduling
- Unreviewed causal claims
- Autonomous external communication
- Completed enterprise identity, KMS, cloud object storage, or customer
  warehouse integrations
- Production container or VM sandboxing and egress enforcement

### Is the repository production-ready for enterprise deployment?

It implements and tests a complete local production slice, but production
deployment still requires infrastructure-specific integrations such as
enterprise identity, hardened worker isolation, customer warehouse
credentials, managed database deployment and recovery, key management, cloud
storage controls, and external publication adapters.

### Why is AMOS written in Rust?

Rust provides strong type and memory safety, predictable performance, and a
good foundation for a security-sensitive runtime. In this repository, the
application, API, CLI, persistence, connectors, workers, and tests are all
implemented in Rust.

### How can I run the demo?

Install a stable Rust toolchain, then run:

```bash
cargo run -- --demo seed
cargo run -- --demo serve
```

The demo identities and signing key are for local development only. See the
project README for authentication examples, CLI commands, and verification
instructions.

## Short descriptions for different audiences

### For an executive

AMOS lets a company use AI agents on internal data while controlling access and
keeping evidence for every important conclusion.

### For an analyst

AMOS helps an AI agent use approved data and metrics, run controlled analyses,
and produce results that can be reviewed and reproduced.

### For a security team

AMOS is a least-privilege gateway that prevents models from receiving direct,
unrestricted database access and records authorized agent activity.

### For a data-platform team

AMOS is a control and evidence plane between analytical agents, governed
context, execution workers, and existing data platforms.

### For an investor

AMOS is infrastructure for trustworthy enterprise data agents, differentiated
by claim-level provenance, review, replay, and continuous validity.

## Core principle

> The AI agent decides what analysis to propose. AMOS decides what is allowed,
> controls how it runs, and preserves the evidence needed to trust it.
