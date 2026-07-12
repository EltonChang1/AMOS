from __future__ import annotations

from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

from amos.config import settings


def create_failure_rate_chart(rows: list[dict[str, object]], chart_id: str) -> Path:
    settings.ensure_dirs()
    path = settings.charts_dir / f"{chart_id}.png"
    buckets = [str(row["bucket"])[11:16] for row in rows]
    rates = [float(row["failure_rate"]) * 100 for row in rows]

    fig, ax = plt.subplots(figsize=(8, 4.2))
    ax.plot(buckets, rates, color="#1f6feb", linewidth=2.2, marker="o")
    ax.axvline("14:00", color="#b42318", linestyle="--", linewidth=1.2, label="spike window")
    ax.set_title("Payment failure rate by event-time hour")
    ax.set_xlabel("Event time")
    ax.set_ylabel("Failure rate (%)")
    ax.grid(True, axis="y", alpha=0.28)
    ax.legend(loc="upper left")
    fig.tight_layout()
    fig.savefig(path, dpi=160)
    plt.close(fig)
    return path
