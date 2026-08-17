# Governed Tool SDK

Status: **installable customer-evaluation catalog implemented**

AMOS does not give an AI agent a shell or unrestricted language runtime. An
agent may select only a versioned tool registered for the active solution pack.
The controller validates the contract, policy, parameters, source, and limits,
then issues a short-lived HMAC capability bound to the tenant, transaction,
plan, complete step hash, subject, tool, source, operations, relations, limits,
policy epoch, and fencing token.

## Installed catalog

| Tool | Runtime | Evaluation implementation |
| --- | --- | --- |
| `sql.readonly.v1` | Rust/SQLite | Source-connected parsed read-only SQL worker |
| `spark.dataframe.aggregate.v1` | PySpark 4.2 | Bounded inline DataFrame aggregation AST |
| `r.statistics.v1` | R | Allowlisted regression, Welch t-test, and chi-square methods |
| `python.dataframe.aggregate.v1` | pandas | Bounded inline DataFrame aggregation AST |
| `polars.dataframe.aggregate.v1` | Polars | Bounded inline DataFrame aggregation AST |
| `duckdb.readonly.v1` | DuckDB | One in-memory SELECT over `input_data`, external access disabled |
| `dbt.manifest.validate.v1` | Python | dbt manifest JSON structure and node validation; no macro execution |
| `stats.regression.v1` | scikit-learn | Ordinary least-squares regression |
| `stats.forecast.v1` | NumPy | Linear-trend forecast |
| `stats.pca.v1` | scikit-learn | Full-SVD principal component analysis |
| `spreadsheet.xlsx.v1` | openpyxl | Editable XLSX compiler without macros or external links |
| `presentation.pptx.v1` | python-pptx | Editable widescreen PPTX compiler |
| `notebook.inspect.v1` | nbformat | Cell metadata and code hashes; cells are never executed |
| `stats.rate_comparison.v1` | Rust | Embedded component used by the reference workflow |
| `chart.timeseries.v1` | Rust | Embedded SVG component used by the reference workflow |

The first thirteen entries are registered `plan_step` executors. The final two
remain `embedded_runtime` components and cannot be independently selected by a
plan. The external toolbox manifests are marked `evaluation_only: true`: they
are executable in the installable Compose package, but do not claim signed,
digest-pinned, production-qualified worker images.

This is the MVP catalog, not a claim to implement every package in the data
ecosystem. New engines still require an explicit bounded contract and trusted
executor registration.

## Execution boundary

The Compose package runs the control plane and toolbox as separate non-root
containers. Only the control plane can reach the toolbox on an internal Docker
network. The toolbox has:

- a read-only root filesystem, no published port, all Linux capabilities
  dropped, `no-new-privileges`, and PID/CPU/memory limits;
- the installation-specific capability key mounted read-only;
- no warehouse credentials, customer filesystem mounts, package installer
  input, arbitrary paths, URLs, code strings, JARs, R source, Python source, or
  Spark configuration in a tool contract;
- pinned evaluation package versions and runtime-version recording; and
- response row/byte accounting, controller timeouts, output-schema checks,
  content hashes, and normal `ExecutionRecord` provenance.

SQL is the only bundled source-connected analytical worker. Other tools consume
bounded inline tables or structured payloads produced by prior verified steps.
The dbt tool validates a manifest artifact; it does not run dbt models. The
notebook tool inspects notebook structure; arbitrary notebook execution remains
an explicit MVP non-goal.

## Manifest contract

Each JSON manifest defines:

- a stable tool ID ending in `.v<number>`;
- availability and an explicit `evaluation_only` qualification flag;
- runtime kind, capability audience, entry point, and optional pinned OCI
  image digest;
- allowed operations and read-only source behavior;
- strict parameter and output schemas;
- maximum time, rows, bytes, and memory;
- `deny_all` or `source_only` networking;
- determinism and seed requirements; and
- a verifier profile.

Parameter schemas use a deliberately bounded JSON Schema subset. It supports
objects, arrays, strings, integers, numbers, booleans, and a JSON `scalar`
type used only for cells in bounded inline tables. All parameter objects are
closed with `additionalProperties: false`, so an agent cannot smuggle an
undeclared shell command, package, network target, or runtime option into a
registered call.

## Operator commands

List all registered contracts:

```bash
amosctl tools list
```

Inspect one contract:

```bash
amosctl tools show --tool-id spark.dataframe.aggregate.v1
```

Authenticated product clients can read the public machine contract at
`GET /v1/tools`. It exposes schemas, availability, operations, limits, network
policy, determinism, and whether a plan-step implementation exists. It omits
capability audiences, entry points, image configuration, and deployment
secrets.

Validate inactive authoring examples without activating them:

```bash
amosctl tools validate \
  --manifest tool-packs/templates/spark.dataframe.aggregate.v1.json \
  --manifest tool-packs/templates/r.statistics.v1.json
```

On an installed Compose deployment, prove every external executor through the
same signed transport used by the runtime:

```bash
docker compose exec -T amos amosctl tools smoke \
  --endpoint toolbox:9000 \
  --capability-key-file /run/secrets/capability_key
```

`deploy/compose/diagnose.sh` runs that catalog-wide smoke check automatically.
The command reports output hashes, accounting, and exact runtime versions but
does not print the generated XLSX or PPTX bodies.

## Adding an executable tool

A manifest alone is insufficient. A new `plan_step` tool requires:

1. a trusted executor registered in the controller build;
2. strict, closed parameter and output schemas with bounded resources;
3. capability validation of every invocation binding, including the complete
   step hash;
4. default-deny credentials, filesystem, process, and network behavior;
5. deterministic or explicitly seeded execution with package versions;
6. output validation, hashes, evidence records, and an appropriate verifier;
7. cancellation and enforced resource limits appropriate to the runtime; and
8. tampering, isolation, retry, replay, and deployment conformance tests.

## Production qualification still required

The customer-evaluation toolbox closes the template-versus-executor gap, but
production qualification still requires:

- separate worker pools by tool and risk instead of one shared toolbox;
- immutable image digests, SBOMs, signatures, provenance attestations,
  vulnerability policy, and compatibility data;
- hard per-invocation CPU, memory, process, filesystem, network, and deadline
  enforcement with cancellation/reaping evidence;
- the selected customer warehouse connector and source-native authorization;
- solution-pack signing and activation controls; and
- workload-specific verifier corpora and replay qualification.
