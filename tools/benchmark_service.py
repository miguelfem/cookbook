#!/usr/bin/env python3
"""Sample a supplied APR binary's local service; no model or token benchmark."""

import argparse
import datetime
import hashlib
import json
import math
import os
import platform
import socket
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


RESPONSE_BUDGET = 8 * 1024


class BenchmarkError(Exception):
    """A fixed, credential-free diagnostic suitable for the command line."""


def cli(binary, state, *arguments):
    environment = os.environ.copy()
    environment.pop("APR_TEST_CRASH_AT", None)
    environment.pop("APR_TEST_WORKER_GATE", None)
    completed = subprocess.run(
        [str(binary), "--state-dir", str(state), *map(str, arguments)],
        stdin=subprocess.DEVNULL, capture_output=True, timeout=10, env=environment,
    )
    if completed.returncode:
        # Never forward command output: attachment capabilities are confidential.
        raise BenchmarkError("APR fixture setup failed; verify the supplied binary supports M1.")
    try:
        return json.loads(completed.stdout)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise BenchmarkError("APR fixture setup returned invalid JSON.") from error


def endpoint_for(state):
    # Same Unix endpoint derivation as apr_runtime::service::socket_path.
    digest = hashlib.sha256(os.fsencode(state.resolve(strict=True))).hexdigest()[:32]
    return Path("/tmp") / f"apr-{os.geteuid()}" / digest / "rpc.sock"


def rpc(endpoint, token, action, arguments, request_id):
    request = {
        "protocol_version": "0.1", "request_id": request_id,
        "attachment_token": token, "action": action, "args": arguments,
        "response_budget": RESPONSE_BUDGET,
    }
    frame = json.dumps(request, separators=(",", ":")).encode() + b"\n"
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        # A single absolute deadline bounds fragmented responses as well.
        deadline = time.monotonic() + 5
        connection.settimeout(5)
        connection.connect(str(endpoint))
        connection.sendall(frame)
        received = bytearray()
        while b"\n" not in received:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise BenchmarkError("Local service response timed out.")
            connection.settimeout(remaining)
            chunk = connection.recv(min(4096, RESPONSE_BUDGET + 1 - len(received)))
            if not chunk:
                raise BenchmarkError("Local service closed an incomplete response.")
            received.extend(chunk)
            if len(received) > RESPONSE_BUDGET:
                raise BenchmarkError("Local service exceeded the response budget.")
    try:
        response = json.loads(received)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise BenchmarkError("Local service returned invalid JSON.") from error
    if (
        not isinstance(response, dict)
        or response.get("protocol_version") != "0.1"
        or response.get("request_id") != request_id
        or "result" not in response
        or "error" in response
    ):
        raise BenchmarkError("Local service rejected or misidentified a benchmark request.")
    return response["result"]


def summary(values):
    ordered = sorted(values)
    return {
        "count": len(values),
        "median": round(statistics.median(values), 3),
        "p95_nearest_rank": round(ordered[math.ceil(len(ordered) * 0.95) - 1], 3),
        "min": round(min(values), 3),
        "max": round(max(values), 3),
        "samples": [round(value, 3) for value in values],
    }


def idle_rss(pid):
    measured = subprocess.run(
        ["ps", "-p", str(pid), "-o", "rss="],
        stdin=subprocess.DEVNULL, capture_output=True, timeout=5,
    )
    if measured.returncode:
        raise BenchmarkError("Could not sample the owned service process RSS.")
    try:
        # macOS and Linux ps expose RSS in KiB, unlike resource.ru_maxrss.
        return int(measured.stdout.strip()) * 1024
    except ValueError as error:
        raise BenchmarkError("The local ps command returned an invalid RSS sample.") from error


