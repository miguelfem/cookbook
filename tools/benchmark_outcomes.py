#!/usr/bin/env python3
"""Run synthetic saved processes through the real CLI and owned local service.

This is a runtime correctness/transport sample with canned caller answers. It
does not invoke models, measure human effort, or compare assignment economics.
"""

import argparse
from collections import Counter
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import socket
import statistics
import subprocess
import sys
import tempfile
import time

from benchmark_service import endpoint_for


DOMAINS = ("research-triage", "operations-intake", "software-verification")
BUDGET = 64 * 1024


class BenchmarkError(Exception):
    """Static diagnostics never expose private attachment or continuation tokens."""


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise BenchmarkError(message)


def environment():
    return {key: value for key, value in os.environ.items() if not key.startswith("APR_TEST_")}


def cli(binary, state, *arguments):
    result = subprocess.run(
        [str(binary), "--state-dir", str(state), *map(str, arguments)],
        stdin=subprocess.DEVNULL, capture_output=True, timeout=10, env=environment(),
    )
    require(result.returncode == 0, "An APR fixture setup or validation command failed.")
    require(len(result.stdout) <= BUDGET, "A fixture CLI response exceeded its allowance.")
    try:
        return json.loads(result.stdout)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise BenchmarkError("An APR fixture CLI command returned invalid JSON.") from error


class Client:
    def __init__(self, endpoint, token):
        self.endpoint, self.token = endpoint, token
        self.actions = Counter()
        self.request_bytes = self.response_bytes = self.sequence = 0

    def snapshot(self):
        return {"actions": dict(self.actions), "request_bytes": self.request_bytes, "response_bytes": self.response_bytes}

    def since(self, previous):
        return {"calls": sum(self.actions.values()) - sum(previous["actions"].values()),
                "actions": dict(self.actions - Counter(previous["actions"])),
                "request_bytes": self.request_bytes - previous["request_bytes"],
                "response_bytes": self.response_bytes - previous["response_bytes"]}

    def call(self, action, args):
        self.sequence += 1
        request_id = f"sample-{self.sequence}"
        frame = encoded({"protocol_version": "0.1", "request_id": request_id,
                         "attachment_token": self.token, "action": action, "args": args,
                         "response_budget": BUDGET}) + b"\n"
        require(len(frame) <= 1024 * 1024, "A fixture request exceeded the frame allowance.")
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            deadline = time.monotonic() + 5
            connection.settimeout(5)
            connection.connect(str(self.endpoint))
            connection.sendall(frame)
            received = bytearray()
            while b"\n" not in received:
                remaining = deadline - time.monotonic()
                require(remaining > 0, "The local service response timed out.")
                connection.settimeout(remaining)
                chunk = connection.recv(min(4096, BUDGET + 1 - len(received)))
                require(bool(chunk), "The local service closed an incomplete response.")
                received.extend(chunk)
                require(len(received) <= BUDGET, "The local service exceeded the response budget.")
        self.actions[action] += 1
        self.request_bytes += len(frame)
        self.response_bytes += len(received)
        try:
            response = json.loads(received)
        except (UnicodeError, json.JSONDecodeError) as error:
            raise BenchmarkError("The local service returned invalid JSON.") from error
        require(isinstance(response, dict) and response.get("protocol_version") == "0.1"
                and response.get("request_id") == request_id and "result" in response
                and "error" not in response, "The local service rejected a fixture request.")
        return response["result"]


