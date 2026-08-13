# ADR 0005: Compile artifacts from a verified-result IR

- Status: Proposed
- Date: 2026-08-12
- Owners: Artifacts and platform lead; agent and runtime lead
- Decision scope: Evidence-to-artifact contract

## Context

Generating prose, charts, slides, and spreadsheets independently invites
numeric drift and makes claim support difficult to inspect. Direct rendering
from model text cannot guarantee that every material number matches an
authoritative calculation.

## Decision

Introduce one versioned verified-result intermediate representation containing
typed tables, units, display precision, chart bindings, claims, evidence links,
assumptions, limitations, freshness, sensitivity, audience, review obligations,
and input/template versions. The model may propose narrative and layout, but
deterministic compilers consume only validated IR.

PPTX, HTML/PDF, XLSX/CSV, charts, and the direct answer must derive from the
same IR. Every generated file and package manifest is hash-addressed; editable
source artifacts and accessible chart data remain available.

## Consequences

- Artifact formats stay consistent and can be validated independently.
- Templates and IR schemas require compatibility, upgrade, and replay rules.
- Unsupported prose or mismatched chart bindings are rejected before review.

## Validation gates

- Every material number resolves to calculation, metric, schema, source state,
  and verification evidence.
- Chart values exactly match their bound tables and include accessible labels.
- Replay with identical inputs produces identical content hashes where the
  format permits determinism.
- A reviewer can inspect claim support without engineering access.
