from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

import duckdb

from amos.config import settings


class DuckDBTool:
    def __init__(self, db_path: Path | None = None) -> None:
        self.db_path = db_path or settings.analytics_db

    def execute(self, sql: str, params: dict[str, Any] | None = None) -> list[dict[str, Any]]:
        with duckdb.connect(str(self.db_path), read_only=True) as conn:
            cursor = conn.execute(sql, params) if params else conn.execute(sql)
            columns = [col[0] for col in cursor.description]
            rows = cursor.fetchall()
        return [dict(zip(columns, row, strict=True)) for row in rows]

    def result_hash(self, rows: list[dict[str, Any]]) -> str:
        payload = json.dumps(rows, default=str, sort_keys=True)
        return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description="Execute read-only SQL against the AMOS DuckDB dataset.")
    parser.add_argument("--query", required=True)
    args = parser.parse_args()
    tool = DuckDBTool()
    rows = tool.execute(args.query)
    for row in rows:
        print(json.dumps(row, default=str, sort_keys=True))


if __name__ == "__main__":
    main()