def run_case(client, process_ref, case, iteration):
    started, before = time.perf_counter(), client.snapshot()
    arguments = {"mode": "start", "process_ref": process_ref, "input": case["input"],
                 "request_key": f"{case['label']}-{iteration}"}
    current = client.call("run", arguments)
    run_id = current["run_id"]
    answers, receipts = {}, {}
    deadline = time.monotonic() + 20
    while current["state"] != "succeeded":
        require(time.monotonic() < deadline, "A synthetic run did not complete within its deadline.")
        require(current["state"] in ("queued", "running", "waiting_for_agent"), "A synthetic run reached an unexpected state.")
        if current["state"] == "waiting_for_agent":
            request = current.get("request")
            require(isinstance(request, dict), "A synthetic handoff was not actionable.")
            step = request["step_instance_id"]
            require(step in case["caller_results"] and step not in answers, "A synthetic run repeated or invented a caller request.")
            submission = {"protocol_version": "0.1", "action": "submit", "run_id": run_id,
                          "request_id": request["request_id"], "step_instance_id": step,
                          "generation": request["generation"], "continuation_token": request["continuation_token"],
                          "submission_key": f"canned-{step}-{iteration}", "output": case["caller_results"][step],
                          "provenance": {"source": "synthetic_fixture", "client": "apr-outcomes-harness", "verification": "canned_answer_no_model"}}
            accepted = client.call("resume", submission)
            answers[step], receipts[step] = submission, accepted["receipt"]
            current = accepted["run"]
        else:
            time.sleep(0.01)
            current = client.call("inspect", {"view": "run", "run_id": run_id})
    elapsed_ms = (time.perf_counter() - started) * 1000
    execution_transport = client.since(before)
    validation_before = client.snapshot()
    require(current.get("output", {}).get("result") == case["expected_result"], "A synthetic run produced an incorrect joined result.")
    require(set(answers) == set(case["caller_results"]), "A synthetic run omitted a required caller result.")
    steps = current["steps"]
    require(sorted(step["id"] for step in steps if step["state"] == "skipped") == sorted(case["expected_skipped"]), "A synthetic run skipped an incorrect branch.")
    require(all(step["attempts"] == (0 if step["state"] == "skipped" else 1) for step in steps), "A synthetic run retried or failed to record selected work.")
    report = current["output"]["report"]
    preview = client.call("inspect", {"view": "artifact", "reference": report})
    require(preview.get("truncated") is False and isinstance(preview.get("text"), str), "A fixture report was missing or truncated.")
    payload = preview["text"].encode()
    require(sha256(payload) == report["sha256"] and len(payload) == report["bytes"], "A fixture report failed byte and digest verification.")
    require(json.loads(payload) == case["expected_result"], "A fixture report disagrees with its saved result.")
    repeated = client.call("run", arguments)
    require(repeated["run_id"] == run_id and repeated["steps"] == steps, "Replaying a request key changed the run or executed more steps.")
    for step, submission in answers.items():
        repeated = client.call("resume", submission)
        require(repeated["receipt"] == receipts[step], "Replaying a submission changed its durable receipt.")
    events, cursor = [], 0
    for _ in range(5):
        page = client.call("inspect", {"view": "events", "run_id": run_id, "after": cursor, "limit": 100})
        events.extend(page["events"])
        cursor = page["next_cursor"]
        if not page["truncated"]:
            break
    else:
        raise BenchmarkError("A fixture event stream exceeded its bounded allowance.")
    counts = Counter(event["kind"] for event in events)
    require(counts["decision.committed"] == 1 and counts["handoff.accepted"] == len(answers), "A run or submission replay duplicated a committed event.")
    return {"case": case["label"], "iteration": iteration,
            "phase": "first_saved_definition_run" if iteration == 0 else "saved_definition_reuse",
            "outcome_verified": True, "elapsed_ms": round(elapsed_ms, 3),
            "selected_route": case["expected_result"]["route"], "caller_handoffs": len(answers),
            "step_attempts": {step["id"]: step["attempts"] for step in steps},
            "artifact_count": 1, "artifact_bytes": report["bytes"], "artifact_sha256": report["sha256"],
            "event_count": len(events), "execution_transport": execution_transport,
            "verification_transport": client.since(validation_before),
            "model_tokens": None, "model_cost": None, "provider_cost": None}


