from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from amos.config import settings


def write_text_artifact(directory: Path, artifact_id: str, suffix: str, text: str) -> Path:
    settings.ensure_dirs()
    path = directory / f"{artifact_id}.{suffix}"
    path.write_text(text, encoding="utf-8")
    return path


def write_json_artifact(directory: Path, artifact_id: str, payload: dict[str, Any]) -> Path:
    return write_text_artifact(directory, artifact_id, "json", json.dumps(payload, default=str, indent=2, sort_keys=True))
