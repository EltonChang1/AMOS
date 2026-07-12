from __future__ import annotations

from amos.memory.store import MemoryStore


def export_provenance(artifact_id: str, store: MemoryStore | None = None) -> list[dict[str, object]]:
    store = store or MemoryStore()
    return [record.model_dump(mode="json") for record in store.list_claim_provenance(artifact_id)]
