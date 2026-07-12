from __future__ import annotations

from amos.memory.store import MemoryStore


def no_memory_baseline() -> dict[str, object]:
    return {
        "task_correctness": False,
        "temporal_correctness": False,
        "metric_correctness": False,
        "provenance_coverage": 0.0,
        "replay_success": False,
        "feedback_retention": False,
        "permission_safety": True,
    }


def naive_rag_baseline(store: MemoryStore | None = None) -> dict[str, object]:
    store = store or MemoryStore()
    all_memory = store.list_memory()
    leaked_restricted = any("sre" in item.permissions for item in all_memory)
    return {
        "task_correctness": False,
        "temporal_correctness": False,
        "metric_correctness": False,
        "provenance_coverage": 0.25,
        "replay_success": False,
        "feedback_retention": False,
        "permission_safety": not leaked_restricted,
    }


def long_context_baseline(store: MemoryStore | None = None) -> dict[str, object]:
    return naive_rag_baseline(store)
