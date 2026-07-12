from __future__ import annotations

import json
from pathlib import Path

from amos.agent.live_agent import (
    CodexCLIProvider,
    OfflineLiveProvider,
    OpenAIResponsesProvider,
    ProviderResponse,
    VerifiedSQLExecutionError,
    _build_sql_prompt,
    _sql_output_contract_errors,
    provider_from_env,
    run_live_agent_task,
)
from amos.memory.models import User
from amos.provenance.replay import replay_artifact
from amos.tools.sql_templates import (
    PAYMENT_WINDOW_START,
    payment_failure_concentration_sql,
    payment_failure_summary_sql,
    payment_failure_timeseries_sql,
)


def test_offline_live_agent_writes_raw_prompt_trace(seeded: None) -> None:
    result = run_live_agent_task(
        "Why did payment failure rate increase over the last six hours, and should we update the dashboard?",
        User(id="analyst_001", permissions=["analytics", "payments"]),
        provider=OfflineLiveProvider(),
    )

    assert result.status in ["pass", "warning"]
    assert result.result is not None
    assert result.artifact_id is not None
    assert result.raw_trace_path.endswith(".json")
    assert result.prompt_count == 3
    assert result.token_usage["input_tokens"] > 0

    trace = json.loads(Path(result.raw_trace_path).read_text(encoding="utf-8"))
    assert trace["provider"] == "offline"
    assert [event["phase"] for event in trace["events"]] == ["analysis_plan", "sql_proposal", "report_draft"]
    assert all(event["prompt"] for event in trace["events"])
    assert all(event["response_text"] for event in trace["events"])
    assert all(event["prompt_hash"] for event in trace["events"])
    assert all(event["response_hash"] for event in trace["events"])

    report_text = Path(result.result.report_path).read_text(encoding="utf-8")
    assert "## Model Draft" in report_text
    assert result.run_id in report_text
    assert replay_artifact(result.result.artifact_id).status == "pass"


def test_live_agent_repairs_rejected_sql_before_execution(seeded: None) -> None:
    provider = RepairingProvider()

    result = run_live_agent_task(
        "Why did payment failure rate increase over the last six hours?",
        User(id="analyst_001", permissions=["analytics", "payments"]),
        provider=provider,
        max_repair_attempts=1,
    )

    assert result.status in ["pass", "warning"]
    assert provider.phases == ["analysis_plan", "sql_proposal", "sql_repair", "report_draft"]
    trace = json.loads(Path(result.raw_trace_path).read_text(encoding="utf-8"))
    repair_events = [event for event in trace["events"] if event["phase"] == "sql_repair"]
    assert len(repair_events) == 1
    assert "failure_reason" in repair_events[0]["prompt"]
    assert json.loads(repair_events[0]["response_text"])["sql"].strip() == payment_failure_summary_sql().strip()


def test_live_agent_repairs_missing_query_output_contract(seeded: None) -> None:
    provider = OutputContractProvider()

    result = run_live_agent_task(
        "Why did payment failure rate increase over the last six hours?",
        User(id="analyst_001", permissions=["analytics", "payments"]),
        provider=provider,
        max_repair_attempts=1,
    )

    assert result.status in ["pass", "warning"]
    assert provider.phases == ["analysis_plan", "sql_proposal", "sql_repair", "report_draft"]
    trace = json.loads(Path(result.raw_trace_path).read_text(encoding="utf-8"))
    repair = next(event for event in trace["events"] if event["phase"] == "sql_repair")
    assert "missing required output columns: period" in repair["prompt"]


def test_live_agent_rejects_output_contract_without_key_error(seeded: None) -> None:
    provider = OutputContractProvider()

    result = run_live_agent_task(
        "Why did payment failure rate increase over the last six hours?",
        User(id="analyst_001", permissions=["analytics", "payments"]),
        provider=provider,
        max_repair_attempts=0,
    )

    assert result.status == "reject"
    assert any("missing required output columns: period" in error for error in result.errors)
    assert Path(result.raw_trace_path).exists()


def test_summary_contract_requires_canonical_period_labels() -> None:
    sql = payment_failure_summary_sql().replace("'current'", "'requested_window'").replace(
        "'previous'", "'comparison_window'"
    )

    errors = _sql_output_contract_errors(sql, "summary")

    assert any("required period label literals: current, previous" in error for error in errors)


def test_sql_prompt_exposes_exact_runtime_contract_and_windows() -> None:
    prompt = _build_sql_prompt("Investigate the spike", [], ["segment analysis"])

    assert "exactly three queries" in prompt
    assert "previous [2026-07-07T08:00:00+00:00, 2026-07-07T14:00:00+00:00)" in prompt
    assert "current [2026-07-07T14:00:00+00:00, 2026-07-07T20:00:00+00:00)" in prompt
    assert "never use TIMESTAMPTZ" in prompt


def test_live_agent_records_verified_sql_execution_failure(seeded: None, monkeypatch) -> None:
    def fail_execution(*args, **kwargs):
        raise RuntimeError("simulated DuckDB binder failure")

    monkeypatch.setattr("amos.agent.live_agent._execute_verified_sqls", fail_execution)
    result = run_live_agent_task(
        "Why did payment failure rate increase over the last six hours?",
        User(id="analyst_001", permissions=["analytics", "payments"]),
        provider=OfflineLiveProvider(),
    )

    assert result.status == "reject"
    assert any("Verified SQL execution raised RuntimeError" in error for error in result.errors)
    trace = json.loads(Path(result.raw_trace_path).read_text(encoding="utf-8"))
    assert trace["status"] == "reject"
    assert any("simulated DuckDB binder failure" in error for error in trace["errors"])


