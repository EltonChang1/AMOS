from __future__ import annotations

from datetime import datetime

import numpy as np
import pytest

pytest.importorskip("hnswlib")

from amos.evaluation.retrieval_engine_comparison import (  # noqa: E402
    ANALYST_PERMISSIONS,
    HNSWMemoryIndex,
    _hybrid_ids,
    _request,
)
from amos.memory.models import MemoryObject  # noqa: E402


class FakeEmbedder:
    model_id = "fake-semantic-embedder"
    revision = "test-revision"
    dimension = 3

    def encode(self, texts):
        rows = []
        for text in texts:
            lowered = text.lower()
            if any(term in lowered for term in ["payment", "checkout", "card", "failure", "did not go through"]):
                vector = np.array([1.0, 0.0, 0.0], dtype="float32")
            elif any(term in lowered for term in ["warehouse", "slot", "storage"]):
                vector = np.array([0.0, 1.0, 0.0], dtype="float32")
            else:
                vector = np.array([0.0, 0.0, 1.0], dtype="float32")
            rows.append(vector)
        return np.asarray(rows, dtype="float32")


def _item(memory_id: str, summary: str, *, status="active", permissions=None) -> MemoryObject:
    return MemoryObject(
        id=memory_id,
        type="semantic_definition",
        summary=summary,
        content={"name": memory_id},
        source="semantic_layer",
        authority="owner_approved",
        effective_start=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
        permissions=permissions or ANALYST_PERMISSIONS,
        version="v1",
        status=status,
    )


def test_hnsw_search_filters_restricted_and_superseded_near_duplicates() -> None:
    target = _item("target", "Approved payment failures for production checkout attempts.")
    restricted = _item(
        "restricted",
        "Exact payment failure checkout note.",
        permissions=[*ANALYST_PERMISSIONS, "sre"],
    )
    superseded = _item("superseded", "Legacy payment failure checkout definition.", status="superseded")
    distractor = _item("distractor", "Warehouse storage slot utilization.")
    index = HNSWMemoryIndex([target, restricted, superseded, distractor], FakeEmbedder())

    ids = index.search(_request("share of card checkouts that did not go through"), limit=4)

    assert ids[0] == "target"
    assert "restricted" not in ids
    assert "superseded" not in ids


def test_hnsw_metadata_updates_hide_revoked_and_superseded_item() -> None:
    target = _item("target", "Approved payment failure metric.")
    distractor = _item("distractor", "Warehouse slot metric.")
    index = HNSWMemoryIndex([target, distractor], FakeEmbedder())
    request = _request("payment failures")
    assert "target" in index.search(request, limit=2)

    index.replace_metadata(target.model_copy(update={"permissions": [*ANALYST_PERMISSIONS, "sre"]}))
    assert "target" not in index.search(request, limit=2)

    index.replace_metadata(target.model_copy(update={"status": "superseded"}))
    assert "target" not in index.search(request, limit=2)


def test_reciprocal_rank_hybrid_is_deterministic() -> None:
    result = _hybrid_ids(["a", "b", "c"], ["b", "d", "a"], limit=4)

    assert result == ["b", "a", "d", "c"]
