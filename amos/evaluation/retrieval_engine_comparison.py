"""Governed BM25, semantic-vector, and hybrid retrieval comparison.

This is an internally authored, single-machine engineering benchmark. It uses
templated distractors and must not be represented as production relevance or
external-product evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import statistics
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Protocol, Sequence

import numpy as np

from amos.memory.models import MemoryObject, RetrieveRequest
from amos.memory.permissions import has_required_permissions
from amos.memory.retrieval import retrieve
from amos.memory.store import MemoryStore


MODEL_ID = "sentence-transformers/all-MiniLM-L6-v2"
MODEL_REVISION = "1110a243fdf4706b3f48f1d95db1a4f5529b4d41"
WINDOW = (
    datetime.fromisoformat("2026-07-07T14:00:00+00:00"),
    datetime.fromisoformat("2026-07-07T20:00:00+00:00"),
)
ANALYST_PERMISSIONS = ["analytics", "payments"]


class Embedder(Protocol):
    model_id: str
    revision: str
    dimension: int

    def encode(self, texts: Sequence[str]) -> np.ndarray:
        """Return row-normalized float32 embeddings."""


class MiniLMEmbedder:
    model_id = MODEL_ID
    revision = MODEL_REVISION

    def __init__(self, *, batch_size: int = 128) -> None:
        import torch
        from transformers import AutoModel, AutoTokenizer

        self._torch = torch
        self._tokenizer = AutoTokenizer.from_pretrained(self.model_id, revision=self.revision)
        self._model = AutoModel.from_pretrained(self.model_id, revision=self.revision)
        self._model.eval()
        self.batch_size = max(batch_size, 1)
        self.dimension = int(self._model.config.hidden_size)

    def encode(self, texts: Sequence[str]) -> np.ndarray:
        rows: list[np.ndarray] = []
        with self._torch.inference_mode():
            for start in range(0, len(texts), self.batch_size):
                batch = list(texts[start : start + self.batch_size])
                encoded = self._tokenizer(
                    batch,
                    padding=True,
                    truncation=True,
                    max_length=256,
                    return_tensors="pt",
                )
                output = self._model(**encoded).last_hidden_state
                mask = encoded["attention_mask"].unsqueeze(-1).expand(output.size()).float()
                pooled = (output * mask).sum(1) / mask.sum(1).clamp(min=1e-9)
                pooled = self._torch.nn.functional.normalize(pooled, p=2, dim=1)
                rows.append(pooled.cpu().numpy().astype("float32"))
        if not rows:
            return np.empty((0, self.dimension), dtype="float32")
        return np.concatenate(rows, axis=0)


@dataclass(frozen=True)
class QueryCase:
    query_id: str
    text: str
    target_id: str
    query_family: str


class HNSWMemoryIndex:
    def __init__(
        self,
        items: Sequence[MemoryObject],
        embedder: Embedder,
        *,
        ef_construction: int = 200,
        search_ef: int = 128,
        m: int = 16,
        random_seed: int = 20260712,
    ) -> None:
        try:
            import hnswlib
        except ImportError as exc:  # pragma: no cover - explicit optional dependency failure
            raise RuntimeError("Install the semantic-eval optional dependencies to run HNSW retrieval.") from exc

        self.embedder = embedder
        self.items = {item.id: item for item in items}
        self.ids = [item.id for item in items]
        texts = [_memory_text(item) for item in items]
        unique_texts = list(dict.fromkeys(texts))
        unique_vectors = embedder.encode(unique_texts)
        vector_by_text = {text: unique_vectors[index] for index, text in enumerate(unique_texts)}
        vectors = np.asarray([vector_by_text[text] for text in texts], dtype="float32")
        if len(vectors) != len(items) or vectors.ndim != 2:
            raise ValueError("Embedding backend returned an invalid matrix shape.")
        self.dimension = int(vectors.shape[1])
        self.index = hnswlib.Index(space="cosine", dim=self.dimension)
        self.index.init_index(
            max_elements=max(len(items), 1),
            ef_construction=max(ef_construction, 8),
            M=max(m, 4),
            random_seed=random_seed,
        )
        if items:
            self.index.add_items(vectors, np.arange(len(items), dtype=np.int64))
        self.index.set_ef(max(search_ef, 8))

    def replace_metadata(self, item: MemoryObject) -> None:
        if item.id not in self.items:
            raise KeyError(item.id)
        self.items[item.id] = item

    def search(self, request: RetrieveRequest, *, limit: int, candidate_k: int = 128) -> list[str]:
        if not self.ids:
            return []
        query_vector = self.embedder.encode([request.task_text])
        count = min(max(candidate_k, limit), len(self.ids))
        labels, _distances = self.index.knn_query(query_vector, k=count)
        ranked: list[str] = []
        for label in labels[0].tolist():
            memory_id = self.ids[int(label)]
            item = self.items[memory_id]
            if _eligible(item, request):
                ranked.append(memory_id)
            if len(ranked) >= limit:
                break
        return ranked

    def save(self, path: str | Path) -> None:
        self.index.save_index(str(path))


def run_retrieval_engine_comparison(
    *,
    distractors: int = 10_000,
    repeats: int = 3,
    output_dir: str | Path | None = None,
    embedder: Embedder | None = None,
) -> dict[str, Any]:
    started = time.perf_counter()
    embedder = embedder or MiniLMEmbedder()
    targets, traps, cases = _benchmark_spec()
    distractor_items = [_distractor(index) for index in range(max(distractors, 0))]
    items = [*targets, *traps, *distractor_items]

    with tempfile.TemporaryDirectory(prefix="amos-retrieval-comparison-") as directory:
        directory_path = Path(directory)
        store = MemoryStore(directory_path / "memory.sqlite")
        store.reset()
        seed_start = time.perf_counter()
        inserted = store.bulk_upsert_memory(items, batch_size=5_000)
        bm25_build_seconds = time.perf_counter() - seed_start

        vector_start = time.perf_counter()
        vector_index = HNSWMemoryIndex(items, embedder)
        vector_build_seconds = time.perf_counter() - vector_start
        vector_path = directory_path / "memory.hnsw"
        vector_index.save(vector_path)

        rows: list[dict[str, Any]] = []
        for case in cases:
            request = _request(case.text)
            for repeat in range(max(repeats, 1)):
                bm25_ids, bm25_latency = _timed(lambda: _bm25_ids(store, request, limit=12))
                vector_ids, vector_latency = _timed(lambda: vector_index.search(request, limit=12))
                hybrid_ids, hybrid_latency = _timed(
                    lambda: _hybrid_ids(
                        _bm25_ids(store, request, limit=64),
                        vector_index.search(request, limit=64),
                        limit=12,
                    )
                )
                for engine, ids, latency in [
                    ("bm25_governed", bm25_ids, bm25_latency),
                    ("minilm_hnsw_governed", vector_ids, vector_latency),
                    ("rrf_hybrid_governed", hybrid_ids, hybrid_latency),
                ]:
                    rows.append(_grade_row(case, repeat, engine, ids, latency, vector_index.items))

        governance = _governance_update_probes(vector_index)
        result = {
            "schema_version": "amos.retrieval_engine_comparison.v1",
            "status": "completed",
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "configuration": {
                "distractors_requested": distractors,
                "memory_objects_indexed": inserted,
                "target_count": len(targets),
                "trap_count": len(traps),
                "query_count": len(cases),
                "repeats": max(repeats, 1),
                "vector_model": embedder.model_id,
                "vector_model_revision": embedder.revision,
                "embedding_dimension": embedder.dimension,
                "vector_index": "hnswlib cosine HNSW (M=16, ef_construction=200, search_ef=128)",
                "hybrid": "reciprocal-rank fusion, k=60",
            },
            "environment": {
                "python": platform.python_version(),
                "platform": platform.platform(),
                "numpy": np.__version__,
            },
            "build": {
                "bm25_seed_and_index_seconds": round(bm25_build_seconds, 6),
                "bm25_database_bytes": _files_bytes(store.db_path),
                "vector_embed_and_index_seconds": round(vector_build_seconds, 6),
                "vector_index_bytes": vector_path.stat().st_size,
            },
            "aggregate": _aggregate(rows),
            "query_family_aggregate": _family_aggregate(rows),
            "paired_engine_comparison": _paired_engine_comparison(rows),
            "governance_update_probes": governance,
            "rows": rows,
            "wall_seconds": round(time.perf_counter() - started, 6),
            "evidence_boundary": (
                "Internally authored synthetic relevance cases with templated distractors on one machine. "
                "This compares local candidate engines and governed output behavior; it is not independent "
                "retrieval evaluation, a distributed-store benchmark, or deployed-product evidence."
            ),
        }

    if output_dir is not None:
        _write_artifacts(result, Path(output_dir))
    return result


def _benchmark_spec() -> tuple[list[MemoryObject], list[MemoryObject], list[QueryCase]]:
    concepts = [
        ("payment_failure_rate", "Fraction of production payment attempts with failure status, excluding test accounts.", "share of real card checkouts that did not go through"),
        ("subscription_churn_rate", "Fraction of active subscribers who cancel or fail to renew during a monthly cohort.", "paying members who stopped continuing their plan"),
        ("warehouse_freshness_delay", "Delay between source ingestion and analytics warehouse table availability.", "how late reporting data arrives after collection"),
        ("refund_request_rate", "Fraction of completed orders that receive a customer refund request.", "portion of purchases customers ask to reverse"),
        ("api_latency_p95", "Ninety-fifth percentile response latency for production API requests.", "slowest five percent of service response times"),
        ("inventory_stockout_rate", "Fraction of catalog items unavailable when customers attempt to purchase.", "products shoppers cannot buy because shelves are empty"),
        ("login_error_rate", "Fraction of production authentication attempts ending in an error.", "share of sign-in attempts that do not succeed"),
        ("delivery_delay_rate", "Fraction of shipments arriving after the promised delivery date.", "orders reaching buyers later than promised"),
        ("fraud_alert_precision", "Fraction of fraud alerts confirmed as truly fraudulent after review.", "how often suspicious-transaction warnings are actually correct"),
        ("support_resolution_time", "Elapsed time from support ticket creation until confirmed resolution.", "how long customers wait for their issue to be solved"),
        ("pipeline_success_rate", "Fraction of scheduled production data pipeline runs completing successfully.", "share of recurring data jobs that finish without breaking"),
        ("experiment_conversion_lift", "Relative increase in conversion for a treatment compared with its control.", "improvement in purchases caused by the tested experience versus control"),
    ]
    targets: list[MemoryObject] = []
    traps: list[MemoryObject] = []
    cases: list[QueryCase] = []
    for index, (name, definition, paraphrase) in enumerate(concepts):
        target_id = f"memory_retrieval_target_{name}"
        target = _memory(
            target_id,
            f"Approved {name.replace('_', ' ')}. {definition}",
            name=name,
            status="active",
            permissions=ANALYST_PERMISSIONS,
            version="v2",
        )
        targets.append(target)
        traps.extend(
            [
                _memory(
                    f"memory_retrieval_restricted_{name}",
                    f"Restricted exact note for {name.replace('_', ' ')}. {definition}",
                    name=name,
                    status="active",
                    permissions=[*ANALYST_PERMISSIONS, "sre"],
                    version="v2",
                ),
                _memory(
                    f"memory_retrieval_superseded_{name}",
                    f"Legacy superseded {name.replace('_', ' ')}. {definition}",
                    name=name,
                    status="superseded",
                    permissions=ANALYST_PERMISSIONS,
                    version="v1",
                ),
            ]
        )
        cases.extend(
            [
                QueryCase(f"q{index:02d}_lexical", name.replace("_", " "), target_id, "lexical"),
                QueryCase(f"q{index:02d}_semantic", paraphrase, target_id, "semantic_paraphrase"),
            ]
        )
    return targets, traps, cases


def _memory(
    memory_id: str,
    summary: str,
    *,
    name: str,
    status: str,
    permissions: list[str],
    version: str,
) -> MemoryObject:
    return MemoryObject(
        id=memory_id,
        type="semantic_definition",
        summary=summary,
        content={"name": name, "required_filters": ["environment = 'production'"], "time_field": "event_time"},
        source="semantic_layer",
        authority="owner_approved",
        effective_start=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
        permissions=permissions,
        version=version,
        status=status,  # type: ignore[arg-type]
    )


def _distractor(index: int) -> MemoryObject:
    themes = [
        "warehouse slot utilization", "marketing email delivery", "database vacuum duration",
        "employee device enrollment", "image cache hit ratio", "invoice export volume",
        "supplier catalog coverage", "network packet duplication", "forecast batch duration",
        "documentation search clicks", "mobile screen render time", "backup archive size",
    ]
    theme = themes[index % len(themes)]
    template = index % 100
    return _memory(
        f"memory_retrieval_distractor_{index:08d}",
        f"Operational metric template {template} for {theme} in an unrelated reporting workflow.",
        name=f"unrelated_metric_{index:08d}",
        status="active",
        permissions=ANALYST_PERMISSIONS,
        version="v1",
    )


def _request(text: str) -> RetrieveRequest:
    return RetrieveRequest(
        task_text=text,
        required_types=["semantic_definition"],
        time_range=WINDOW,
        user_permissions=ANALYST_PERMISSIONS,
        max_items=12,
    )


def _memory_text(item: MemoryObject) -> str:
    return f"{item.summary} {json.dumps(item.content, sort_keys=True)}"


def _eligible(item: MemoryObject, request: RetrieveRequest) -> bool:
    if request.required_types and item.type not in request.required_types:
        return False
    if item.status != "active":
        return False
    if not has_required_permissions(item, request.user_permissions):
        return False
    if item.effective_end is not None and item.effective_end < request.time_range[0]:
        return False
    if item.effective_start is not None and item.effective_start > request.time_range[1]:
        return False
    return True


def _bm25_ids(store: MemoryStore, request: RetrieveRequest, *, limit: int) -> list[str]:
    expanded = request.model_copy(update={"max_items": limit})
    return [item.id for item in retrieve(expanded, store).items]


def _hybrid_ids(bm25_ids: Sequence[str], vector_ids: Sequence[str], *, limit: int, k: int = 60) -> list[str]:
    scores: dict[str, float] = {}
    first_seen: dict[str, int] = {}
    sequence = 0
    for ranking in [bm25_ids, vector_ids]:
        for rank, memory_id in enumerate(ranking, start=1):
            scores[memory_id] = scores.get(memory_id, 0.0) + 1.0 / (k + rank)
            first_seen.setdefault(memory_id, sequence)
            sequence += 1
    return sorted(scores, key=lambda memory_id: (-scores[memory_id], first_seen[memory_id], memory_id))[:limit]


def _timed(operation):
    start = time.perf_counter()
    value = operation()
    return value, time.perf_counter() - start


def _grade_row(
    case: QueryCase,
    repeat: int,
    engine: str,
    ids: Sequence[str],
    latency: float,
    items: dict[str, MemoryObject],
) -> dict[str, Any]:
    rank = ids.index(case.target_id) + 1 if case.target_id in ids else None
    permission_leaks = [memory_id for memory_id in ids if not has_required_permissions(items[memory_id], ANALYST_PERMISSIONS)]
    superseded_leaks = [memory_id for memory_id in ids if items[memory_id].status != "active"]
    return {
        "query_id": case.query_id,
        "query_family": case.query_family,
        "query_text": case.text,
        "target_id": case.target_id,
        "repeat": repeat,
        "engine": engine,
        "target_rank": rank,
        "top1": rank == 1,
        "recall_at_5": rank is not None and rank <= 5,
        "reciprocal_rank": round(1.0 / rank, 6) if rank else 0.0,
        "latency_seconds": latency,
        "returned_ids": list(ids),
        "permission_leaks": permission_leaks,
        "superseded_leaks": superseded_leaks,
    }


def _aggregate(rows: Sequence[dict[str, Any]]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for engine in sorted({str(row["engine"]) for row in rows}):
        selected = [row for row in rows if row["engine"] == engine]
        latencies = sorted(float(row["latency_seconds"]) for row in selected)
        output[engine] = {
            "observations": len(selected),
            "top1_accuracy": round(sum(bool(row["top1"]) for row in selected) / len(selected), 6),
            "recall_at_5": round(sum(bool(row["recall_at_5"]) for row in selected) / len(selected), 6),
            "mean_reciprocal_rank": round(statistics.fmean(float(row["reciprocal_rank"]) for row in selected), 6),
            "p50_latency_seconds": round(_percentile(latencies, 0.50), 6),
            "p95_latency_seconds": round(_percentile(latencies, 0.95), 6),
            "permission_leak_count": sum(len(row["permission_leaks"]) for row in selected),
            "superseded_leak_count": sum(len(row["superseded_leaks"]) for row in selected),
        }
    return output


def _family_aggregate(rows: Sequence[dict[str, Any]]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for family in sorted({str(row["query_family"]) for row in rows}):
        output[family] = _aggregate([row for row in rows if row["query_family"] == family])
    return output


def _paired_engine_comparison(rows: Sequence[dict[str, Any]]) -> dict[str, Any]:
    by_key = {(row["query_id"], row["repeat"], row["engine"]): row for row in rows}
    engines = sorted({str(row["engine"]) for row in rows})
    pairs: dict[str, Any] = {}
    keys = sorted({(str(row["query_id"]), int(row["repeat"])) for row in rows})
    for left_index, left in enumerate(engines):
        for right in engines[left_index + 1 :]:
            left_wins = right_wins = ties = 0
            for query_id, repeat in keys:
                left_rank = by_key[(query_id, repeat, left)]["target_rank"] or 10_000
                right_rank = by_key[(query_id, repeat, right)]["target_rank"] or 10_000
                if left_rank < right_rank:
                    left_wins += 1
                elif right_rank < left_rank:
                    right_wins += 1
                else:
                    ties += 1
            pairs[f"{left}__vs__{right}"] = {
                "left_wins": left_wins,
                "right_wins": right_wins,
                "ties": ties,
                "paired_observations": len(keys),
            }
    return pairs


def _governance_update_probes(index: HNSWMemoryIndex) -> dict[str, Any]:
    target_id = "memory_retrieval_target_payment_failure_rate"
    original = index.items[target_id]
    request = _request("payment failure rate")
    before = index.search(request, limit=12)
    revoked = original.model_copy(update={"permissions": [*ANALYST_PERMISSIONS, "sre"], "version": "v3"})
    index.replace_metadata(revoked)
    after_revocation = index.search(request, limit=12)
    restored = original.model_copy(update={"status": "superseded"})
    index.replace_metadata(restored)
    after_supersession = index.search(request, limit=12)
    index.replace_metadata(original)
    return {
        "permission_revocation": {
            "passed": target_id in before and target_id not in after_revocation,
            "before_visible": target_id in before,
            "after_visible": target_id in after_revocation,
        },
        "metric_supersession": {
            "passed": target_id not in after_supersession,
            "after_visible": target_id in after_supersession,
        },
    }


def _files_bytes(path: Path) -> int:
    return sum(candidate.stat().st_size for candidate in path.parent.glob(f"{path.name}*") if candidate.is_file())


def _percentile(values: Sequence[float], percentile: float) -> float:
    if not values:
        return 0.0
    if len(values) == 1:
        return float(values[0])
    position = (len(values) - 1) * percentile
    lower = int(position)
    upper = min(lower + 1, len(values) - 1)
    fraction = position - lower
    return float(values[lower] * (1 - fraction) + values[upper] * fraction)


def _write_artifacts(result: dict[str, Any], output_dir: Path) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    results_path = output_dir / "results.json"
    rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
    results_path.write_text(rendered, encoding="utf-8")
    digest = hashlib.sha256(rendered.encode("utf-8")).hexdigest()
    (output_dir / "results.sha256").write_text(f"{digest}  results.json\n", encoding="utf-8")
    lines = [
        "# Governed Retrieval-Engine Comparison",
        "",
        f"- Distractors: {result['configuration']['distractors_requested']}",
        f"- Queries: {result['configuration']['query_count']}",
        f"- Vector model revision: `{result['configuration']['vector_model_revision']}`",
        "",
        "| Engine | Top-1 | Recall@5 | MRR | p50 (s) | p95 (s) | Permission leaks | Superseded leaks |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for engine, metrics in result["aggregate"].items():
        lines.append(
            f"| {engine} | {metrics['top1_accuracy']} | {metrics['recall_at_5']} | "
            f"{metrics['mean_reciprocal_rank']} | {metrics['p50_latency_seconds']} | "
            f"{metrics['p95_latency_seconds']} | {metrics['permission_leak_count']} | "
            f"{metrics['superseded_leak_count']} |"
        )
    lines.extend(["", f"Evidence boundary: {result['evidence_boundary']}", ""])
    (output_dir / "summary.md").write_text("\n".join(lines), encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description="Compare governed BM25, MiniLM/HNSW, and hybrid retrieval.")
    parser.add_argument("--distractors", type=int, default=10_000)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--output-dir", required=True)
    args = parser.parse_args()
    result = run_retrieval_engine_comparison(
        distractors=max(args.distractors, 0),
        repeats=max(args.repeats, 1),
        output_dir=args.output_dir,
    )
    print(json.dumps({"status": result["status"], "output_dir": str(Path(args.output_dir).resolve())}, indent=2))


if __name__ == "__main__":
    main()
