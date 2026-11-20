#!/usr/bin/env python3
"""Measure 4/32/64 local workers in disposable APR state, without models or providers.

Each arm runs 64 synthetic native jobs with exact JSON payloads. Setup, raw
traces, worker timestamps, CLI status samples, and state checks stay under a
new private --output directory. Never point this at an existing state store.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import time

from measure_batch_workers import Mcp, cli, digest, ready, save


COUNT = 64
POLICIES = (4, 32, 64)
WORKER = """import json,sys,time
from pathlib import Path
r=json.load(sys.stdin)['input']
start=time.monotonic_ns()
directory=Path(r['marker_dir'])
(directory / ('started-' + r['id'])).write_text('started')
deadline=time.monotonic()+20
while not (directory / 'release').exists():
    if time.monotonic() > deadline: raise RuntimeError('synthetic concurrency gate timed out')
    time.sleep(0.02)
time.sleep(r['delay_ms']/1000)
print(json.dumps({'protocol_version':'0.1','output':{'id':r['id'],'payload':r['payload'],'start_ns':start,'end_ns':time.monotonic_ns()}}))
"""


def jobs() -> list[dict]:
    samples = [
        {"answer": "Ada", "evidence": "  preserve spaces  "},
        {"answer": 7, "evidence": "line one\nline two"},
        {"answer": "Mérida / 東京", "evidence": 'quote: "yes"'},
        {"answer": [True, False, None], "evidence": "exact JSON"},
    ]
    return [
        {"id": f"job-{index:02}", "payload": {"index": index, **samples[index % len(samples)]},
         "delay_ms": 2400 if index == 0 else 550 + (index % 4) * 80}
        for index in range(COUNT)
    ]


def recipe() -> dict:
    item = {"type": "object", "required": ["id", "payload", "delay_ms"],
            "properties": {"id": {"type": "string"}, "payload": {"type": "object"},
                           "delay_ms": {"type": "integer"}}, "additionalProperties": False}
    return {
        "schema_version": "0.2", "name": "pool.synthetic64",
        "description": "Synthetic local worker pool measurement.",
        "input_schema": {"type": "object", "required": ["jobs", "marker_dir", "cap"],
                         "properties": {"jobs": {"type": "array", "minItems": COUNT,
                                                 "maxItems": COUNT, "items": item},
                                        "marker_dir": {"type": "string"},
                                        "cap": {"type": "integer"}},
                         "additionalProperties": False},
        "output_schema": {"type": "object"},
        "config": {"schema": {"type": "object", "additionalProperties": False},
                   "defaults": {}, "run_override_keys": []},
        "capabilities": [{"kind": "script", "language": "python"}],
        "limits": {"max_step_ms": 30000, "max_artifact_bytes": 1048576,
                   "max_provider_calls": 0, "max_agent_requests": 0,
                   "max_step_instances": 128},
        "files": [{"path": "main.py", "sha256": hashlib.sha256(WORKER.encode()).hexdigest()}],
        "steps": [{"id": "work", "kind": "script", "entrypoint": "main.py",
                   "effect": "write", "needs": [], "retry": {"max_attempts": 1},
                   "foreach": {"items": {"input": "/jobs"}, "key": "/id",
                               "max_items": COUNT, "concurrency": COUNT},
                   "inputs": {"id": {"item": "/id"}, "payload": {"item": "/payload"},
                              "delay_ms": {"item": "/delay_ms"},
                              "marker_dir": {"input": "/marker_dir"},
                              "cap": {"input": "/cap"}},
                   "result_schema": {"type": "object", "required": ["id", "payload",
                                                                  "start_ns", "end_ns"]}}],
        "output": {"results": {"step": "work", "pointer": "/items"}},
    }


def check_result(result: dict, expected: list[dict], cap: int) -> dict:
    assert result["state"] == "succeeded", result
    assert set(result["output"]) == {"results"}, result["output"]
    rows = result["output"]["results"]
    assert len(rows) == COUNT
    intervals = []
    for row, source in zip(rows, expected):
        assert set(row) == {"key", "output"}
        assert row["key"] == source["id"]
        output = row["output"]
        assert set(output) == {"id", "payload", "start_ns", "end_ns"}
        assert output["id"] == source["id"] and output["payload"] == source["payload"]
        assert output["end_ns"] > output["start_ns"]
        intervals.extend(((output["start_ns"], 1), (output["end_ns"], -1)))
    active = peak = 0
    for _, change in sorted(intervals):
        active += change
        peak = max(peak, active)
    assert active == 0 and peak == cap, (active, peak, cap)
    refilled_before_slow_end = None
    if cap < COUNT:
        refilled_before_slow_end = rows[cap]["output"]["start_ns"] < rows[0]["output"]["end_ns"]
        assert refilled_before_slow_end, "free slot waited for the slow first worker"
    return {"interval_peak": peak, "refilled_before_slow_end": refilled_before_slow_end,
            "first_start_ns": rows[0]["output"]["start_ns"],
            "slow_end_ns": rows[0]["output"]["end_ns"],
            "next_start_ns": rows[cap]["output"]["start_ns"] if cap < COUNT else None}


def run_arm(binary: Path, output: Path, cap: int, expected: list[dict]) -> dict:
    setup_start = time.perf_counter()
    output.mkdir(mode=0o700)
    project, state = output / "project", output / "runtime"
    project.mkdir(mode=0o700)
    state.mkdir(mode=0o700)
    marker_dir = output / "markers"
    marker_dir.mkdir(mode=0o700)
    (project / "main.py").write_text(WORKER)
    save(project / "process.json", recipe())
    project_id = cli(binary, state, "project", "attach", project)["project_id"]
    assignment = cli(binary, state, "assignment", "create", "--project", project_id,
                     "--name", "Synthetic worker pool")["assignment_id"]
    attachment = state / "attachments/author.json"
    cli(binary, state, "attachment", "create", "--assignment", assignment,
        "--mode", "author", "--output-file", attachment)
    cli(binary, state, "execution", "grant-python", "--assignment", assignment,
        "--interpreter", Path(sys.executable).resolve())
    policy = cli(binary, state, "execution", "worker-policy-show")
    policy["max_workers"] = cap
    policy["max_local_workers"] = cap
    save(output / "policy.json", policy)
    applied = cli(binary, state, "execution", "worker-policy-set", "--policy-file",
                  output / "policy.json", "--expected-revision", policy["revision"])
    assert applied["max_workers"] == cap and applied["max_local_workers"] == cap
    log = (output / "service.log").open("x")
    service = subprocess.Popen([str(binary), "--state-dir", str(state), "service"],
                               stdout=log, stderr=log, start_new_session=True)
    mcp = None
    samples = []
    try:
        ready(binary, state, service)
        mcp = Mcp(binary, state, attachment, log)
        process_ref = mcp.tool("apr_manage", {"action": "process.register",
                                               "path": "process.json"})["process_ref"]
        setup_ms = round((time.perf_counter() - setup_start) * 1000, 3)
        save(output / "setup.json", {"setup_ms": setup_ms, "process_ref": process_ref,
                                     "policy": applied, "binary_sha256": digest(binary)})
        workload = {"jobs": expected, "marker_dir": str(marker_dir), "cap": cap}
        measured = time.perf_counter()
        result = mcp.tool("apr_run", {"mode": "start", "process_ref": process_ref,
                                      "input": workload, "request_key": f"pool-{cap}",
                                      "wait_ms": 0, "result_view": "compact"})
        run_id = result["run_id"]
        deadline = time.monotonic() + 45
        released = False
        while result["state"] in ("queued", "running", "waiting_for_child"):
            assert time.monotonic() < deadline, f"worker arm {cap} timed out: {result}"
            status = cli(binary, state, "execution", "worker-status")
            samples.append({"elapsed_ms": round((time.perf_counter() - measured) * 1000, 3),
                            "running": status["running"], "running_local": status["running_local"],
                            "queued": status["queued"]})
            if not released and sum(1 for _ in marker_dir.glob("started-*")) >= cap:
                (marker_dir / "release").write_text("release")
                released = True
            time.sleep(0.04)
            result = mcp.tool("apr_inspect", {"view": "run", "run_id": run_id,
                                               "detail": True})
        wall_ms = round((time.perf_counter() - measured) * 1000, 3)
        proof = check_result(result, expected, cap)
        status_peak = max(sample["running"] for sample in samples)
        assert status_peak == cap, (status_peak, cap)
        save(output / "status-samples.json", samples)
        save(output / "run-output.json", result["output"])
        with sqlite3.connect(f"file:{state / 'state.sqlite3'}?mode=ro", uri=True) as db:
            counts = {table: db.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
                      for table in ("runs", "effect_intents", "billing_calls")}
            states = dict(db.execute("SELECT status,count(*) FROM effect_intents GROUP BY status"))
            assert counts == {"runs": 1, "effect_intents": COUNT, "billing_calls": 0}, counts
            assert states == {"succeeded": COUNT}, states
            assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            assert not db.execute("PRAGMA foreign_key_check").fetchall()
        save(output / "state-check.json", {"counts": counts, "effect_states": states})
        return {"policy": cap, "quality_pass": True, "setup_ms": setup_ms,
                "wall_ms": wall_ms, "status_peak": status_peak,
                "initial_run_state": samples[0]["running"] if samples else None,
                "model_tokens": None, "provider_calls": 0, **proof}
    finally:
        if mcp:
            save(output / "mcp-trace.json", mcp.events)
            mcp.close()
        service.terminate()
        try:
            service.wait(timeout=5)
        except subprocess.TimeoutExpired:
            service.kill()
            service.wait(timeout=3)
        log.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    os.chmod(args.output, 0o700)
    expected = jobs()
    protocol = {"version": 1, "policies": POLICIES, "jobs": expected,
                "worker_sha256": hashlib.sha256(WORKER.encode()).hexdigest(),
                "binary_sha256": digest(binary), "helper_sha256": digest(__file__),
                "imported_helper_sha256": digest(Path(__file__).with_name("measure_batch_workers.py")),
                "setup_boundary": "Fresh private state, assignment, grant, policy, service and process registration precede measured run.",
                "quality": "Exact complete id/payload and monotonic worker intervals for all 64 items; SQLite integrity and effect counts.",
                "measurement": "Local native Python only. CLI status sampling and MCP inspection are included in measured wall time. No models, providers, token claims, or normal user state."}
    save(args.output / "protocol.json", protocol)
    (args.output / "protocol.sha256").write_text(digest(args.output / "protocol.json") + "\n")
    rows = []
    for cap in POLICIES:
        try:
            row = run_arm(binary, args.output / f"workers-{cap}", cap, expected)
            rows.append(row)
            save(args.output / "metrics.json", rows)
            print(json.dumps(row), flush=True)
        except Exception as error:
            save(args.output / "failure.json", {"policy": cap, "type": type(error).__name__,
                                                "message": str(error)})
            raise
    save(args.output / "decision.json", {"acceptance_pass": all(row["quality_pass"] for row in rows),
                                         "measured_jobs_per_policy": COUNT,
                                         "model_tokens": None, "provider_calls": 0})


if __name__ == "__main__":
    main()
