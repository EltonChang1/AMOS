# AMOS

AMOS is a governed analytical-memory runtime for LLM data agents. It keeps changing analytical state—schemas, metric definitions, snapshots or stream offsets, permissions, prior analyses, reviewer feedback, and provenance—outside the prompt as typed, versioned memory. A permission-first control loop selects context, reconciles conflicts, verifies computations, attaches claim-level support, and packages results for replay.

## Paper

- [Professor-review PDF](output/pdf/AMOS_paper.pdf)
- [PVLDB LaTeX source](venue/pvldb2027/AMOS_pvldb2027.tex)
- [Frozen external-evaluation protocol](evaluation_protocols/pvldb_v20_preregistration.json)

The paper is formatted for PVLDB Volume 20 and contains 10 content pages plus references.

## Results at a glance

| Evaluation | AMOS result |
|---|---:|
| Analytical-state invariant benchmark | 12/12 expected outcomes |
| Strongest local baseline | 6/12 expected outcomes |
| Multi-domain capability contract | 12/12 variants in each of 3 domains |
| Verifier regression corpus | 6/6 valid accepted; 10/10 invalid rejected |
| Seeded security suite | 6/6 probes passed |
| Claim extraction | 0.958 precision; 1.000 recall |
| Indexed memory scale | 1,000,001 memory objects |
| Indexed provenance scale | 1,000,000 edges |
| Permission and supersession leaks | 0 across archived retrieval studies |

The evaluation spans payment failure, subscription churn, and warehouse quality. Component ablations isolate the verifier, permission gate, and provenance recorder. Retrieval experiments compare governed BM25, pinned MiniLM/HNSW, and reciprocal-rank fusion at 1k, 10k, and 100k distractors. The frozen result tables and machine-readable outputs are under [`artifacts/evaluation/`](artifacts/evaluation/RESULTS.md).

## Quick start

Requires Python 3.11+.

```bash
python3 -m pip install -e ".[dev]"
python3 -m amos.memory.seed_memory
python3 -m amos.tools.seed_duckdb

python3 -m amos.agent.controller \
  --request "Why did payment failure rate increase over the last six hours, and should we update the executive dashboard?" \
  --user analyst_001 \
  --permissions analytics,payments
```

The controller returns a cited report, chart, verification result, warnings, and replay package.

## Run the evaluation

```bash
pytest -q
python3 -m amos.evaluation.run_eval \
  --all --samples 3 --scale-items 5000 \
  --run-dir /tmp/amos_eval
```

Run the complete paper bundle:

```bash
python3 -m amos.evaluation.paper_bundle --all \
  --run-dir /tmp/amos_paper_bundle \
  --variants 12 --samples 3 \
  --generated-tasks 120 --variant-seed 20260711 \
  --scale-items 5000 --concurrency 8 \
  --systems-scale-sizes 10000,100000,1000000 \
  --provider-mode offline
```

The deterministic offline provider makes the core experiment reproducible without API credentials. Provider-backed runs use the same verifier, provenance, replay, and raw-trace contracts.

## Build the paper

```bash
cd venue/pvldb2027
latexmk -pdf -interaction=nonstopmode -halt-on-error AMOS_pvldb2027.tex
```

## Repository map

- `amos/`: runtime, API, memory, verifier, provenance, tools, and evaluation code
- `tests/`: 85-test regression suite
- `scenarios/`: versioned three-domain scenario contracts
- `artifacts/evaluation/`: frozen aggregate results and retrieval/scale measurements
- `evaluation_protocols/`: preregistered external-validation design
- `external_studies/`: independent-study handoff and intake templates
- `venue/pvldb2027/`: paper source and official venue template

## Citation

```bibtex
@article{chang2027amos,
  title  = {AMOS: Enforcing Analytical-State Invariants for LLM Data Agents},
  author = {Chang, Elton},
  year   = {2027},
  note   = {Manuscript},
  url    = {https://github.com/EltonChang1/AMOS}
}
```