def test_live_agent_repairs_verified_sql_execution_failure(seeded: None, monkeypatch) -> None:
    from amos.agent import live_agent

    execute_verified = live_agent._execute_verified_sqls
    attempts = 0

    def fail_once(artifact_id, sqls):
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            query_id, info = next(
                (query_id, info)
                for query_id, info in sqls.items()
                if info["kind"] == "summary"
            )
            raise VerifiedSQLExecutionError(
                query_id,
                "summary",
                str(info["sql"]),
                RuntimeError("simulated binder failure"),
            )
        return execute_verified(artifact_id, sqls)

    monkeypatch.setattr("amos.agent.live_agent._execute_verified_sqls", fail_once)
    result = run_live_agent_task(
        "Why did payment failure rate increase over the last six hours?",
        User(id="analyst_001", permissions=["analytics", "payments"]),
        provider=OfflineLiveProvider(),
    )

    assert result.status in {"pass", "warning"}
    assert attempts == 2
    trace = json.loads(Path(result.raw_trace_path).read_text(encoding="utf-8"))
    assert [event["phase"] for event in trace["events"]] == [
        "analysis_plan",
        "sql_proposal",
        "sql_repair",
        "report_draft",
    ]
    repair = next(event for event in trace["events"] if event["phase"] == "sql_repair")
    assert "SQL execution raised RuntimeError: simulated binder failure" in repair["prompt"]


def test_provider_from_env_selects_offline_or_openai(monkeypatch) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.setenv("AMOS_LIVE_AGENT_PROVIDER", "auto")
    assert isinstance(provider_from_env(), OfflineLiveProvider)

    monkeypatch.setenv("OPENAI_API_KEY", "test-key")
    monkeypatch.setenv("AMOS_LIVE_AGENT_PROVIDER", "openai")
    monkeypatch.setenv("AMOS_LIVE_LLM_MODEL", "gpt-test")
    provider = provider_from_env()
    assert isinstance(provider, OpenAIResponsesProvider)
    assert provider.model == "gpt-test"

    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.setenv("AMOS_LIVE_AGENT_PROVIDER", "codex_cli")
    monkeypatch.setenv("AMOS_CODEX_MODEL", "gpt-cli-test")
    monkeypatch.setattr("amos.agent.live_agent.shutil.which", lambda name: "/usr/local/bin/codex")
    cli_provider = provider_from_env()
    assert isinstance(cli_provider, CodexCLIProvider)
    assert cli_provider.model == "gpt-cli-test"


class RepairingProvider:
    provider_name = "test_repair"
    model = "test-live-agent"

    def __init__(self) -> None:
        self.phases: list[str] = []

    def complete(self, prompt: str, *, phase: str, response_format: str = "text") -> ProviderResponse:
        self.phases.append(phase)
        if phase == "analysis_plan":
            text = json.dumps(
                {
                    "required_memory_types": [
                        "semantic_definition",
                        "schema",
                        "stream_state",
                        "document",
                        "feedback",
                        "permission_policy",
                    ],
                    "query_kinds": ["summary", "concentration", "timeseries"],
                    "chart_kinds": ["failure_rate_timeseries"],
                    "provenance_level": 3,
                }
            )
        elif phase == "sql_proposal":
            text = json.dumps(
                {
                    "queries": [
                        {
                            "kind": "summary",
                            "sql": (
                                "SELECT failure_reason, COUNT(*) AS failures "
                                "FROM payment_events "
                                f"WHERE event_time >= TIMESTAMP '{PAYMENT_WINDOW_START}' "
                                "GROUP BY failure_reason"
                            ),
                        },
                        {"kind": "concentration", "sql": payment_failure_concentration_sql()},
                        {"kind": "timeseries", "sql": payment_failure_timeseries_sql()},
                    ]
                }
            )
        elif phase == "sql_repair":
            text = json.dumps({"sql": payment_failure_summary_sql()})
        else:
            text = "Use the repaired, verified SQL results and keep causal claims under review."
        return ProviderResponse(
            provider=self.provider_name,
            model=self.model,
            text=text,
            raw_request={"phase": phase, "prompt": prompt, "response_format": response_format},
            raw_response={"text": text},
            usage={"input_tokens": len(prompt.split()), "output_tokens": len(text.split())},
        )


class OutputContractProvider:
    provider_name = "test_output_contract"
    model = "test-live-agent"

    def __init__(self) -> None:
        self.phases: list[str] = []

    def complete(self, prompt: str, *, phase: str, response_format: str = "text") -> ProviderResponse:
        self.phases.append(phase)
        if phase == "analysis_plan":
            text = json.dumps(
                {
                    "required_memory_types": [
                        "semantic_definition",
                        "schema",
                        "stream_state",
                        "document",
                        "feedback",
                        "permission_policy",
                    ],
                    "query_kinds": ["summary", "concentration", "timeseries"],
                    "chart_kinds": ["failure_rate_timeseries"],
                    "provenance_level": 3,
                }
            )
        elif phase == "sql_proposal":
            summary_without_period = payment_failure_summary_sql().replace("      period,\n", "", 1)
            text = json.dumps(
                {
                    "queries": [
                        {"kind": "summary", "sql": summary_without_period},
                        {"kind": "concentration", "sql": payment_failure_concentration_sql()},
                        {"kind": "timeseries", "sql": payment_failure_timeseries_sql()},
                    ]
                }
            )
        elif phase == "sql_repair":
            text = json.dumps({"sql": payment_failure_summary_sql()})
        else:
            text = "Use verified results and keep causal claims under human review."
        return ProviderResponse(
            provider=self.provider_name,
            model=self.model,
            text=text,
            raw_request={"phase": phase, "prompt": prompt, "response_format": response_format},
            raw_response={"text": text},
            usage={"input_tokens": len(prompt.split()), "output_tokens": len(text.split())},
        )
