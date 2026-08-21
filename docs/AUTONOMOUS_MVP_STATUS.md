# AMOS autonomous MVP execution status

Status: **active engineering execution; MVP release candidate not complete**

Last updated: 2026-08-21

Working branch: `finance` (never `main` or `MVP`)

## Current execution item

**Phase 2 — signed domain-neutral solution-pack foundation.** The typed
contract, Ed25519 signature/trust boundary, validation and signing CLI, two
signed synthetic packs, authoring guide, and fail-closed tests are implemented
in the current working increment. Runtime activation, persistent pack history,
workflow routing, parameter binding, connector registration, and replacement
of fixed payment behavior remain the next dependent slice.

## Completed capabilities and evidence

### Inherited verified baseline

- Domain-neutral governed A-TXN control primitives, permission-first memory,
  read-only connector and SQL policies, deterministic execution, claim-level
  evidence, review, publication, invalidation, replay, recovery, API/UI, and
  operator CLI exist in the Rust baseline.
- The customer-evaluation Compose package uses non-root control/toolbox
  containers, explicit secrets, persistent volumes, health checks, diagnostics,
  and stopped-service backup. This remains evaluation-only.
- The governed tool catalog contains strict contracts and executable bounded
  adapters for the documented analysis/artifact primitives.
- Historical Python scenarios and evaluation commands are explicitly archived,
  non-executable research evidence in `PRODUCT_READINESS.md` and the evaluation
  guides.
- The bundled payment behavior is explicitly gated behind demo/reference
  commands and described as regression evidence, not product definition.

### Current increment

- Added strict `amos.solution_pack.v1` Rust types covering workflow identity,
  effective versions, owner approval, questions/schedules, explicit parameters,
  context roles, read-only sources/schemas, metrics/limits, optional banking
  metadata, plans/query shapes, verifier/review rules, claims/evidence,
  artifacts, publication, retention, and frozen evaluations.
- Added Ed25519 signing and tenant-scoped trust stores. Unsigned, tampered,
  unknown-key, wrong-tenant, incompatible, inactive, overlapping, ambiguous,
  write-capable, structurally unknown, or referentially invalid contracts fail
  closed before activation.
- Added `amosctl solution-packs sign` with private-key input only on standard
  input and `amosctl solution-packs validate` with manifest-hash and signer
  evidence output.
- Added signed aggregate synthetic bank-liquidity and payment-regression packs.
  The development publisher is limited to fixture tenants and is prohibited in
  pilot/production by documentation.
- Added unit and integration tests for signature, tamper, authorization,
  compatibility, ambiguity, strict parsing, source write protection, overlapping
  activation, checked-in signatures, and CLI validation.
- Added finance-branch CI and packaged-container checks for both signed fixtures.

## Verification evidence

Verification completed locally on 2026-08-21:

- `cargo fmt --all -- --check` passed.
- `cargo clippy --all-targets --all-features -- -D warnings` passed.
- `cargo test --all-targets --all-features` passed 83 tests (44 library, 3
  deployment, 15 API/CLI/UI, 18 governed-runtime, and 3 signed-pack tests) and
  the debug benchmark executable.
- `cargo test --all-targets --all-features --release` passed the same 83 tests
  and the 10,000-item bounded release benchmark. The final rerun after signer
  hardening observed 87.929 ms retrieval, 39.359 ms governed task, and 33.659
  ms persisted replay at p95, all below configured gates on this machine.
- `cargo build --all-features --release` and warning-free Rust documentation
  passed.
- `cargo audit` scanned 171 locked dependencies with no reported
  vulnerabilities.
- Both signed fixture validation commands passed with manifest hashes
  `sha256:213f6ae7189b38632436642bbf012cc42f380ce78faa68cd65035eae37e52451`
  (bank) and
  `sha256:d424164c937a4d73aff870d9dc213a7bd63624356dab6f1317fc5059c4b6c6c8`
  (payment regression).
