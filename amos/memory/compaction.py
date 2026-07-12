from __future__ import annotations

from amos.memory.models import MemoryObject


def compact(items: list[MemoryObject]) -> list[dict[str, object]]:
    return [
        {
            "id": item.id,
            "type": item.type,
            "summary": item.summary,
            "authority": item.authority,
            "version": item.version,
            "effective_start": item.effective_start,
            "effective_end": item.effective_end,
        }
        for item in items
    ]
