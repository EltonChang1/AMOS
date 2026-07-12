from __future__ import annotations

from amos.memory.models import ClaimRecord, MemoryObject, VerificationResult
from amos.provenance.models import ClaimProvenance
from amos.verifier.freshness_checks import check_freshness
from amos.verifier.metric_checks import check_metric_rules
from amos.verifier.permission_checks import check_memory_permissions
from amos.verifier.provenance_checks import check_provenance_level
from amos.verifier.schema_checks import check_schema
from amos.verifier.sql_checks import check_sql_read_only


def verify_sql(
    sql: str,
    schema: MemoryObject,
    metric: MemoryObject,
    stream_state: MemoryObject,
    memory_items: list[MemoryObject],
    user_permissions: list[str],
) -> VerificationResult:
    passed: list[str] = []
    warnings: list[str] = []
    errors: list[str] = []

    sql_result = check_sql_read_only(sql)
    warnings.extend(sql_result.warnings)
    errors.extend(sql_result.errors)
    if sql_result.ok:
        passed.append("sql_read_only")

    schema_warnings, schema_errors = check_schema(sql, schema)
    warnings.extend(schema_warnings)
    errors.extend(schema_errors)
    if not schema_errors:
        passed.append("schema_compatible")

    metric_warnings, metric_errors = check_metric_rules(sql, metric)
    warnings.extend(metric_warnings)
    errors.extend(metric_errors)
    if not metric_errors:
        passed.append("metric_rules")

    freshness_warnings, freshness_errors = check_freshness(stream_state)
    warnings.extend(freshness_warnings)
    errors.extend(freshness_errors)
    if not freshness_errors:
        passed.append("freshness")

    permission_warnings, permission_errors = check_memory_permissions(memory_items, user_permissions)
    warnings.extend(permission_warnings)
    errors.extend(permission_errors)
    if not permission_errors:
        passed.append("permissions")

    return VerificationResult(status=_status(errors, warnings), passed_checks=passed, warnings=warnings, errors=errors)


def verify_provenance(
    claims: list[ClaimRecord],
    provenance: list[ClaimProvenance],
    provenance_level: int,
) -> VerificationResult:
    warnings, errors, coverage = check_provenance_level(claims, provenance, provenance_level)
    passed = ["provenance_coverage"] if not errors else []
    return VerificationResult(
        status=_status(errors, warnings),
        passed_checks=passed,
        warnings=warnings,
        errors=errors,
        provenance_coverage=coverage,
    )


def _status(errors: list[str], warnings: list[str]) -> str:
    if errors:
        return "fail"
    if warnings:
        return "warning"
    return "pass"
