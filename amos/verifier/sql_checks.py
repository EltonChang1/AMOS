from __future__ import annotations

from dataclasses import dataclass, field

import sqlglot
from sqlglot import exp


BLOCKED_STATEMENTS = (exp.Insert, exp.Update, exp.Delete, exp.Drop, exp.Alter, exp.Create, exp.Command)
BLOCKED_COLUMNS = {"customer_email", "payment_token", "raw_payload"}


@dataclass
class SQLCheckResult:
    ok: bool
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    tables: set[str] = field(default_factory=set)
    columns: set[str] = field(default_factory=set)


def check_sql_read_only(sql: str) -> SQLCheckResult:
    try:
        expressions = sqlglot.parse(sql, read="duckdb")
    except Exception as exc:
        return SQLCheckResult(ok=False, errors=[f"SQL parse failed: {exc}"])

    result = SQLCheckResult(ok=True)
    for expression in expressions:
        if not isinstance(expression, exp.Query):
            result.errors.append("SQL must be a SELECT query.")
        if expression.find(BLOCKED_STATEMENTS):
            result.errors.append("SQL contains a blocked write or DDL statement.")
        result.tables.update(table.name for table in expression.find_all(exp.Table))
        for column in expression.find_all(exp.Column):
            name = column.name
            result.columns.add(name)
            if name.lower() in BLOCKED_COLUMNS:
                result.errors.append(f"SQL references blocked column: {name}")

    result.ok = not result.errors
    return result


def table_columns(sql: str) -> tuple[set[str], set[str]]:
    parsed = sqlglot.parse_one(sql, read="duckdb")
    tables = {table.name for table in parsed.find_all(exp.Table)}
    columns = {column.name for column in parsed.find_all(exp.Column)}
    return tables, columns
