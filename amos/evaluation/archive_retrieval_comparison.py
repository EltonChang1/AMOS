"""Validate and archive pre-executed governed retrieval-engine comparisons."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Sequence


REQUIRED_ENGINES = {"bm25_governed", "minilm_hnsw_governed", "rrf_hybrid_governed"}


def archive_retrieval_comparisons(
    source_dirs: Sequence[str | Path],
    output_dir: str | Path,
) -> dict[str, Any]:
    if not source_dirs:
        raise ValueError("At least one retrieval-comparison directory is required.")
    output = Path(output_dir).resolve()
    output.mkdir(parents=True, exist_ok=True)
    archived_runs: list[dict[str, Any]] = []
    files: list[dict[str, Any]] = []
    seen_scales: set[int] = set()
    for source_value in source_dirs:
        source = Path(source_value).resolve()
        results_source = source / "results.json"
        summary_source = source / "summary.md"
        sha_source = source / "results.sha256"
        for required in [results_source, summary_source, sha_source]:
            if not required.exists() or not required.is_file():
                raise FileNotFoundError(f"Retrieval-comparison artifact is missing: {required}")
        payload = json.loads(results_source.read_text(encoding="utf-8"))
        _validate_payload(payload)
        expected_sha = sha_source.read_text(encoding="utf-8").split()[0]
        actual_sha = hashlib.sha256(results_source.read_bytes()).hexdigest()
        if expected_sha != actual_sha:
            raise ValueError(f"Retrieval-comparison result hash mismatch: {results_source}")
        scale = int(payload["configuration"]["distractors_requested"])
        if scale in seen_scales:
            raise ValueError(f"Duplicate retrieval-comparison scale: {scale}")
        seen_scales.add(scale)
        label = _scale_label(scale)
        destination = output / label
        destination.mkdir(parents=True, exist_ok=True)
        for source_file in [results_source, summary_source, sha_source]:
            copied = destination / source_file.name
            shutil.copy2(source_file, copied)
            files.append(_file_record(copied, output))
        archived_runs.append(
            {
                "label": label,
                "distractors": scale,
                "source_directory": str(source),
                "results_sha256": actual_sha,
                "query_count": payload["configuration"]["query_count"],
                "repeats": payload["configuration"]["repeats"],
                "vector_model": payload["configuration"]["vector_model"],
                "vector_model_revision": payload["configuration"]["vector_model_revision"],
                "aggregate": payload["aggregate"],
                "governance_update_probes": payload["governance_update_probes"],
            }
        )

    archived_runs.sort(key=lambda run: int(run["distractors"]))
    manifest = {
        "archive_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "run_count": len(archived_runs),
        "runs": archived_runs,
        "files": files,
        "evidence_boundary": (
            "Pre-executed, internally authored synthetic retrieval cases with templated distractors. "
            "Archival validates integrity and schema; it does not convert the runs into independent, "
            "distributed, production, or external-product evidence."
        ),
    }
    (output / "archive_manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return manifest


def _validate_payload(payload: dict[str, Any]) -> None:
    if payload.get("schema_version") != "amos.retrieval_engine_comparison.v1":
        raise ValueError("Unsupported retrieval-comparison schema version.")
    if payload.get("status") != "completed":
        raise ValueError("Only completed retrieval comparisons can be archived.")
    configuration = payload.get("configuration", {})
    for field in ["distractors_requested", "query_count", "repeats", "vector_model", "vector_model_revision"]:
        if field not in configuration:
            raise ValueError(f"Retrieval-comparison configuration is missing: {field}")
    aggregate = payload.get("aggregate", {})
    missing_engines = sorted(REQUIRED_ENGINES - set(aggregate))
    if missing_engines:
        raise ValueError(f"Retrieval-comparison aggregate is missing engines: {missing_engines}")
    for engine in REQUIRED_ENGINES:
        metrics = aggregate[engine]
        for metric in [
            "top1_accuracy",
            "recall_at_5",
            "mean_reciprocal_rank",
            "p50_latency_seconds",
            "p95_latency_seconds",
            "permission_leak_count",
            "superseded_leak_count",
        ]:
            if metric not in metrics:
                raise ValueError(f"Retrieval-comparison {engine} is missing metric: {metric}")
    probes = payload.get("governance_update_probes", {})
    for probe in ["permission_revocation", "metric_supersession"]:
        if probe not in probes or "passed" not in probes[probe]:
            raise ValueError(f"Retrieval-comparison is missing governance probe: {probe}")


def _file_record(path: Path, root: Path) -> dict[str, Any]:
    data = path.read_bytes()
    return {
        "path": str(path.relative_to(root)),
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def _scale_label(scale: int) -> str:
    if scale >= 1_000_000 and scale % 1_000_000 == 0:
        return f"{scale // 1_000_000}m"
    if scale >= 1_000 and scale % 1_000 == 0:
        return f"{scale // 1_000}k"
    return str(scale)


def main() -> None:
    parser = argparse.ArgumentParser(description="Archive governed retrieval-engine comparison results.")
    parser.add_argument("source_dir", nargs="+")
    parser.add_argument("output_dir")
    args = parser.parse_args()
    print(json.dumps(archive_retrieval_comparisons(args.source_dir, args.output_dir), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
