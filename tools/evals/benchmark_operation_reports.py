#!/usr/bin/env python3
"""Compare inline and reference-only reports against an independent synthetic oracle."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time

from measure_batch_workers import Mcp, cli, ready, save


EXAMPLE = Path(__file__).resolve().parents[2] / "examples" / "operation-reports"


def load_fixtures():
    spec = importlib.util.spec_from_file_location("operation_datasets", EXAMPLE / "datasets.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("Unable to load the synthetic fixtures")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def measure(mcp, binary, state, project, output):
    fixtures = load_fixtures()
    evidence = []
    for task in fixtures.TASKS:
        versions = {}
        for manifest in (task, f"{task}-brief"):
            versions[manifest] = mcp.tool("apr_manage", {
                "action": "process.register", "path": f"{manifest}.json"
            })["process_ref"]
        for count in (*fixtures.SIZES, 300):
            csv_file, counts, expected, digest = fixtures.fixture(task, count, project)
            reference = mcp.tool("apr_manage", {
                "action": "artifact.import", "path": csv_file.name, "media_type": "text/csv"
            })
            modes = ("brief",) if count == 300 else ("inline", "report")
            for mode in modes:
                inputs = {"source": reference}
                if mode != "brief":
                    inputs["include_rows"] = mode == "inline"
                started = time.perf_counter()
                result = mcp.tool("apr_run", {
                    "process_ref": versions[f"{task}-brief" if mode == "brief" else task],
                    "mode": "start", "wait_ms": 5000, "result_view": "output",
                    "input": inputs, "request_key": f"{task}-{count}-{mode}"
                })
                elapsed = time.perf_counter() - started
                assert result["state"] == "succeeded" and result["output_complete"] is True
                reply_bytes = len(fixtures.canonical(mcp.events[-1]["response"]["result"]))
                observed = result["output"]
                report = observed["report"]
                assert observed["summary"] == counts and report["sha256"] == digest
                assert observed.get("rows") == (expected if mode == "inline" else None)
                if mode == "brief":
                    assert set(observed) == {"summary", "report"}
                report_ref = output / f"{task}-{count}-{mode}-ref.json"
                report_file = output / f"{task}-{count}-{mode}.json"
                save(report_ref, report)
                cli(binary, state, "artifact", "read", "--reference-file", report_ref,
                    "--output-file", report_file)
                assert report_file.read_bytes() == fixtures.canonical({"rows": expected})
                evidence.append({"recipe": task, "rows": count, "mode": mode, "reply_bytes": reply_bytes,
                                 "report_bytes": report["bytes"], "seconds": elapsed, "exact": True})
    return evidence


def run(binary, output):
    output.mkdir(mode=0o700)
    project, state = output / "project", output / "runtime"
    shutil.copytree(EXAMPLE, project)
    project_id = cli(binary, state, "project", "attach", project)["project_id"]
    assignment = cli(binary, state, "assignment", "create", "--project", project_id,
                     "--name", "Synthetic operation reports")["assignment_id"]
    attachment = state / "attachments" / "author.json"
    cli(binary, state, "attachment", "create", "--assignment", assignment, "--mode", "author",
        "--output-file", attachment)
    cli(binary, state, "execution", "grant-python", "--assignment", assignment,
        "--interpreter", Path(sys.executable).resolve())
    with (output / "service.log").open("x") as log:
        service = subprocess.Popen([binary, "--state-dir", state, "service"], stdout=log, stderr=log)
        try:
            ready(binary, state, service)
            mcp = Mcp(binary, state, attachment, log)
            try:
                evidence = measure(mcp, binary, state, project, output)
            finally:
                mcp.close()
        finally:
            service.terminate()
            try:
                service.wait(timeout=5)
            except subprocess.TimeoutExpired:
                service.kill()
                service.wait()
    save(output / "metrics.json", evidence)
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apr", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New private directory")
    args = parser.parse_args()
    run(args.apr.resolve(strict=True), args.output.resolve())
