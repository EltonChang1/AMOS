# AMOS tool packs

`builtin/` contains contracts implemented directly by the trusted AMOS binary.
`executable/` contains the installed, capability-bound toolbox contracts for
Spark, R, pandas, Polars, DuckDB, dbt metadata validation, statistical models,
XLSX, PPTX, and safe notebook inspection.

`templates/` contains inactive authoring examples. Validating or copying a
template never activates it; activation requires a controller implementation.
The corresponding Spark and R executables are separately registered from
`executable/`, so the inactive examples do not create a runtime gap.

External manifests are marked `evaluation_only: true`: the installable Compose
package builds and runs them, but release image digests, signatures, SBOMs, and
production isolation qualification are still required before a pilot claim.

See [the Governed Tool SDK guide](../docs/GOVERNED_TOOL_SDK.md) for the manifest
contract, validation commands, and executor promotion requirements.
