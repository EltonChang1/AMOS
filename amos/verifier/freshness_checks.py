from __future__ import annotations

from datetime import datetime

from amos.memory.models import MemoryObject


def check_freshness(stream_state: MemoryObject) -> tuple[list[str], list[str]]:
    warnings: list[str] = []
    errors: list[str] = []
    watermark = datetime.fromisoformat(stream_state.content["watermark"].replace("Z", "+00:00"))
    end_time = datetime.fromisoformat(stream_state.content["event_time_end"].replace("Z", "+00:00"))
    lag_seconds = (end_time - watermark).total_seconds()
    if lag_seconds > 0:
        warnings.append(
            f"Watermark trails requested window end by {int(lag_seconds)} seconds; late data may change small counts."
        )
    if lag_seconds > 900:
        errors.append("Watermark is beyond the configured 15 minute late-data tolerance.")
    return warnings, errors
