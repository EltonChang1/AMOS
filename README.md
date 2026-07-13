# AMOS: AMOS: A Memory Operating Layer for Autonomous Data Analysis

AMOS is a governed memory and execution layer for autonomous data-analysis systems. It keeps schemas, metric definitions, data-state references, permissions, prior analyses, reviewer feedback, execution records, and claim provenance outside the model as typed, versioned state. Each task receives a bounded, permission-safe working set and passes through verification, publication, and replay controls.

## Papers

The repository intentionally contains exactly two PDF documents, both in [`papers/`](papers/), both in a single-column review format, and both carrying the same requested title.

- [Startup system design proposal](papers/AMOS_design_proposal.pdf) - a gated, step-by-step plan from a narrow design-partner workflow to a controlled-autonomy platform
- [Research paper](papers/AMOS_research_paper.pdf) - the full system abstraction, architecture, implementation, evaluation, limitations, and research context
- [Design proposal source](papers/design_proposal.tex)
- [Research paper source](papers/research_paper.tex)

## Quick start

AMOS requires Python 3.11 or newer.

```bash
python3 -m pip install -e ".[dev]"
python3 -m amos.memory.seed_memory
python3 -m amos.tools.seed_duckdb

python3 -m amos.agent.controller \
  --request "Why did payment failure rate increase over the last six hours?" \
  --user analyst_001 \
  --permissions analytics,payments
```

The controller returns a cited report, chart, verification result, warnings, and replay package.

## Validate the system

Run the regression suite:

```bash
pytest -q
```

Run the deterministic evaluation bundle outside the source tree:

```bash
python3 -m amos.evaluation.paper_bundle --all \
  --run-dir /tmp/amos_paper_bundle \
  --variants 12 --samples 3 \
  --generated-tasks 120 --variant-seed 20260711 \
  --scale-items 5000 --concurrency 8 \
  --systems-scale-sizes 10000,100000,1000000 \
  --provider-mode offline
```

Frozen aggregate results and machine-readable scale measurements are under [`artifacts/evaluation/`](artifacts/evaluation/RESULTS.md). The offline provider makes the core harness reproducible without API credentials; it is not evidence of live-model robustness.

## Build the papers

From the repository root:

```bash
latexmk -pdf -interaction=nonstopmode -halt-on-error \
  -jobname=AMOS_design_proposal \
  -outdir=papers papers/design_proposal.tex

latexmk -pdf -interaction=nonstopmode -halt-on-error \
  -jobname=AMOS_research_paper \
  -outdir=papers papers/research_paper.tex
```

LaTeX intermediates and all PDFs other than the two canonical paper files are ignored.

## Repository structure

```text
amos/                  Runtime, API, memory, verification, provenance, tools
artifacts/evaluation/  Small frozen aggregate evidence checked into Git
docs/evaluation/       Reproduction, evidence-intake, and validation guides
evaluation_protocols/  Frozen preregistration material
external_studies/      Independent-study handoff and manifest templates
papers/                The two canonical PDFs and their LaTeX sources
scenarios/             Versioned analytical scenario contracts
tests/                 Regression and evaluation tests
```

Generated databases, raw experiment runs, charts, reports, caches, and LaTeX build products are intentionally excluded from the repository.

## Evidence snapshot

| Evaluation | AMOS result |
| --- | ---: |
| Analytical-state invariant benchmark | 12/12 expected outcomes |
| Strongest local baseline | 6/12 expected outcomes |
| Multi-domain capability contract | 12/12 variants in each of 3 domains |
| Verifier regression corpus | 6/6 valid accepted; 10/10 invalid rejected |
| Seeded security suite | 6/6 probes passed |
| Claim extraction | 0.958 precision; 1.000 recall |
| Indexed memory scale | 1,000,001 memory objects |
| Indexed provenance scale | 1,000,000 edges |

These are development and engineering results. The limits on independent and deployed-product evidence are documented in [`docs/evaluation/independent_evaluation_protocol.md`](docs/evaluation/independent_evaluation_protocol.md).

## Citation

```bibtex
@article{chang2026amos,
  title  = {AMOS: AMOS: A Memory Operating Layer for Autonomous Data Analysis},
  author = {Chang, Elton},
  year   = {2026},
  note   = {Manuscript},
  url    = {https://github.com/EltonChang1/AMOS}
}
```
