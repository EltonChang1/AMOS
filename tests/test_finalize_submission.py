from __future__ import annotations

import hashlib
import json

from amos.evaluation.finalize_submission import finalize_submission_bundle


def test_finalize_submission_hashes_manuscript_and_refreshes_bundle(tmp_path) -> None:
    run_dir = tmp_path / "run"
    evaluation = run_dir / "artifacts" / "evaluation"
    evaluation.mkdir(parents=True)
    (evaluation / "bundle_manifest.json").write_text(
        json.dumps({"status": "pass", "artifact_count": 0, "artifact_bytes": 0}),
        encoding="utf-8",
    )
    (evaluation / "source_manifest.json").write_text('{"source_tree_sha256":"abc"}', encoding="utf-8")
    (evaluation / "PAPER_RESULTS.md").write_text("# Results\n", encoding="utf-8")
    tex = tmp_path / "paper.tex"
    pdf = tmp_path / "paper.pdf"
    tex.write_text("paper", encoding="utf-8")
    pdf.write_bytes(b"%PDF-test")

    result = finalize_submission_bundle(run_dir, manuscript_tex=tex, manuscript_pdf=pdf)
    assert result["status"] == "pass"
    assert result["artifact_count"] >= 4
    assert (evaluation / "submission" / "paper.tex").exists()
    assert (evaluation / "submission" / "paper.pdf").exists()
    submission = json.loads((evaluation / "submission" / "submission_manifest.json").read_text())
    assert {item["path"] for item in submission["files"]} == {
        "submission/paper.tex",
        "submission/paper.pdf",
    }
    rendered = (evaluation / "bundle_manifest.json").read_bytes()
    digest = (evaluation / "bundle_manifest.sha256").read_text().split()[0]
    assert digest == hashlib.sha256(rendered).hexdigest()
