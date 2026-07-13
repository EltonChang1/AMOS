# AMOS: A Memory Operating Layer for Autonomous Data Analysis

- [Design proposal](papers/AMOS_design_proposal.pdf) - An end-to-end product architecture and implementation guide with concrete contracts for transactions, data, context, connectors, workers, claims, security, and phased delivery.
- [Research paper](papers/AMOS_research_paper.pdf) - A complete introduction to AMOS covering the system model, memory architecture, implementation, evaluation, limitations, and broader research vision.

## MVP workspace

Requires Python 3.11+.

```bash
python3 -m pip install -e ".[dev]"
python3 -m amos.memory.seed_memory
python3 -m amos.tools.seed_duckdb
python3 -m uvicorn amos.api.main:app --reload
```

Open [http://127.0.0.1:8000](http://127.0.0.1:8000) for the AMOS MVP workspace. The product flow runs a governed payment-failure investigation, shows the verified report and chart, exposes claim-level evidence, saves reviewer feedback as memory, and can replay the recorded analysis.

The command-line workflow remains available:

```bash
python3 -m amos.agent.controller \
  --request "Why did payment failure rate increase over the last six hours, and should we update the executive dashboard?" \
  --user analyst_001 \
  --permissions analytics,payments
```

Run the regression suite with `pytest -q`.
