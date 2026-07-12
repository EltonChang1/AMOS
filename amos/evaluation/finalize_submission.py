"""Finalize a completed paper bundle with the exact manuscript and PDF."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from amos.evaluation.paper_bundle import _artifact_manifest


def finalize_submission_bundle(
    run_dir: str | Path,
    *,
    manuscript_tex: str | Path,
    manuscript_pdf: str | Path,
    supporting_docs: list[str | Path] | None = None,
) -> dict[str, Any]:
    root = Path(run_dir).resolve()
    evaluation = root / "artifacts" / "evaluation"
    bundle_path = evaluation / "bundle_manifest.json"
    if not bundle_path.exists():
        raise FileNotFoundError(f"Completed bundle manifest is missing: {bundle_path}")
    bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    if bundle.get("status") != "pass":
        raise ValueError("Only a passing paper bundle may be finalized for submission.")

    submission = evaluation / "submission"
    submission.mkdir(parents=True, exist_ok=True)
    copied = []
    for source_value in [manuscript_tex, manuscript_pdf, *(supporting_docs or [])]:
        source = Path(source_value).resolve()
        if not source.exists() or not source.is_file():
            raise FileNotFoundError(f"Submission file is missing: {source}")
        destination = submission / source.name
        shutil.copy2(source, destination)
        copied.append(_file_record(destination, evaluation))

    submission_manifest = {
        "finalized_at": datetime.now(timezone.utc).isoformat(),
        "files": copied,
        "evaluation_source_manifest": str(evaluation / "source_manifest.json"),
        "paper_results": str(evaluation / "PAPER_RESULTS.md"),
        "evidence_boundary": (
            "The manuscript is finalized after evaluation. The source manifest hashes evaluation code/tests; "
            "this manifest separately hashes the submitted manuscript, PDF, and supporting documents."
        ),
    }
    submission_manifest_path = submission / "submission_manifest.json"
    submission_manifest_path.write_text(
        json.dumps(submission_manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    artifact_manifest = _artifact_manifest(root / "artifacts")
    artifact_manifest_path = evaluation / "artifact_manifest.json"
    artifact_manifest_path.write_text(
        json.dumps(artifact_manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    bundle.update(
        {
            "finalized_at": submission_manifest["finalized_at"],
            "artifact_count": artifact_manifest["artifact_count"],
            "artifact_bytes": artifact_manifest["total_bytes"],
            "submission_manifest": str(submission_manifest_path),
            "manuscript_tex": str(submission / Path(manuscript_tex).name),
            "manuscript_pdf": str(submission / Path(manuscript_pdf).name),
        }
    )
    rendered = json.dumps(bundle, indent=2, sort_keys=True) + "\n"
    bundle_path.write_text(rendered, encoding="utf-8")
    digest = hashlib.sha256(rendered.encode("utf-8")).hexdigest()
    (evaluation / "bundle_manifest.sha256").write_text(
        f"{digest}  bundle_manifest.json\n",
        encoding="utf-8",
    )
    return bundle


def _file_record(path: Path, root: Path) -> dict[str, Any]:
    data = path.read_bytes()
    return {
        "path": str(path.relative_to(root)),
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Attach and hash the final AMOS manuscript and PDF.")
    parser.add_argument("--run-dir", required=True)
    parser.add_argument("--tex", required=True)
    parser.add_argument("--pdf", required=True)
    parser.add_argument("--supporting-doc", action="append", default=[])
    args = parser.parse_args()
    print(
        json.dumps(
            finalize_submission_bundle(
                args.run_dir,
                manuscript_tex=args.tex,
                manuscript_pdf=args.pdf,
                supporting_docs=args.supporting_doc,
            ),
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
