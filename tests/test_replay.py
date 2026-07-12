from __future__ import annotations

from amos.agent.controller import run_amos_task
from amos.memory.models import User
from amos.provenance.replay import replay_artifact


def test_replay_succeeds_for_generated_artifact(seeded: None) -> None:
    result = run_amos_task(
        "Why did payment failure rate increase over the last six hours?",
        User(id="analyst_001", permissions=["analytics", "payments"]),
        provenance_level=3,
    )
    replay = replay_artifact(result.artifact_id)
    assert replay.status == "pass"
