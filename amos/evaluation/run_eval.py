from __future__ import annotations

import argparse
import json
from pathlib import Path

from amos.config import settings
from amos.evaluation.benchmark import run_benchmark_suite
from amos.evaluation.extended_experiments import run_extended_experiments


def run_all(samples: int = 3, scale_items: int = 5000, run_dir: str | Path | None = None) -> dict[str, object]:
    if run_dir is not None:
        settings.use_run_dir(run_dir)
    return run_benchmark_suite(samples=samples, scale_items=scale_items)


def run_extended(
    scale_items: int = 5000,
    concurrency: int = 8,
    llm_samples: int = 3,
    claim_items: int = 1000,
    run_dir: str | Path | None = None,
) -> dict[str, object]:
    if run_dir is not None:
        settings.use_run_dir(run_dir)
    return run_extended_experiments(
        scale_items=scale_items,
        concurrency=concurrency,
        llm_samples=llm_samples,
        claim_items=claim_items,
        write_artifacts=True,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Run AMOS benchmark scenarios.")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--scenario", default="benchmark_suite")
    parser.add_argument("--samples", default=3, type=int)
    parser.add_argument("--scale-items", default=5000, type=int)
    parser.add_argument("--extended", action="store_true")
    parser.add_argument("--concurrency", default=8, type=int)
    parser.add_argument("--llm-samples", default=3, type=int)
    parser.add_argument("--claim-items", default=1000, type=int)
    parser.add_argument("--run-dir", default=None, help="Use an isolated run directory for data and artifacts.")
    args = parser.parse_args()
    if args.extended or args.scenario == "extended_experiments":
        print(
            json.dumps(
                run_extended(
                    scale_items=max(args.scale_items, 0),
                    concurrency=max(args.concurrency, 1),
                    llm_samples=max(args.llm_samples, 1),
                    claim_items=max(args.claim_items, 4),
                    run_dir=args.run_dir,
                ),
                indent=2,
                sort_keys=True,
            )
        )
    elif args.all or args.scenario == "benchmark_suite":
        print(
            json.dumps(
                run_all(samples=max(args.samples, 1), scale_items=max(args.scale_items, 0), run_dir=args.run_dir),
                indent=2,
                sort_keys=True,
            )
        )
    else:
        raise SystemExit(f"Unknown scenario: {args.scenario}")


if __name__ == "__main__":
    main()