- Docker Compose model validation and a clean Linux/aarch64 non-root image
  build passed. Image `sha256:8a8314b4391d8e7c82f79a135d6a2787abda714f1eeba93544124ff145367d3e`
  preserved user `10001:10001`, the expected entrypoint, and successfully
  validated both packaged packs from `/usr/share/amos/solution-packs`.
- `git diff --check` passed.

## Remaining work, ordered by priority and dependency

1. Complete Phase 2 runtime integration: durable tenant activation/version
   history, audit, upgrade/rollback, explicit workflow routing and parameters,
   pack-specific connector registration, configured plans/composition, and two
   end-to-end pack executions without core domain constants.
2. Finish unblocked Phase 0 repository gates: publish `SECURITY.md` after a
   private reporting route is selected; add the chosen root license after IP
   confirmation; record founder review/ownership and ADR decisions; audit the
   clean baseline. These decisions cannot be fabricated by engineering.
3. Implement the provider-neutral typed propose/validate/execute/interpret
   model loop and frozen evaluations without giving the model credentials or
   authoritative calculation duties.
4. Define the verified-result IR and compile deterministic direct answer,
   accessible charts, editable PPTX, HTML/PDF, XLSX/CSV, JSON, and evidence
   package outputs from verified data.
5. Complete bank application workflows: dashboard, parameter-scoped runs,
   async progress, schedules, follow-ups, evidence/artifact review,
   acknowledgments, administration, and accessibility.
6. Add customer-selected production infrastructure: one demanded read-only
   connector, enterprise identity, PostgreSQL/RLS, managed secrets, durable
   queue/object storage, and isolated risk-tiered workers.
7. Finish release lifecycle: signed digest-pinned images, SBOM/provenance,
   install, backup/restore, upgrade/rollback, export/uninstall, supported-version
   policy, and clean-server qualification.
8. Run security, failure-injection, recovery, performance, accessibility, and
   support qualification, then update release evidence.
9. Complete the external paid-partner workflow freeze and four scored weekly
   shadow cycles before any customer-validation or regulatory-readiness claim.

## External blockers and required decisions

- **Named owners and scope approval:** founders must name the five accountable
  roles, approve/amend the MVP boundary, and review the proposed ADRs.
- **License/IP:** contributors/IP owners must confirm whether MIT, currently
  declared only in `Cargo.toml`, is the intended distribution license.
- **Security reporting:** a repository owner must enable GitHub private
  vulnerability reporting or select another working private channel before a
  truthful `SECURITY.md` can be published.
- **Design partner:** a paid bank or bank-like partner must provide the frozen
  workflow, source, identity, policy/metric owners, reviewer, template,
  destination, scorecard, and commercial decision date.
- **Customer infrastructure:** production connector, identity provider,
  secret manager, database/object/queue topology, model, and deployment choices
  require the selected customer's environment and credentials.

No partner interview, customer acceptance, policy approval, regulatory
applicability decision, production qualification, pilot cycle, or commercial
decision is claimed without external evidence.

## Security, deployment, recovery, and release state

- Bank/customer systems remain read-only; no money movement, funding draw,
  collateral pledge, ledger/limit change, autonomous regulated decision,
  filing, or unreviewed external publication is authorized.
- The new pack trust boundary is asymmetric and tenant scoped. Production key
  custody, rotation/revocation, durable activation audit, and release signing
  are not yet complete.
- Evaluation installation and stopped-service backup exist. Restore,
  upgrade/rollback, export/uninstall, HA/PITR, signed-image, and clean-server
  release qualification remain open.

## Git state and next action

- Last committed baseline: `d44e971` (`Tailor MVP plan to banking finance`).
- Current increment: verified locally and ready for a scoped commit.
- Next recommended action: finish full qualification for this increment, commit
  and push it to `origin/finance`, then implement durable solution-pack
  activation and runtime routing.
