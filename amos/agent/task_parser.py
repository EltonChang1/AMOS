from __future__ import annotations

from datetime import datetime, timezone

from pydantic import BaseModel

from amos.tools.sql_templates import PAYMENT_WINDOW_END, PAYMENT_WINDOW_START


class ParsedTask(BaseModel):
    task_id: str
    target_metric: str
    time_range: tuple[datetime, datetime]
    asks_dashboard_update: bool


def parse_task(request: str, task_id: str) -> ParsedTask:
    lowered = request.lower()
    metric = "payment_failure_rate" if "payment" in lowered and "failure" in lowered else "unknown"
    return ParsedTask(
        task_id=task_id,
        target_metric=metric,
        time_range=(
            datetime.fromisoformat(PAYMENT_WINDOW_START).astimezone(timezone.utc),
            datetime.fromisoformat(PAYMENT_WINDOW_END).astimezone(timezone.utc),
        ),
        asks_dashboard_update="dashboard" in lowered or "should we update" in lowered,
    )
