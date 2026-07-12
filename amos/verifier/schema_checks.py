from __future__ import annotations

import sqlglot
from sqlglot import exp
from sqlglot.errors import OptimizeError
from sqlglot.optimizer.qualify import qualify

from amos.memory.models import MemoryObject


def check_schema(sql: str, schema: MemoryObject) -> tuple[list[str], list[str]]:
    warnings: list[str] = []
    errors: list[str] = []
    expected_table = schema.content["table"]
    allowed_columns = set(schema.content["columns"])
    blocked_columns = set(schema.content.get("blocked_columns", []))
    column_types = {
        str(column): str(data_type).upper()
        for column, data_type in schema.content.get("column_types", {}).items()
    }

    try:
        parsed = sqlglot.parse_one(sql, read="duckdb")
    except Exception as exc:
        return warnings, [f"SQL parse failed before schema verification: {exc}"]

    cte_names = {cte.alias_or_name for cte in parsed.find_all(exp.CTE)}
    physical_tables = {
        table.name
        for table in parsed.find_all(exp.Table)
        if table.name not in cte_names
    }
    if expected_table not in physical_tables:
        errors.append(f"SQL does not reference expected table {expected_table}.")

    for column in sorted({column.name for column in parsed.find_all(exp.Column)}):
        if column in blocked_columns:
            errors.append(f"SQL references blocked column {column}.")

    naive_timestamp_columns = {
        column
        for column, data_type in column_types.items()
        if data_type in {"TIMESTAMP", "TIMESTAMPNTZ", "TIMESTAMP WITHOUT TIME ZONE"}
    }
    references_naive_timestamp = any(
        column.name in naive_timestamp_columns
        for column in parsed.find_all(exp.Column)
    )
    uses_timezone_aware_literal = any(
        data_type.this == exp.DataType.Type.TIMESTAMPTZ
        for data_type in parsed.find_all(exp.DataType)
    )
    if references_naive_timestamp and uses_timezone_aware_literal:
        errors.append(
            "SQL compares or combines timezone-naive TIMESTAMP schema fields with TIMESTAMPTZ values; "
            "use TIMESTAMP literals matching the governed schema to avoid session-timezone shifts."
        )

    # sqlglot's qualifier resolves names in scope, including SELECT aliases,
    # CTE outputs, subqueries, and derived columns. This avoids treating a
    # legitimate alias as an unknown base-table column while still rejecting
    # unresolved references inside a derived scope.
    try:
        qualify(
            parsed.copy(),
            dialect="duckdb",
            schema={expected_table: {column: "UNKNOWN" for column in allowed_columns}},
            validate_qualify_columns=True,
            identify=False,
        )
    except OptimizeError as exc:
        errors.append(f"Schema qualification failed for {schema.id}: {exc}")

    return warnings, errors
