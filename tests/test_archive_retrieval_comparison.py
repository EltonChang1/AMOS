from __future__ import annotations

import hashlib
import json

import pytest

from amos.evaluation.archive_retrieval_comparison import archive_retrieval_comparisons


def _write_run(path, scale: int) -> None:
    path.mkdir(parents=True)
    metrics = {
        "top1_accuracy": 0.5,
        "recall_at_5": 0.75,
        "mean_reciprocal_rank": 0.6,
        "p50_latency_seconds": 0.01,
        "p95_latency_seconds": 0.02,
        "permission_leak_count": 0,
        "superseded_leak_count": 0,
    }
    payload = {
        "schema_version": "amos.retrieval_engine_comparison.v1",
        "status": "completed",
        "configuration": {
            "distractors_requested": scale,
            "query_count": 24,
            "repeats": 1,
            "vector_model": "model-a",
            "vector_model_revision": "revision-a",
        },
        "aggregate": {
            "bm25_governed": metrics,
            "minilm_hnsw_governed": metrics,
            "rrf_hybrid_governed": metrics,
        },
        "governance_update_probes": {
            "permission_revocation": {"passed": True},
            "metric_supersession": {"passed": True},
        },
    }
    rendered = json.dumps(payload, indent=2, sort_keys=True) + "\n"
    (path / "results.json").write_text(rendered, encoding="utf-8")
    digest = hashlib.sha256(rendered.encode("utf-8")).hexdigest()
    (path / "results.sha256").write_text(f"{digest}  results.json\n", encoding="utf-8")
    (path / "summary.md").write_text("# Summary\n", encoding="utf-8")


def test_archive_retrieval_comparisons_preserves_scales_and_hashes(tmp_path) -> None:
    first = tmp_path / "first"
    second = tmp_path / "second"
    _write_run(first, 1_000)
    _write_run(second, 10_000)

    output = tmp_path / "archive"
    manifest = archive_retrieval_comparisons([second, first], output)

    assert [run["distractors"] for run in manifest["runs"]] == [1_000, 10_000]
    assert len(manifest["files"]) == 6
    assert (output / "1k" / "results.json").exists()
    assert (output / "10k" / "results.json").exists()


def test_archive_retrieval_comparison_rejects_hash_mismatch(tmp_path) -> None:
    source = tmp_path / "source"
    _write_run(source, 1_000)
    (source / "results.sha256").write_text("bad  results.json\n", encoding="utf-8")

    with pytest.raises(ValueError, match="hash mismatch"):
        archive_retrieval_comparisons([source], tmp_path / "archive")
