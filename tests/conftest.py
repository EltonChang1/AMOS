from __future__ import annotations

import pytest

from amos.config import settings
from amos.memory.seed_memory import seed_memory
from amos.tools.seed_duckdb import seed_duckdb


@pytest.fixture(scope="session", autouse=True)
def seeded(tmp_path_factory: pytest.TempPathFactory) -> None:
    run_dir = tmp_path_factory.mktemp("amos_isolated_run")
    settings.use_run_dir(run_dir)
    seed_memory(reset=True)
    seed_duckdb()
