#!/usr/bin/env python3
"""Test-only Gemini generateContent fixture for the packaged HTTP rehearsal."""

from __future__ import annotations

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


PLAN_SCHEMA_VERSION = "amos.plan-proposal.v1"
NARRATIVE_SCHEMA_VERSION = "amos.narrative-plan.v1"


def plan_response() -> dict[str, object]:
    all_rows = (
        "event_date >= '2026-07-13' AND event_date < '2026-07-27' "
        "AND segment = 'SMB' AND environment = 'production' "
        "AND is_test_account = 0"
    )
    current = (
        "event_date >= '2026-07-20' AND event_date < '2026-07-27' "
        "AND segment = 'SMB' AND environment = 'production' "
        "AND is_test_account = 0"
    )
    return {
        "schema_version": PLAN_SCHEMA_VERSION,
        "summary": "Compare SMB churn, its largest concentration, and daily trend.",
        "steps": [
            {
                "analysis_kind": "rate_comparison",
                "purpose": "Compare current and prior week SMB logo churn",
                "sql": (
                    "SELECT CASE WHEN event_date >= '2026-07-20' "
                    "THEN 'current' ELSE 'baseline' END AS period, "
                    "SUM(churned) AS churned_accounts, "
                    "COUNT(DISTINCT account_id) AS eligible_accounts, "
                    "CAST(SUM(churned) AS REAL)/COUNT(DISTINCT account_id) "
                    f"AS churn_rate FROM subscription_events WHERE {all_rows} "
                    "GROUP BY period ORDER BY period"
                ),
                "relations": ["subscription_events"],
                "expected_columns": [
                    "period",
                    "churned_accounts",
                    "eligible_accounts",
                    "churn_rate",
                ],
            },
            {
                "analysis_kind": "concentration",
                "purpose": "Find the largest churn concentration",
                "sql": (
                    "SELECT plan_tier, churn_type, "
                    "SUM(churned) AS churned_accounts, "
                    "COUNT(DISTINCT account_id) AS eligible_accounts, "
                    "CAST(SUM(churned) AS REAL)/COUNT(DISTINCT account_id) "
                    f"AS churn_rate FROM subscription_events WHERE {current} "
                    "GROUP BY plan_tier,churn_type "
                    "ORDER BY churned_accounts DESC LIMIT 10"
                ),
                "relations": ["subscription_events"],
                "expected_columns": [
                    "plan_tier",
                    "churn_type",
                    "churned_accounts",
                    "eligible_accounts",
                    "churn_rate",
                ],
            },
            {
                "analysis_kind": "timeseries",
                "purpose": "Show the daily churn trend",
                "sql": (
                    "SELECT event_date AS day, "
                    "CAST(SUM(churned) AS REAL)/COUNT(DISTINCT account_id) "
                    f"AS churn_rate FROM subscription_events WHERE {all_rows} "
                    "GROUP BY event_date ORDER BY event_date"
                ),
                "relations": ["subscription_events"],
                "expected_columns": ["day", "churn_rate"],
            },
        ],
    }


def narrative_response(payload: dict[str, object]) -> dict[str, object]:
    permitted = payload["permitted_context"]
    assert isinstance(permitted, list)
    ids = {
        item["logical_key"]: item["object_id"]
        for item in permitted
        if isinstance(item, dict)
    }
    return {
        "schema_version": NARRATIVE_SCHEMA_VERSION,
        "title": "SMB churn review",
        "executive_summary": (
            "Churn worsened. The largest verified concentration is "
            "{{fact:concentration.top}} The pricing email is temporally "
            "associated but not proven causal."
        ),
        "finding_order": [
            "metric.rate_change",
            "concentration.top",
            "trend.daily",
        ],
        "sections": [
            {
                "heading": "What changed",
                "fact_ids": ["metric.rate_change", "trend.daily"],
                "commentary": "The verified movement merits investigation.",
            }
        ],
        "judgment_claims": [
            {
                "claim_type": "causal",
                "text": "The pricing email may have contributed to the increase.",
                "support_fact_ids": ["metric.rate_change"],
                "support_memory_ids": [ids["document:pricing_email_launch"]],
                "review_required": True,
            },
            {
                "claim_type": "operational_recommendation",
                "text": (
                    "Annotate the dashboard with a non-causal warning while "
                    "freshness and cause are reviewed."
                ),
                "support_fact_ids": [
                    "metric.rate_change",
                    "trend.daily",
                ],
                "support_memory_ids": [
                    ids["snapshot:subscription_events:2026-07-27"],
                    ids["policy:review:subscriptions"],
                ],
                "review_required": True,
            },
        ],
        "slide_outline": [
            {
                "title": "SMB churn increased this week",
                "fact_ids": ["metric.rate_change", "trend.daily"],
            }
        ],
    }


class Handler(BaseHTTPRequestHandler):
    server_version = "AMOSMockGemma/1"

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/health":
            self.send_response(200)
            self.end_headers()
            return
        self.send_error(404)

    def do_POST(self) -> None:  # noqa: N802
        print(f"fixture request {self.path}", flush=True)
        length = int(self.headers.get("content-length", "0"))
        envelope = json.loads(self.rfile.read(length))
        prompt = envelope["contents"][0]["parts"][0]["text"]
        request = json.loads(prompt)
        system_instruction = envelope["systemInstruction"]["parts"][0]["text"]
        generation_config = envelope["generationConfig"]
        assert generation_config["thinkingConfig"]["thinkingLevel"] == "minimal"
        if "planning proposal model" in system_instruction:
            purpose = "plan"
        elif "narrative proposal model" in system_instruction:
            purpose = "narrative"
        else:
            self.send_error(400)
            return
        print(f"fixture purpose {purpose}", flush=True)
        if purpose == "plan":
            output = plan_response()
        elif purpose == "narrative":
            output = narrative_response(request)
        else:
            self.send_error(400)
            return
        response = {
            "candidates": [
                {"content": {"parts": [{"text": json.dumps(output)}]}}
            ],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 50,
            },
            "responseId": f"fixture-{purpose.lower()}",
        }
        body = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        print(f"fixture response {purpose}", flush=True)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
