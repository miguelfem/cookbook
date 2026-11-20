#!/usr/bin/env python3
"""Account-free E05 preparation and truthful provenance-gate reproduction.

Usage: python3 tools/evals/e05_improvement.py prepare|run --apr ABSOLUTE_INSTALLED_BINARY
All state and raw request/response material remains under ignored work/evals-2026-09-22/e05.
"""
import argparse
import copy
import hashlib
import json
import select
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "work/evals-2026-09-22/e05"
SPEC = ROOT / "docs/evaluations/fixtures/e05/protocol.json"
SOURCE = ROOT / "examples/recipe-improvement/baseline.json"
BIN: Path | None = None


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n")


def cases(cycle, split):
    result = []
    for number in range(1, 7):
        source = f"e05:{cycle}:{split}:{number}"
        value = {"answer": f"answer/{source}", "evidence": f"evidence/{source}"}
        result.append({"case_id": source, "source_id": source,
                       "input": value, "expected": value.copy(),
                       "provenance": "deterministic_specification_agent_evaluation"})
    return result


def cli(state, *arguments, failure_ok=False):
    process = subprocess.run([str(BIN), "--state-dir", str(state), *map(str, arguments)],
                             text=True, capture_output=True, timeout=45)
    if process.returncode and not failure_ok:
        raise RuntimeError(f"CLI {' '.join(map(str, arguments[:2]))}: {process.stderr.strip()}")
    try:
        value = json.loads(process.stdout or process.stderr)
    except json.JSONDecodeError:
        value = {"stdout": process.stdout.strip(), "stderr": process.stderr.strip()}
    return {"exit": process.returncode, "value": value}


def prepare():
    if (WORK / "freeze.json").exists():
        raise RuntimeError("E05 evidence is already frozen; prepare will not overwrite it")
    protocol = json.loads(SPEC.read_text())
    project = WORK / "project"
    project.mkdir(parents=True, exist_ok=True)
    baseline = json.loads(SOURCE.read_text())
    save(project / "baseline.json", baseline)
    good = json.loads((SOURCE.parent / "candidate.json").read_text())
    variants = {"simpler": good}
    wrong = copy.deepcopy(good)
    wrong["description"] = "E05 cheaper but wrong answer"
    wrong["steps"][0]["inputs"]["pointer"]["literal"] = "/evidence"
    variants["cheaper_wrong"] = wrong
    missing = copy.deepcopy(good)
    missing["description"] = "E05 evidence field incorrectly repeats answer"
    missing["output"]["evidence"] = {"step": "answer", "pointer": "/value"}
    variants["missing_evidence"] = missing
    unsupported = copy.deepcopy(good)
    unsupported["description"] = "E05 non-allowlisted pure builtin negative control"
    unsupported["steps"][0]["operation"] = "json.write"
    variants["unsupported"] = unsupported
    effect = copy.deepcopy(good)
    effect["description"] = "E05 ungranted write connector negative control"
    effect["capabilities"] = [{"kind": "connection", "name": "unapproved-write"}]
    effect["limits"]["max_provider_calls"] = 1
    effect["steps"][0] = {"id": "answer", "kind": "connector", "effect": "write",
                            "operation": "http.json", "connection": "unapproved-write",
                            "needs": [], "retry": {"max_attempts": 1},
                            "inputs": {"record": {"input": ""}},
                            "result_schema": {"type": "object"}}
    variants["effect"] = effect
    for cycle, manifest in variants.items():
        save(project / f"{cycle}.json", manifest)
    data = {cycle["id"]: {"policy": {"evaluator_version": "pure-v1",
                    "split_method": protocol["split_method"], "min_step_reduction": 1},
                    "development": cases(cycle["id"], "dev"),
                    "holdout": cases(cycle["id"], "held")}
            for cycle in protocol["cycles"]}
    save(WORK / "private-labels-and-cases.json", data)
    paths = [SPEC, SOURCE, SOURCE.parent / "candidate.json", Path(__file__),
             WORK / "private-labels-and-cases.json", *sorted(project.glob("*.json"))]
    lock = {str(path.relative_to(ROOT)): digest(path) for path in paths}
    save(WORK / "freeze.json", {"frozen_before_run": True, "sha256": lock,
                                "binary_sha256": digest(BIN), "labels_are_human": False})
    print(json.dumps({"prepared": True, "cycles": list(data),
                      "development_per_cycle": 6, "holdout_per_cycle": 6,
                      "freeze_sha256": digest(WORK / "freeze.json"),
                      "binary_sha256": digest(BIN)}))