def benchmark(binary, iterations, fixtures):
    started = time.perf_counter()
    binary_digest = hashlib.sha256()
    with binary.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            binary_digest.update(chunk)
    samples, definitions = [], []
    graceful = False
    with tempfile.TemporaryDirectory(prefix="apr-outcomes-") as temporary:
        root = Path(temporary)
        project, state = root / "project", root / "state"
        project.mkdir(mode=0o700)
        setup_started = time.perf_counter()
        trusted = cli(binary, state, "project", "attach", project)
        assignment = cli(binary, state, "assignment", "create", "--project", trusted["project_id"], "--name", "Synthetic domain outcomes")
        attachment_file = state / "attachments/benchmark.json"
        cli(binary, state, "attachment", "create", "--assignment", assignment["assignment_id"], "--mode", "author", "--output-file", attachment_file)
        attachment = json.loads(attachment_file.read_bytes())
        client = Client(endpoint_for(state), attachment["token"])
        child = subprocess.Popen([str(binary), "--state-dir", str(state), "service"], stdin=subprocess.DEVNULL,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=environment())
        try:
            deadline = time.monotonic() + 5
            while True:
                require(child.poll() is None, "The owned service exited before readiness.")
                try:
                    info = client.call("service.info", {})
                    break
                except (FileNotFoundError, ConnectionRefusedError):
                    require(time.monotonic() < deadline, "The owned service did not become ready.")
                    time.sleep(0.01)
            require(info.get("process_id") == child.pid, "The service endpoint is not owned by this benchmark.")
            setup_ms = (time.perf_counter() - setup_started) * 1000
            for domain in DOMAINS:
                folder = project / domain
                folder.mkdir(mode=0o700)
                manifest = (fixtures / domain / "process.json").read_bytes()
                case_bytes = (fixtures / domain / "cases.json").read_bytes()
                require(len(manifest) <= BUDGET and len(case_bytes) <= BUDGET, "A fixture exceeds the input allowance.")
                (folder / "process.json").write_bytes(manifest)
                registration_started = time.perf_counter()
                validated = cli(binary, state, "process", "validate", folder / "process.json")
                registered = client.call("manage", {"action": "process.register", "path": domain + "/process.json"})
                require(registered["process_ref"] == validated["process_version"], "CLI and service process versions disagree.")
                cases = json.loads(case_bytes)
                require(isinstance(cases, list) and len(cases) == 2, "A domain must define two bounded synthetic cases.")
                definitions.append({"domain": domain, "process_ref": registered["process_ref"],
                                    "manifest_bytes": len(manifest), "manifest_sha256": sha256(manifest),
                                    "cases_sha256": sha256(case_bytes),
                                    "validation_and_registration_ms": round((time.perf_counter() - registration_started) * 1000, 3)})
                for iteration in range(iterations):
                    for case in cases:
                        sample = run_case(client, registered["process_ref"], case, iteration)
                        sample["domain"] = domain
                        samples.append(sample)
            storage = cli(binary, state, "storage", "usage")
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                    graceful = child.returncode == 0
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
    durations = [sample["elapsed_ms"] for sample in samples]
    return {"benchmark": "apr-synthetic-domain-outcomes-v1", "timestamp_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "platform": platform.platform(), "machine": platform.machine(), "python_version": platform.python_version(),
            "binary_sha256": binary_digest.hexdigest(), "binary_bytes": binary.stat().st_size,
            "iterations_per_case": iterations, "cases_per_iteration": 6, "verified_runs": len(samples),
            "fixture_setup_ms": round(setup_ms, 3), "total_elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
            "run_elapsed_ms": {"median": round(statistics.median(durations), 3), "min": min(durations), "max": max(durations)},
            "definitions": definitions, "samples": samples, "final_storage_metadata": storage,
            "graceful_owned_service_shutdown": graceful, "model_tokens": None, "model_cost": None,
            "caveats": [
                "Synthetic fixtures and canned caller results check runtime correctness; no model, human-quality assessment, or live provider participates.",
                "First/reuse phases refer to existing saved definitions. Authoring, setup maintenance, direct-agent baselines and correction effort are not measured.",
                "Protocol bytes are exact UTF-8 frames, including auth fields and polling. They are not model tokens or inferred savings; no token values are printed.",
                "Elapsed run time includes Python calls and 10 ms polling; verification/replay traffic is reported separately.",
                "The executable is supplied by the caller; the harness does not attest its build profile or complete installed footprint.",
                "No runtime settings or client configuration are installed. Temporary state is removed; the runtime-managed permanent endpoint lock remains.",
            ]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apr", type=Path, required=True)
    parser.add_argument("--iterations", type=int, default=3)
    args = parser.parse_args()
    if not 1 <= args.iterations <= 10:
        parser.error("iterations must be between 1 and 10")
    print(json.dumps(benchmark(args.apr.resolve(strict=True), args.iterations, Path(__file__).resolve().parents[1] / "examples"), indent=2, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except BenchmarkError as error:
        print(json.dumps({"error": str(error)}), file=sys.stderr)
        sys.exit(2)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
        print(json.dumps({"error": "Synthetic outcome verification failed; confirm the supplied APR binary supports v0.2 routing and inspect the fixture contract."}), file=sys.stderr)
        sys.exit(2)
