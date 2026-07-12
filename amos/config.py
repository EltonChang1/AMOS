from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(os.environ.get("AMOS_ROOT", ".")).resolve()


@dataclass
class Settings:
    root: Path = ROOT
    memory_db: Path = Path(os.environ.get("AMOS_MEMORY_DB", ROOT / "data" / "amos_memory.sqlite"))
    analytics_db: Path = Path(
        os.environ.get("AMOS_ANALYTICS_DB", ROOT / "data" / "synthetic" / "payments.duckdb")
    )
    artifact_dir: Path = Path(os.environ.get("AMOS_ARTIFACT_DIR", ROOT / "artifacts"))
    rotate_analytics_db_on_seed: bool = os.environ.get("AMOS_ROTATE_ANALYTICS_DB_ON_SEED") == "1"

    @property
    def reports_dir(self) -> Path:
        return self.artifact_dir / "reports"

    @property
    def charts_dir(self) -> Path:
        return self.artifact_dir / "charts"

    @property
    def queries_dir(self) -> Path:
        return self.artifact_dir / "queries"

    @property
    def provenance_dir(self) -> Path:
        return self.artifact_dir / "provenance"

    @property
    def replay_dir(self) -> Path:
        return self.artifact_dir / "replay"

    @property
    def llm_runs_dir(self) -> Path:
        return self.artifact_dir / "llm_runs"

    def use_run_dir(self, run_dir: str | Path, rotate_analytics_db_on_seed: bool = True) -> None:
        root = Path(run_dir).resolve()
        self.root = root
        self.memory_db = Path(os.environ.get("AMOS_MEMORY_DB", root / "data" / "amos_memory.sqlite"))
        self.analytics_db = Path(
            os.environ.get("AMOS_ANALYTICS_DB", root / "data" / "synthetic" / "payments.duckdb")
        )
        self.artifact_dir = Path(os.environ.get("AMOS_ARTIFACT_DIR", root / "artifacts"))
        self.rotate_analytics_db_on_seed = rotate_analytics_db_on_seed

    def use_paths(
        self,
        memory_db: str | Path | None = None,
        analytics_db: str | Path | None = None,
        artifact_dir: str | Path | None = None,
    ) -> None:
        if memory_db is not None:
            self.memory_db = Path(memory_db).resolve()
        if analytics_db is not None:
            self.analytics_db = Path(analytics_db).resolve()
        if artifact_dir is not None:
            self.artifact_dir = Path(artifact_dir).resolve()

    def ensure_dirs(self) -> None:
        self.memory_db.parent.mkdir(parents=True, exist_ok=True)
        self.analytics_db.parent.mkdir(parents=True, exist_ok=True)
        for path in [
            self.reports_dir,
            self.charts_dir,
            self.queries_dir,
            self.provenance_dir,
            self.replay_dir,
            self.llm_runs_dir,
        ]:
            path.mkdir(parents=True, exist_ok=True)


settings = Settings()
