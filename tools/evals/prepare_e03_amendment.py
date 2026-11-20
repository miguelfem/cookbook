"""Prepare only unattempted E03 cells for an already approved current adapter."""

from __future__ import annotations

import hashlib
import json
import argparse
import subprocess
from pathlib import Path

from prepare_e03 import manifest, write

RELATIVE_DIR = Path("cookbook/.local/api-inputs")
AMENDMENT = Path(__file__).resolve().parents[2] / "docs/evaluations/fixtures/e03/amendment-1.json"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project-root", required=True, type=Path,
                        help="Existing disposable E03 evaluation directory")
    parser.add_argument("--input-root", required=True, type=Path,
                        help="Explicit adapter project root with ignored cookbook/.local/api-inputs")
    args = parser.parse_args()
    project = args.project_root.resolve()
    input_root = args.input_root.resolve()
    amendment_sha = hashlib.sha256(AMENDMENT.read_bytes()).hexdigest()
    mpath = project / "e03-openrouter-current-process.json"
    manifest_sha = write(mpath, manifest("eval_openrouter_current", "openai/gpt-4.1-mini"))
    source = json.loads((project / "e03-index.json").read_text())["dispatches"]
    prepared = []
    for item in source:
        if item["provider"] != "openrouter" or (item["case_id"], item["rep"]) == ("s01", 1):
            continue
        raw = Path(item["payload_path"]).read_bytes()
        payload_sha = hashlib.sha256(raw).hexdigest()
        if payload_sha != item["payload_sha256"]:
            raise ValueError("frozen payload changed")
        relative = RELATIVE_DIR / f"cookbook-e03-20260922-{item['case_id']}-r{item['rep']}-{payload_sha[:12]}.json"
        ignored = subprocess.run(["git", "-C", str(input_root), "check-ignore", "-q", str(relative)])
        if ignored.returncode != 0:
            raise ValueError("payload directory is not ignored")
        destination = input_root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as stream:
            stream.write(raw)
        prepared.append({"provider": "openrouter", "case_id": item["case_id"],
                         "repetition": item["rep"], "relative_payload_path": str(relative),
                         "payload_sha256": payload_sha, "request_key": f"e03-am1-or-{item['case_id']}-r{item['rep']}"})
    index = {"amendment_sha256": amendment_sha, "manifest_path": str(mpath),
             "manifest_sha256": manifest_sha, "dispatches": prepared}
    write(project / "e03-amended-index.json", index)
    print(json.dumps({"amendment_sha256": amendment_sha,
                      "manifest_sha256": manifest_sha, "prepared_cells": len(prepared)}))


if __name__ == "__main__":
    main()