def benchmark(binary, iterations, idle_seconds):
    digest = hashlib.sha256()
    with binary.open("rb") as executable:
        for block in iter(lambda: executable.read(1024 * 1024), b""):
            digest.update(block)
    binary_digest = digest.hexdigest()
    environment = os.environ.copy()
    environment.pop("APR_TEST_CRASH_AT", None)
    environment.pop("APR_TEST_WORKER_GATE", None)
    graceful_shutdown = False
    with tempfile.TemporaryDirectory(prefix="apr-service-benchmark-") as temporary:
        root = Path(temporary)
        state, project = root / "state", root / "project"
        project.mkdir(mode=0o700)
        attached = cli(binary, state, "project", "attach", project)
        assignment = cli(
            binary, state, "assignment", "create", "--project", attached["project_id"],
            "--name", "Local resource sample",
        )
        attachment_path = state / "attachments" / "benchmark.json"
        cli(
            binary, state, "attachment", "create", "--assignment", assignment["assignment_id"],
            "--mode", "author", "--output-file", attachment_path,
        )
        with attachment_path.open() as attachment_file:
            attachment = json.load(attachment_file)
        endpoint = endpoint_for(state)
        started = time.perf_counter()
        child = subprocess.Popen(
            [str(binary), "--state-dir", str(state), "service"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            env=environment,
        )
        try:
            ready_deadline = time.monotonic() + 5
            while True:
                if child.poll() is not None:
                    raise BenchmarkError("The owned local service exited before readiness.")
                try:
                    info = rpc(endpoint, attachment["token"], "service.info", {}, "ready")
                    break
                except (FileNotFoundError, ConnectionRefusedError):
                    if time.monotonic() >= ready_deadline:
                        raise BenchmarkError("The owned local service did not become ready.")
                    time.sleep(0.01)
            readiness_ms = (time.perf_counter() - started) * 1000
            if info.get("process_id") != child.pid:
                raise BenchmarkError("The endpoint is not the benchmark's owned service process.")
            for index in range(3):
                catalog = rpc(endpoint, attachment["token"], "discover", {}, f"warmup-{index}")
                if catalog.get("processes") != []:
                    raise BenchmarkError("The disposable catalog is unexpectedly nonempty.")
            time.sleep(idle_seconds)
            rss_samples = []
            for index in range(3):
                if child.poll() is not None:
                    raise BenchmarkError("The owned service exited during RSS sampling.")
                rss_samples.append(idle_rss(child.pid))
                if index < 2:
                    time.sleep(0.1)
            latencies = {"service_info_ms": [], "discover_empty_catalog_ms": []}
            for iteration in range(iterations):
                for name, action in [("service_info_ms", "service.info"), ("discover_empty_catalog_ms", "discover")]:
                    before = time.perf_counter()
                    rpc(endpoint, attachment["token"], action, {}, f"{action}-{iteration}")
                    latencies[name].append((time.perf_counter() - before) * 1000)
        finally:
            # Only terminate/reap the child this invocation created, never search
            # for or signal an unrelated APR process by executable name or PID.
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                    graceful_shutdown = child.returncode == 0
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)

    return {
        "benchmark": "apr-local-service-v1",
        "timestamp_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "python_version": platform.python_version(),
        "binary_bytes": binary.stat().st_size,
        "binary_sha256": binary_digest,
        "iterations_per_action": iterations,
        "service_readiness_ms": round(readiness_ms, 3),
        "idle_wait_seconds": idle_seconds,
        "idle_service_rss_bytes": summary(rss_samples),
        "rss_source": "ps -p <owned service pid> -o rss=; KiB multiplied by 1024",
        "latency_ms": {name: summary(values) for name, values in latencies.items()},
        "graceful_owned_service_shutdown": graceful_shutdown,
        "caveats": [
            "This samples the supplied binary; the script does not prove it was built in release mode.",
            "Readiness is a foreground service start with precreated SQLite state and warm OS caches, not cold MCP or app startup.",
            "RSS is three current idle resident-set samples after three discovery warmups, not peak RSS or complete installed footprint.",
            "Latency includes Python JSON, a fresh Unix socket, attachment authentication, service dispatch, and response parsing; calls are sequential against an empty catalog.",
            "Python, ps, client apps, active workers, provider usage, quality, and token savings are not measured.",
            "Temporary project, SQLite state, and attachment credentials are removed; the runtime-managed permanent endpoint lock is retained.",
            "Small local samples depend on host load and OS caches and do not establish performance across devices or real workflows.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apr", type=Path, required=True, help="path to a built APR release binary")
    parser.add_argument("--iterations", type=int, default=20)
    parser.add_argument("--idle-seconds", type=float, default=1.0)
    arguments = parser.parse_args()
    if platform.system() not in {"Darwin", "Linux"}:
        parser.error("this sample supports macOS and Linux")
    if not 1 <= arguments.iterations <= 100:
        parser.error("iterations must be between 1 and 100")
    if not math.isfinite(arguments.idle_seconds) or not 0 <= arguments.idle_seconds <= 30:
        parser.error("idle-seconds must be between 0 and 30")
    try:
        binary = arguments.apr.resolve(strict=True)
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error("apr must name an executable regular file")
        result = benchmark(binary, arguments.iterations, arguments.idle_seconds)
    except BenchmarkError as error:
        parser.exit(1, f"benchmark_service: {error}\n")
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError):
        # Keep unexpected exception payloads and credential-bearing JSON private.
        parser.exit(1, "benchmark_service: local setup, transport, or resource sampling failed.\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
