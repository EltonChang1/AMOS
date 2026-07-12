from __future__ import annotations

from amos.verifier.sql_checks import check_sql_read_only


def test_read_only_checker_accepts_union_all_of_selects() -> None:
    result = check_sql_read_only(
        "SELECT 'previous' AS period FROM payment_events "
        "UNION ALL SELECT 'current' AS period FROM payment_events"
    )

    assert result.ok is True
    assert result.errors == []


def test_read_only_checker_rejects_write_inside_cte() -> None:
    result = check_sql_read_only(
        "WITH deleted AS (DELETE FROM payment_events RETURNING event_id) SELECT * FROM deleted"
    )

    assert result.ok is False
    assert any("blocked write" in error for error in result.errors)
