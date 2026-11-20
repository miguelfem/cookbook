#!/usr/bin/env python3
"""Compare two APR binaries with private, model-free prepared batch fixtures.

This explicitly grants a pinned Python interpreter only in disposable state.
It never reads the normal app store, creates client entries or calls providers.
Raw traces/state stay under --output, which must be a new private directory.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import select
import socket
import sqlite3
import subprocess
import sys
import time


WORKER = """import json,sys,time
r=json.load(sys.stdin)['input']
start=time.monotonic_ns()
time.sleep(0.35)
print(json.dumps({'protocol_version':'0.1','output':{'id':r['id'],'payload':r['payload'],'start_ns':start,'end_ns':time.monotonic_ns()}}))
"""
JOBS = {"jobs": [
    {"id": "owner", "payload": {"answer": "Ada", "evidence": "  preserve spaces  "}},
    {"id": "amount", "payload": {"answer": 7, "evidence": "line one\nline two"}},
    {"id": "label", "payload": {"answer": "Mérida / 東京", "evidence": 'quote: "yes"'}},
    {"id": "flags", "payload": {"answer": [True, False, None], "evidence": "exact JSON"}},
]}


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def cli(binary, state, *args):
    result = subprocess.run([str(binary), "--state-dir", str(state), *map(str, args)],
                            capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


class Mcp:
    def __init__(self, binary, state, attachment, log):
        self.process = subprocess.Popen(
            [str(binary), "--state-dir", str(state), "mcp", "--attachment-file", str(attachment)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log,
            text=True, bufsize=1)
        self.number = 0
        self.events = []
        self.call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                  "clientInfo": {"name": "prepared-batch-measurement", "version": "1"}})
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.process.stdin.flush()

    def call(self, method, params):
        self.number += 1
        request = {"jsonrpc": "2.0", "id": self.number, "method": method, "params": params}
        start = time.perf_counter()
        self.process.stdin.write(json.dumps(request, ensure_ascii=False) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if not select.select([self.process.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
                break
            response = json.loads(self.process.stdout.readline())
            if response.get("id") != self.number:
                continue
            self.events.append({"request": request, "response": response,
                                "elapsed_ms": round((time.perf_counter() - start) * 1000, 3)})
            if "error" in response:
                raise RuntimeError(f"MCP protocol error: {response['error']['code']}")
            return response["result"]
        raise TimeoutError(f"MCP {method} timed out")

    def tool(self, name, arguments):
        result = self.call("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise RuntimeError(f"Fixture tool failed: {name}: {result}")
        return result["structuredContent"]

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=3)


def ready(binary, state, service):
    identity = hashlib.sha256(os.fsencode(state.resolve())).hexdigest()[:32]
    endpoint = f"/tmp/apr-{os.geteuid()}/{identity}/rpc.sock"
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if service.poll() is not None:
            raise RuntimeError("fixture service exited")
        try:
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(0.1)
                client.connect(endpoint)
            return
        except OSError:
            time.sleep(0.025)
    raise TimeoutError("fixture service readiness")


def recipe(name, step, output, *, native=False, batch=False):
    item = {"type": "object", "additionalProperties": False, "required": ["id", "payload"],
            "properties": {"id": {"type": "string"}, "payload": {"type": "object"}}}
    schema = {"type": "object", "required": ["jobs"], "additionalProperties": False,
              "properties": {"jobs": {"type": "array", "minItems": 1, "maxItems": 4, "items": item}}} if batch else item
    value = {"schema_version": "0.2", "name": name, "description": "Synthetic batch worker measurement.",
             "input_schema": schema, "output_schema": {"type": "object"},
             "config": {"schema": {"type": "object", "additionalProperties": False}, "defaults": {}, "run_override_keys": []},
             "capabilities": [{"kind": "script", "language": "python"}] if native else ([{"kind": "builtin"}] if step["kind"] == "builtin" else []),
             "limits": {"max_step_ms": 2000, "max_artifact_bytes": 1048576, "max_provider_calls": 0,
                        "max_agent_requests": 0, "max_call_depth": 4, "max_child_runs": 8, "max_step_instances": 64},
             "steps": [step], "output": output}
    if native:
        value["files"] = [{"path": "main.py", "sha256": hashlib.sha256(WORKER.encode()).hexdigest()}]
    return value


def register_fixtures(mcp, project):
    def register(filename, value):
        save(project / filename, value)
        return mcp.tool("apr_manage", {"action": "process.register", "path": filename})["process_ref"]

    steps = {
        "native": {"id": "work", "kind": "script", "entrypoint": "main.py", "effect": "write", "needs": [],
                   "retry": {"max_attempts": 1}, "inputs": {"id": {"input": "/id"}, "payload": {"input": "/payload"}},
                   "result_schema": {"type": "object", "required": ["id", "payload", "start_ns", "end_ns"]}},
        "pure": {"id": "work", "kind": "builtin", "operation": "json.select", "effect": "pure", "needs": [],
                 "retry": {"max_attempts": 1}, "inputs": {"value": {"input": "/payload"}, "pointer": {"literal": ""}}},
    }
    refs = {}
    for kind, step in steps.items():
        output = {"id": {"input": "/id"}, "payload": {"step": "work", "pointer": "/value"}} if kind == "pure" else {"result": {"step": "work", "pointer": ""}}
        child = register(f"{kind}-child.json", recipe(f"batch.{kind}-child", step, output, native=kind == "native"))
        call = {"id": "jobs", "kind": "call", "effect": "control", "needs": [], "retry": {"max_attempts": 1},
                "foreach": {"items": {"input": "/jobs"}, "key": "/id", "max_items": 4, "concurrency": 4},
                "inputs": {"id": {"item": "/id"}, "payload": {"item": "/payload"}},
                "call": {"process_ref": child, "output_pointer": ""}}
        refs[f"composed-{kind}"] = register(f"composed-{kind}.json", recipe(f"batch.composed-{kind}", call, {"results": {"step": "jobs", "pointer": "/items"}}, batch=True))
    direct = dict(steps["native"], id="jobs", foreach=call["foreach"], inputs=call["inputs"])
    refs["direct-native"] = register("direct-native.json", recipe("batch.direct-native", direct, {"results": {"step": "jobs", "pointer": "/items"}}, native=True, batch=True))
    return refs


def check_output(case, output):
    assert set(output) == {"results"}
    rows = output["results"]
    assert len(rows) == len(JOBS["jobs"])
    events = []
    for row, expected in zip(rows, JOBS["jobs"]):
        assert set(row) == {"key", "output"}
        assert row["key"] == expected["id"]
        value = row["output"]
        if case == "composed-native":
            assert set(value) == {"result"}
            value = value["result"]
        assert {key: value[key] for key in ("id", "payload")} == expected
        if case.endswith("native"):
            assert set(value) == {"id", "payload", "start_ns", "end_ns"}
            assert value["end_ns"] > value["start_ns"]
            events.extend([(value["start_ns"], 1), (value["end_ns"], -1)])
        else:
            assert set(value) == {"id", "payload"}
    active = maximum = 0
    for _, change in sorted(events):
        active += change
        maximum = max(maximum, active)
    assert active == 0
    return maximum if events else None


def run_arm(binary, folder, pair, arm):
    started = time.perf_counter()
    folder.mkdir(mode=0o700)
    project, state = folder / "project", folder / "runtime"
    project.mkdir(mode=0o700)
    state.mkdir(mode=0o700)
    (project / "main.py").write_text(WORKER)
    project_id = cli(binary, state, "project", "attach", project)["project_id"]
    assignment = cli(binary, state, "assignment", "create", "--project", project_id, "--name", "Synthetic prepared batch")["assignment_id"]
    attachment = state / "attachments/author.json"
    cli(binary, state, "attachment", "create", "--assignment", assignment, "--mode", "author", "--output-file", attachment)
    cli(binary, state, "execution", "grant-python", "--assignment", assignment, "--interpreter", Path(sys.executable).resolve())
    log = (folder / "stderr.log").open("x")
    service = subprocess.Popen([str(binary), "--state-dir", str(state), "service"], stdout=log, stderr=log, start_new_session=True)
    mcp, rows = None, []
    try:
        ready(binary, state, service)
        mcp = Mcp(binary, state, attachment, log)
        refs = register_fixtures(mcp, project)
        contract = mcp.tool("apr_discover", {"guide": "overview"})
        if arm == "candidate":
            assert contract["runtime"]["features"]["parallel_execution"] is True
            assert contract["runtime"]["features"]["max_parallel_workers"] == 4
        save(folder / "setup.json", {"elapsed_ms": round((time.perf_counter() - started) * 1000, 3), "contract": contract})
        for case in ("composed-pure", "direct-native", "composed-native"):
            arguments = {"process_ref": refs[case], "input": JOBS, "request_key": f"{case}-{pair}",
                         "mode": "start", "wait_ms": 5000, "result_view": "compact"}
            trace_start = len(mcp.events)
            measured = time.perf_counter()
            result = mcp.tool("apr_run", arguments)
            initial_state = result["state"]
            run_id = result["run_id"]
            deadline = time.monotonic() + 15
            while result["state"] in ("queued", "running", "waiting_for_child") and time.monotonic() < deadline:
                time.sleep(0.1)
                result = mcp.tool("apr_inspect", {"view": "run", "run_id": run_id, "detail": True})
            elapsed_ms = round((time.perf_counter() - measured) * 1000, 3)
            assert result["state"] == "succeeded", result
            maximum = check_output(case, result["output"])
            measured_events = mcp.events[trace_start:]
            row = {"pair": pair, "arm": arm, "case": case, "quality_pass": True,
                   "initial_state": initial_state, "wall_ms": elapsed_ms, "peak_workers": maximum,
                   "caller_tool_calls": len(measured_events), "caller_inspections": len(measured_events) - 1,
                   "request_bytes": sum(len(encoded(e["request"])) for e in measured_events),
                   "response_bytes": sum(len(encoded(e["response"])) for e in measured_events),
                   "model_tokens": None, "provider_calls": 0}
            replay = mcp.tool("apr_run", arguments)
            assert replay["run_id"] == run_id and replay["output"] == result["output"]
            row["request_key_replay_pass"] = True
            rows.append(row)
            save(folder / "metrics.json", rows)
        with sqlite3.connect(f"file:{state / 'state.sqlite3'}?mode=ro", uri=True) as db:
            counts = {table: db.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
                      for table in ("runs", "effect_intents", "billing_calls", "saved_accounts")}
            assert counts == {"runs": 11, "effect_intents": 8, "billing_calls": 0, "saved_accounts": 0}, counts
            assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            assert not db.execute("PRAGMA foreign_key_check").fetchall()
            save(folder / "state-check.json", counts)
        return rows
    finally:
        if mcp:
            save(folder / "trace.json", mcp.events)
            mcp.close()
        service.terminate()
        try:
            service.wait(timeout=5)
        except subprocess.TimeoutExpired:
            service.kill()
            service.wait(timeout=3)
        log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pairs", type=int, default=3, choices=range(1, 6))
    args = parser.parse_args()
    args.output.mkdir(parents=True, mode=0o700, exist_ok=False)
    os.chmod(args.output, 0o700)
    binaries = {name: getattr(args, name).resolve() for name in ("baseline", "candidate")}
    protocol = {"version": 1, "pairs": args.pairs, "order": "AB, BA, AB (alternating)", "jobs": JOBS,
                "cases": ["composed-pure", "direct-native", "composed-native"], "wait_ms": 5000,
                "poll_ms_if_incomplete": 100, "python_sleep_ms": 350,
                "setup": "New private state, grant, service, MCP initialization and all registration excluded from reuse timing; retained separately.",
                "quality": "Exact complete id/payload including Unicode, whitespace and JSON types; timestamps are measured outputs.",
                "acceptance": "All outputs exact; candidate first call complete; native overlap between 2 and 4; same request key adds no effects.",
                "measurement": "Actual MCP tool boundaries; compact JSON byte counts are not model tokens. No real provider/model calls.",
                "binary_sha256": {name: digest(path) for name, path in binaries.items()},
                "helper_sha256": digest(__file__), "worker_sha256": hashlib.sha256(WORKER.encode()).hexdigest()}
    save(args.output / "protocol.json", protocol)
    (args.output / "protocol.sha256").write_text(digest(args.output / "protocol.json") + "\n")
    rows = []
    for pair in range(1, args.pairs + 1):
        order = ("baseline", "candidate") if pair % 2 else ("candidate", "baseline")
        for arm in order:
            try:
                rows.extend(run_arm(binaries[arm], args.output / f"pair-{pair}-{arm}", pair, arm))
            except Exception as error:
                save(args.output / "failure.json", {"pair": pair, "arm": arm, "type": type(error).__name__, "message": str(error)})
                raise
            save(args.output / "metrics.json", rows)
            print(json.dumps({"pair": pair, "arm": arm, "completed_cases": len(rows)}), flush=True)
    with (args.output / "metrics.csv").open("x", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    passed = all(row["quality_pass"] and (row["arm"] != "candidate" or
                 (row["caller_tool_calls"] == 1 and (row["peak_workers"] is None or 2 <= row["peak_workers"] <= 4))) for row in rows)
    save(args.output / "decision.json", {"acceptance_pass": passed, "cases": len(rows), "model_tokens": None})
    if not passed:
        raise SystemExit("Frozen batch acceptance did not pass; retain evidence.")


if __name__ == "__main__":
    main()