class Mcp:
    def __init__(self, state, attachment):
        self.process = subprocess.Popen([str(BIN), "--state-dir", str(state), "mcp",
                                         "--attachment-file", str(attachment)],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=(WORK / "mcp-stderr-private.log").open("a"),
                                        text=True, bufsize=1)
        self.sequence = 0
        self.request("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                    "clientInfo": {"name": "e05-independent-preparation", "version": "1"}})
        self.process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        self.process.stdin.flush()

    def request(self, method, params):
        self.sequence += 1
        number = self.sequence
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": number,
                                              "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            ready, _, _ = select.select([self.process.stdout], [], [], 1)
            if ready:
                response = json.loads(self.process.stdout.readline())
                if response.get("id") == number:
                    return response
        raise RuntimeError(f"MCP {method} timed out")

    def tool(self, name, arguments):
        response = self.request("tools/call", {"name": name, "arguments": arguments})
        if "error" in response or response.get("result", {}).get("isError"):
            raise RuntimeError(f"MCP {name} failed: {response}")
        return response["result"]["structuredContent"]

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=5)


def run():
    frozen = json.loads((WORK / "freeze.json").read_text())
    for filename, checksum in frozen["sha256"].items():
        if digest(ROOT / filename) != checksum:
            raise RuntimeError(f"frozen material changed: {filename}")
    if digest(BIN) != frozen["binary_sha256"]:
        raise RuntimeError("installed binary changed after freeze")
    if frozen["labels_are_human"]:
        raise RuntimeError("this archival harness cannot claim human review or run a controlled evaluation")
    state = WORK / "state"
    project = WORK / "project"
    attached = cli(state, "project", "attach", project)["value"]
    assignment = cli(state, "assignment", "create", "--project", attached["project_id"],
                     "--name", "E05 synthetic evaluation")["value"]
    attachment = state / "attachments/e05-author.json"
    cli(state, "attachment", "create", "--assignment", assignment["assignment_id"],
        "--mode", "author", "--output-file", attachment)
    records = {"binary_sha256": frozen["binary_sha256"], "cycles": {},
               "labels": "deterministic_specification_agent_evaluation_not_human"}
    client = Mcp(state, attachment)
    try:
        base = client.tool("apr_manage", {"action": "process.register", "path": "baseline.json"})["process_ref"]
        initial = client.tool("apr_run", {"process_ref": base, "mode": "start",
                          "request_key": "e05-historical-run", "input": {"answer": "historic",
                                                                     "evidence": "historical-source"}})
        run_id = initial["run_id"]
        for _ in range(120):
            observed = client.tool("apr_inspect", {"view": "run", "run_id": run_id})
            if observed["state"] in ("succeeded", "failed"):
                break
            time.sleep(.1)
        if observed["state"] != "succeeded":
            raise RuntimeError(f"baseline evidence run failed: {observed['state']}")
        records["baseline_ref"] = base
        records["historical_run_id"] = run_id
        data = json.loads((WORK / "private-labels-and-cases.json").read_text())
        for cycle_id, case_set in data.items():
            candidate_file = project / f"{cycle_id}.json"
            entry = {"candidate_file_sha256": digest(candidate_file)}
            opened = client.tool("apr_manage", {"action": "improvement.open",
                "request_key": f"e05-{cycle_id}-v1", "process_ref": base, "run_ids": [run_id]})
            cycle = opened["cycle_id"]
            entry["cycle_id"] = cycle
            proposed = client.tool("apr_manage", {"action": "improvement.propose",
                "cycle_id": cycle, "candidate_path": f"{cycle_id}.json",
                "proposal": {"hypothesis": "one step can be removed or tested",
                    "change": f"frozen E05 {cycle_id} manifest", "benefit": "one pure step if supported",
                    "risks": "incorrect output, missing evidence, or unsupported effects",
                    "evidence_run_ids": [run_id]}, "review": {}})
            entry["candidate_ref"] = proposed["candidate_ref"]
            # Honest agent-derived labels are rejected by the installed runtime.
            # Never replace provenance with 'human' simply to obtain a report.
            attempted = client.request("tools/call", {"name": "apr_manage", "arguments":
                {"action": "improvement.freeze", "cycle_id": cycle, **case_set}})
            entry["freeze_rejected"] = bool(attempted.get("result", {}).get("isError"))
            entry["freeze_error"] = attempted.get("result", {}).get("structuredContent")
            if not entry["freeze_rejected"]:
                raise RuntimeError("non-human provenance was unexpectedly accepted; stop before evaluation")
            records["cycles"][cycle_id] = entry
            save(WORK / "raw-result-private.json", records)
    finally:
        client.close()
    print(json.dumps({"baseline_ref": records.get("baseline_ref"),
        "cycles": {name: {"freeze_rejected": item["freeze_rejected"],
                            "candidate_ref": item["candidate_ref"]}
                   for name, item in records["cycles"].items()},
        "evaluations": 0, "adoptions": 0}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "run"))
    parser.add_argument("--apr", required=True, type=Path,
                        help="Absolute installed APR executable for this disposable run")
    args = parser.parse_args()
    BIN = args.apr.resolve(strict=True)
    if not BIN.is_file():
        parser.error("--apr must name an executable file")
    {"prepare": prepare, "run": run}[args.action]()
