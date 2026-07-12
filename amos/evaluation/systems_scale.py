"""Local indexed-memory and provenance scaling benchmark.

The benchmark is intentionally descriptive. It measures the SQLite/FTS5
prototype on one machine and does not stand in for a distributed production
deployment.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import resource
import sqlite3
import statistics
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

from amos.memory.models import MemoryObject, RetrieveRequest
from amos.memory.retrieval import retrieve
from amos.memory.store import MemoryStore


WINDOW = (
    datetime.fromisoformat("2026-07-07T14:00:00+00:00"),
    datetime.fromisoformat("2026-07-07T20:00:00+00:00"),
)
ANALYST_PERMISSIONS = ["analytics", "payments"]
TARGET_ID = "memory_metric_payment_failure_rate_v3"


def run_systems_scale_experiment(
    *,
    memory_items: int = 100_000,
    readers: int = 8,
    mixed_writes: int = 64,
    provenance_edges: int = 100_000,
    retrieval_repeats: int = 30,
    output_dir: str | Path | None = None,
) -> dict[str, Any]:
    """Run an isolated local systems experiment and optionally archive results."""

    started = datetime.now(timezone.utc)
    with tempfile.TemporaryDirectory(prefix="amos-systems-scale-") as directory:
        db_path = Path(directory) / "systems_scale.sqlite"
        store = MemoryStore(db_path)
        store.reset()
        store.upsert_memory(_target_metric())

        seed_start = time.perf_counter()
        inserted = store.bulk_upsert_memory(
            (_distractor(index) for index in range(max(memory_items, 0))),
            batch_size=5_000,
        )
        seed_seconds = time.perf_counter() - seed_start
        counts = _database_counts(store)

        serial = _repeat(retrieval_repeats, lambda: _retrieve_target(store))
        concurrent_reads = _concurrent_reads(store, readers=max(readers, 1))
        update_consistency = {
            "permission_revocation": _permission_revocation_probe(store),
            "metric_supersession": _supersession_probe(store),
        }
        mixed = _mixed_read_write_probe(store, readers=max(readers, 1), writes=max(mixed_writes, 0))
        provenance = _provenance_growth_probe(store, max(provenance_edges, 0))
        storage_bytes = _database_file_bytes(db_path)

        result: dict[str, Any] = {
            "schema_version": "amos.systems_scale.v1",
            "status": "completed",
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "statistical_scope": (
                "Descriptive measurements over repeated operations on one local database; "
                "operations are not independent task samples and no population inference is made."
            ),
            "configuration": {
                "requested_memory_distractors": memory_items,
                "readers": readers,
                "mixed_writes": mixed_writes,
                "requested_provenance_edges": provenance_edges,
                "retrieval_repeats": retrieval_repeats,
                "retrieval_engine": "SQLite FTS5 BM25 candidate generation plus governed reranking",
            },
            "environment": _environment(),
            "memory_scale": {
                "distractors_inserted": inserted,
                "total_memory_objects": counts["memory_objects"],
                "fts_rows": counts["fts_rows"],
                "index_synchronized": counts["memory_objects"] == counts["fts_rows"],
                "seed_seconds": round(seed_seconds, 6),
                "seed_objects_per_second": round(inserted / seed_seconds, 2) if seed_seconds else None,
                "database_bytes_after_all_probes": storage_bytes,
            },
            "serial_retrieval": serial,
            "concurrent_reads": concurrent_reads,
            "update_consistency": update_consistency,
            "mixed_read_write": mixed,
            "provenance_growth": provenance,
            "wall_seconds": round((datetime.now(timezone.utc) - started).total_seconds(), 6),
            "claim_boundary": (
                "This is evidence for correctness and local FTS5 scaling only. It does not establish "
                "distributed consistency, hosted-product performance, vector/hybrid retrieval quality, "
                "or production-scale robustness."
            ),
        }

    if output_dir is not None:
        _write_artifacts(result, Path(output_dir))
    return result


def _target_metric() -> MemoryObject:
    return MemoryObject(
        id=TARGET_ID,
        type="semantic_definition",
        summary="Approved payment failure rate for production payment attempts excluding test accounts.",
        content={
            "name": "payment_failure_rate",
            "required_filters": ["environment = 'production'", "is_test_account = false"],
            "time_field": "event_time",
        },
        source="semantic_layer",
        authority="owner_approved",
        effective_start=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
        permissions=ANALYST_PERMISSIONS,
        version="v3",
        status="active",
    )


def _distractor(index: int) -> MemoryObject:
    return MemoryObject(
        id=f"memory_scale_metric_{index:08d}",
        type="semantic_definition",
        summary=f"Warehouse utilization metric {index} for unrelated operational analytics.",
        content={
            "name": f"warehouse_utilization_{index}",
            "required_filters": ["environment = 'production'"],
            "time_field": "event_time",
        },
        source="semantic_layer",
        authority="owner_approved" if index % 7 == 0 else "user_note",
        effective_start=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
        permissions=ANALYST_PERMISSIONS,
        version="v1",
        status="active",
    )


def _request(text: str, *, permissions: list[str] | None = None, required_types: list[str] | None = None) -> RetrieveRequest:
    return RetrieveRequest(
        task_text=text,
        required_types=required_types or [],
        time_range=WINDOW,
        user_permissions=permissions or ANALYST_PERMISSIONS,
        max_items=12,
    )


def _retrieve_target(store: MemoryStore) -> dict[str, Any]:
    result = retrieve(
        _request(
            "approved payment failure rate production test accounts event time",
            required_types=["semantic_definition"],
        ),
        store,
    )
    ids = [item.id for item in result.items]
    return {
        "passed": bool(ids and ids[0] == TARGET_ID),
        "target_rank": ids.index(TARGET_ID) + 1 if TARGET_ID in ids else None,
        "returned": len(ids),
    }


def _repeat(count: int, operation: Callable[[], dict[str, Any]]) -> dict[str, Any]:
    rows: list[dict[str, Any]] = []
    for _ in range(max(count, 1)):
        start = time.perf_counter()
        row = operation()
        rows.append({**row, "latency_seconds": time.perf_counter() - start})
    latencies = sorted(float(row["latency_seconds"]) for row in rows)
    return {
        "runs": len(rows),
        "passed": sum(1 for row in rows if row.get("passed")),
        "p50_latency_seconds": round(_percentile(latencies, 0.50), 6),
        "p95_latency_seconds": round(_percentile(latencies, 0.95), 6),
        "p99_latency_seconds": round(_percentile(latencies, 0.99), 6),
        "max_latency_seconds": round(max(latencies), 6),
        "target_rank": rows[-1].get("target_rank"),
    }


def _concurrent_reads(store: MemoryStore, *, readers: int) -> dict[str, Any]:
    operations = readers * 8
    latencies: list[float] = []
    passed = 0
    errors: list[str] = []

    def one() -> tuple[float, bool]:
        start = time.perf_counter()
        outcome = _retrieve_target(store)
        return time.perf_counter() - start, bool(outcome["passed"])

    wall_start = time.perf_counter()
    with ThreadPoolExecutor(max_workers=readers) as executor:
        futures = [executor.submit(one) for _ in range(operations)]
        for future in as_completed(futures):
            try:
                latency, ok = future.result()
                latencies.append(latency)
                passed += int(ok)
            except Exception as exc:  # pragma: no cover - archived as evidence
                errors.append(repr(exc))
    wall = time.perf_counter() - wall_start
    latencies.sort()
    return {
        "workers": readers,
        "operations": operations,
        "completed": len(latencies),
        "passed": passed,
        "errors": errors,
        "throughput_operations_per_second": round(len(latencies) / wall, 2) if wall else None,
        "p50_latency_seconds": round(_percentile(latencies, 0.50), 6) if latencies else None,
        "p95_latency_seconds": round(_percentile(latencies, 0.95), 6) if latencies else None,
        "p99_latency_seconds": round(_percentile(latencies, 0.99), 6) if latencies else None,
    }


def _permission_revocation_probe(store: MemoryStore) -> dict[str, Any]:
    item = MemoryObject(
        id="memory_permission_churn_quartz",
        type="prior_analysis",
        summary="Quartz payment retry evidence for permission revocation testing.",
        content={"finding": "Quartz retry evidence."},
        source="incident_archive",
        authority="reviewer_approved",
        effective_start=WINDOW[0],
        permissions=ANALYST_PERMISSIONS,
        version="v1",
        status="active",
    )
    store.upsert_memory(item)
    request = _request("quartz payment retry evidence", required_types=["prior_analysis"])
    before = retrieve(request, store)
    start = time.perf_counter()
    store.upsert_memory(item.model_copy(update={"permissions": [*ANALYST_PERMISSIONS, "sre"], "version": "v2"}))
    after = retrieve(request, store)
    latency = time.perf_counter() - start
    return {
        "passed": item.id in {entry.id for entry in before.items}
        and item.id not in {entry.id for entry in after.items}
        and item.id in after.filtered_permission_ids,
        "revocation_and_observation_seconds": round(latency, 6),
        "before_visible": item.id in {entry.id for entry in before.items},
        "after_visible": item.id in {entry.id for entry in after.items},
        "after_filtered": item.id in after.filtered_permission_ids,
    }


def _supersession_probe(store: MemoryStore) -> dict[str, Any]:
    old = MemoryObject(
        id="memory_metric_zephyr_latency_v1",
        type="semantic_definition",
        summary="Zephyr latency metric definition version one.",
        content={"name": "zephyr_latency", "time_field": "processing_time"},
        source="semantic_layer",
        authority="owner_approved",
        effective_start=WINDOW[0],
        permissions=ANALYST_PERMISSIONS,
        version="v1",
        status="active",
    )
    new = old.model_copy(
        update={
            "id": "memory_metric_zephyr_latency_v2",
            "summary": "Zephyr latency metric definition version two using event time.",
            "content": {"name": "zephyr_latency", "time_field": "event_time"},
            "version": "v2",
        }
    )
    store.upsert_memory(old)
    store.upsert_memory(new)
    start = time.perf_counter()
    store.supersede(old.id, new.id)
    result = retrieve(
        _request("zephyr latency metric event time", required_types=["semantic_definition"]),
        store,
    )
    latency = time.perf_counter() - start
    ids = [entry.id for entry in result.items]
    return {
        "passed": new.id in ids and old.id not in ids,
        "supersession_and_observation_seconds": round(latency, 6),
        "returned_ids": ids,
    }


def _mixed_read_write_probe(store: MemoryStore, *, readers: int, writes: int) -> dict[str, Any]:
    read_operations = readers * 4
    tasks: list[tuple[str, int]] = [("read", index) for index in range(read_operations)]
    tasks.extend(("write", index) for index in range(writes))

    def one(kind: str, index: int) -> dict[str, Any]:
        start = time.perf_counter()
        if kind == "read":
            passed = bool(_retrieve_target(store)["passed"])
        else:
            store.upsert_memory(
                MemoryObject(
                    id=f"memory_mixed_write_{index:06d}",
                    type="feedback",
                    summary=f"Mixed workload feedback record {index}.",
                    content={"feedback": f"Observe mixed workload record {index}."},
                    source="reviewer",
                    authority="reviewer_approved",
                    effective_start=WINDOW[0],
                    permissions=ANALYST_PERMISSIONS,
                    version="v1",
                    status="active",
                )
            )
            passed = True
        return {"kind": kind, "passed": passed, "latency_seconds": time.perf_counter() - start}

    rows: list[dict[str, Any]] = []
    errors: list[str] = []
    workers = max(readers + min(2, writes), 1)
    wall_start = time.perf_counter()
    with ThreadPoolExecutor(max_workers=workers) as executor:
        futures = [executor.submit(one, kind, index) for kind, index in tasks]
        for future in as_completed(futures):
            try:
                rows.append(future.result())
            except Exception as exc:  # pragma: no cover - archived as evidence
                errors.append(repr(exc))
    wall = time.perf_counter() - wall_start
    return {
        "workers": workers,
        "read_operations": read_operations,
        "write_operations": writes,
        "completed": len(rows),
        "passed": sum(1 for row in rows if row["passed"]),
        "errors": errors,
        "throughput_operations_per_second": round(len(rows) / wall, 2) if wall else None,
        "read_latency": _latency_summary([row["latency_seconds"] for row in rows if row["kind"] == "read"]),
        "write_latency": _latency_summary([row["latency_seconds"] for row in rows if row["kind"] == "write"]),
    }


def _provenance_growth_probe(store: MemoryStore, edge_count: int) -> dict[str, Any]:
    targets = 100
    target_id = "artifact_scale_000"
    start = time.perf_counter()
    inserted = store.bulk_add_edges(
        (
            (
                f"claim_scale_{index:09d}",
                f"artifact_scale_{index % targets:03d}",
                "supported_by",
                {"query_id": f"query_{index:09d}"},
            )
            for index in range(edge_count)
        ),
        batch_size=5_000,
    )
    insert_seconds = time.perf_counter() - start

    query_latencies = []
    returned = 0
    for _ in range(30):
        query_start = time.perf_counter()
        returned = len(store.list_edges_for_target(target_id, limit=100))
        query_latencies.append(time.perf_counter() - query_start)
    counts = _database_counts(store)
    return {
        "edges_inserted": inserted,
        "total_edges": counts["provenance_edges"],
        "insert_seconds": round(insert_seconds, 6),
        "insert_edges_per_second": round(inserted / insert_seconds, 2) if insert_seconds else None,
        "target_query_returned": returned,
        "target_query_latency": _latency_summary(query_latencies),
    }


def _latency_summary(values: list[float]) -> dict[str, float | None]:
    if not values:
        return {"p50_seconds": None, "p95_seconds": None, "p99_seconds": None, "max_seconds": None}
    ordered = sorted(values)
    return {
        "p50_seconds": round(_percentile(ordered, 0.50), 6),
        "p95_seconds": round(_percentile(ordered, 0.95), 6),
        "p99_seconds": round(_percentile(ordered, 0.99), 6),
        "max_seconds": round(max(ordered), 6),
    }


def _database_counts(store: MemoryStore) -> dict[str, int]:
    with store.connect() as conn:
        return {
            "memory_objects": int(conn.execute("SELECT COUNT(*) FROM memory_objects").fetchone()[0]),
            "fts_rows": int(conn.execute("SELECT COUNT(*) FROM memory_fts").fetchone()[0]),
            "provenance_edges": int(conn.execute("SELECT COUNT(*) FROM provenance_edges").fetchone()[0]),
        }


def _database_file_bytes(path: Path) -> int:
    return sum(
        candidate.stat().st_size
        for candidate in [path, path.with_name(f"{path.name}-wal"), path.with_name(f"{path.name}-shm")]
        if candidate.exists()
    )


def _environment() -> dict[str, Any]:
    max_rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    # macOS reports bytes; Linux and most BSDs report KiB.
    max_resident_set_bytes = int(max_rss if platform.system() == "Darwin" else max_rss * 1024)
    return {
        "platform": platform.platform(),
        "python": platform.python_version(),
        "sqlite": sqlite3.sqlite_version,
        "cpu_count": os.cpu_count(),
        "machine": platform.machine(),
        "max_resident_set_bytes": max_resident_set_bytes,
    }


def _percentile(values: list[float], quantile: float) -> float:
    if not values:
        return 0.0
    if len(values) == 1:
        return values[0]
    position = (len(values) - 1) * quantile
    lower = int(position)
    upper = min(lower + 1, len(values) - 1)
    fraction = position - lower
    return values[lower] * (1 - fraction) + values[upper] * fraction


def _write_artifacts(result: dict[str, Any], output_dir: Path) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    results_path = output_dir / "results.json"
    summary_path = output_dir / "summary.md"
    payload = json.dumps(result, indent=2, sort_keys=True)
    results_path.write_text(payload, encoding="utf-8")
    digest = hashlib.sha256(payload.encode("utf-8")).hexdigest()
    (output_dir / "results.sha256").write_text(f"{digest}  results.json\n", encoding="utf-8")
    serial = result["serial_retrieval"]
    concurrent = result["concurrent_reads"]
    provenance = result["provenance_growth"]
    summary_path.write_text(
        "\n".join(
            [
                "# AMOS Local Systems-Scale Experiment",
                "",
                f"- Memory objects: {result['memory_scale']['total_memory_objects']}",
                f"- FTS synchronized: {result['memory_scale']['index_synchronized']}",
                f"- Serial retrieval p50/p95/p99: {serial['p50_latency_seconds']} / {serial['p95_latency_seconds']} / {serial['p99_latency_seconds']} s",
                f"- Concurrent reads: {concurrent['completed']}/{concurrent['operations']} completed, p95 {concurrent['p95_latency_seconds']} s",
                f"- Permission revocation observed: {result['update_consistency']['permission_revocation']['passed']}",
                f"- Supersession observed: {result['update_consistency']['metric_supersession']['passed']}",
                f"- Mixed workload errors: {len(result['mixed_read_write']['errors'])}",
                f"- Provenance edges: {provenance['total_edges']}, target-query p95 {provenance['target_query_latency']['p95_seconds']} s",
                "",
                f"Claim boundary: {result['claim_boundary']}",
                "",
            ]
        ),
        encoding="utf-8",
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Run AMOS local indexed systems-scale measurements.")
    parser.add_argument("--memory-items", type=int, default=100_000)
    parser.add_argument("--readers", type=int, default=8)
    parser.add_argument("--mixed-writes", type=int, default=64)
    parser.add_argument("--provenance-edges", type=int, default=100_000)
    parser.add_argument("--retrieval-repeats", type=int, default=30)
    parser.add_argument("--output-dir", default="artifacts/evaluation/systems_scale")
    args = parser.parse_args()
    result = run_systems_scale_experiment(
        memory_items=max(args.memory_items, 0),
        readers=max(args.readers, 1),
        mixed_writes=max(args.mixed_writes, 0),
        provenance_edges=max(args.provenance_edges, 0),
        retrieval_repeats=max(args.retrieval_repeats, 1),
        output_dir=args.output_dir,
    )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
