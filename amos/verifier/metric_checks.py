from __future__ import annotations

import sqlglot
from sqlglot import exp

from amos.memory.models import MemoryObject


def check_metric_rules(sql: str, metric: MemoryObject) -> tuple[list[str], list[str]]:
    warnings: list[str] = []
    errors: list[str] = []

    try:
        expressions = sqlglot.parse(sql, read="duckdb")
    except Exception as exc:
        return warnings, [f"SQL parse failed before metric verification: {exc}"]

    if not expressions:
        return warnings, ["SQL parse failed before metric verification: no expressions found."]

    row_filters = list(_row_filter_conditions(expressions))
    has_failure_numerator = _has_failure_numerator(expressions)
    has_count_denominator = _has_unfiltered_count_star(expressions)
    has_production_filter = _has_environment_production(row_filters)
    has_test_account_filter = _has_test_account_exclusion(row_filters)
    has_event_time_filter = _has_event_time_filter(row_filters)

    if not has_failure_numerator:
        errors.append("Metric numerator must count status = 'failure'.")
    if not has_count_denominator:
        errors.append("Metric denominator must use total attempts via COUNT(*).")
    if not has_event_time_filter:
        errors.append("Metric must filter by event_time.")

    required_filters = metric.content.get("required_filters", [])
    for required_filter in required_filters:
        if not _required_filter_present(str(required_filter), row_filters, has_production_filter, has_test_account_filter):
            errors.append(f"Missing required metric filter: {required_filter}")

    return warnings, errors


def _row_filter_conditions(expressions: list[exp.Expression]) -> list[exp.Expression]:
    conditions: list[exp.Expression] = []
    for expression in expressions:
        for select in expression.find_all(exp.Select):
            where = select.args.get("where")
            if isinstance(where, exp.Where) and where.this is not None:
                conditions.append(where.this)
            for join in select.args.get("joins") or []:
                join_condition = join.args.get("on")
                if isinstance(join_condition, exp.Expression):
                    conditions.append(join_condition)
    return conditions


def _has_failure_numerator(expressions: list[exp.Expression]) -> bool:
    for expression in expressions:
        for node in expression.find_all(exp.AggFunc):
            if isinstance(node.parent, exp.Filter):
                continue
            if _contains_status_failure(node):
                return True
        for node in expression.find_all(exp.Filter):
            if isinstance(node.this, exp.AggFunc) and _contains_status_failure(node):
                return True
    return False


def _has_unfiltered_count_star(expressions: list[exp.Expression]) -> bool:
    for expression in expressions:
        for node in expression.find_all(exp.Count):
            if not _is_count_star(node):
                continue
            parent = node.parent
            if not isinstance(parent, exp.Filter):
                return True
            # COUNT(*) FILTER (...) is a valid total-attempt denominator when
            # the filter selects a time bucket/window rather than an outcome.
            # A status-conditioned count remains a numerator, not a total.
            filter_condition = parent.expression
            if filter_condition is not None and not _contains_named_column(filter_condition, "status"):
                return True
    return False


def _has_environment_production(conditions: list[exp.Expression]) -> bool:
    return any(
        _has_comparison(condition, "environment", "production")
        for condition in conditions
    )


def _has_test_account_exclusion(conditions: list[exp.Expression]) -> bool:
    for condition in conditions:
        for node in condition.walk():
            if isinstance(node, exp.Not) and _is_column(node.this, "is_test_account"):
                return True
            if isinstance(node, (exp.EQ, exp.Is)):
                if _column_boolean_match(node.this, node.expression, "is_test_account", False):
                    return True
                if _column_boolean_match(node.expression, node.this, "is_test_account", False):
                    return True
            if isinstance(node, exp.NEQ):
                if _column_boolean_match(node.this, node.expression, "is_test_account", True):
                    return True
                if _column_boolean_match(node.expression, node.this, "is_test_account", True):
                    return True
    return False


def _has_event_time_filter(conditions: list[exp.Expression]) -> bool:
    comparison_types = (exp.Between, exp.EQ, exp.GT, exp.GTE, exp.LT, exp.LTE)
    for condition in conditions:
        for node in condition.walk():
            if isinstance(node, comparison_types) and _contains_column(node, "event_time"):
                return True
    return False


def _required_filter_present(
    required_filter: str,
    conditions: list[exp.Expression],
    has_production_filter: bool,
    has_test_account_filter: bool,
) -> bool:
    normalized = " ".join(required_filter.lower().split())
    if normalized in {"environment = 'production'", 'environment = "production"'}:
        return has_production_filter
    if normalized in {"is_test_account = false", "is_test_account is false", "not is_test_account"}:
        return has_test_account_filter

    try:
        required = sqlglot.parse_one(required_filter, read="duckdb", into=exp.Condition)
    except Exception:
        return any(normalized == condition.sql(dialect="duckdb").lower() for condition in conditions)

    required_sql = required.sql(dialect="duckdb").lower()
    return any(
        required_sql == candidate.sql(dialect="duckdb").lower()
        for condition in conditions
        for candidate in condition.walk()
        if isinstance(candidate, exp.Expression)
    )


def _contains_status_failure(node: exp.Expression) -> bool:
    for candidate in node.walk():
        if isinstance(candidate, exp.EQ) and _comparison_matches(candidate, "status", "failure"):
            return True
    return False


def _has_comparison(condition: exp.Expression, column: str, literal: str) -> bool:
    for node in condition.walk():
        if isinstance(node, exp.EQ) and _comparison_matches(node, column, literal):
            return True
    return False


def _comparison_matches(node: exp.EQ, column: str, literal: str) -> bool:
    return (
        _is_column(node.this, column)
        and _literal_text(node.expression) == literal
    ) or (
        _is_column(node.expression, column)
        and _literal_text(node.this) == literal
    )


def _column_boolean_match(left: exp.Expression | None, right: exp.Expression | None, column: str, value: bool) -> bool:
    return _is_column(left, column) and _literal_bool(right) is value


def _contains_column(node: exp.Expression, name: str) -> bool:
    return any(_is_column(candidate, name) for candidate in node.find_all(exp.Column))


def _contains_named_column(node: exp.Expression, name: str) -> bool:
    return any(candidate.name.lower() == name for candidate in node.find_all(exp.Column))


def _is_column(node: exp.Expression | None, name: str) -> bool:
    return isinstance(node, exp.Column) and node.name.lower() == name


def _literal_text(node: exp.Expression | None) -> str | None:
    if isinstance(node, exp.Literal):
        return str(node.this).strip("'\"").lower()
    return None


def _literal_bool(node: exp.Expression | None) -> bool | None:
    if isinstance(node, exp.Boolean):
        return bool(node.this)
    if isinstance(node, exp.Literal):
        value = str(node.this).strip("'\"").lower()
        if value in {"false", "0"}:
            return False
        if value in {"true", "1"}:
            return True
    return None


def _is_count_star(node: exp.Count) -> bool:
    return isinstance(node.this, exp.Star)
