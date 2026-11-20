#!/usr/bin/env python3
"""Benchmark native scheduling and a loopback-only durable relay with synthetic input."""

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hmac
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
from pathlib import Path
import re
import secrets
import shutil
import sqlite3
import subprocess
import sys
import threading
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from benchmark_codex_head_to_head import ROOT, canonical, fixture, installed_entry
from measure_batch_workers import Mcp, cli, ready, save


def wall_ms():
    return time.time_ns() // 1_000_000


@contextmanager
def supervisor(binary, state, log):
    process = subprocess.Popen(
        [binary, "--state-dir", state, "service"], stdout=log, stderr=log
    )
    try:
        ready(binary, state, process)
        yield process
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def scoped_report(binary, state, reference, expected, destination):
    ref_file = destination.with_suffix(".ref.json")
    save(ref_file, reference)
    cli(
        binary,
        state,
        "artifact",
        "read",
        "--reference-file",
        ref_file,
        "--output-file",
        destination,
    )
    assert destination.read_bytes() == canonical({"rows": expected})


def scheduled(binary, state, attachment, version, case, csv, brief, expected, output):
    baseline = []
    for index in range(4):
        for runner in (
            ("script", "cookbook") if index % 2 == 0 else ("cookbook", "script")
        ):
            due = wall_ms() + 650
            if runner == "script":
                while wall_ms() < due:
                    time.sleep(0.005)
                started = wall_ms()
                target = output / f"script-{index}.json"
                observed = json.loads(
                    subprocess.check_output([sys.executable, case, csv, target])
                )
                done = wall_ms()
                assert observed["summary"] == brief
                assert target.read_bytes() == canonical({"rows": expected})
            else:
                at = datetime.fromtimestamp(due / 1000, timezone.utc).isoformat(
                    timespec="milliseconds"
                )
                value = cli(
                    binary,
                    state,
                    "schedule",
                    "create",
                    "--attachment",
                    attachment,
                    "--name",
                    f"Fixture SLA {index}",
                    "--process-ref",
                    version,
                    "--input-file",
                    output / "sla-input.json",
                    "--request-key",
                    f"scheduled-sla-{index}",
                    "--at",
                    at,
                )
                schedule = value["schedule_id"]
                deadline = time.monotonic() + 8
                record = None
                with sqlite3.connect(
                    f"file:{state/'state.sqlite3'}?mode=ro", uri=True
                ) as db:
                    while time.monotonic() < deadline:
                        record = db.execute(
                            """SELECT r.output,r.created_at,e.created_at FROM runs r
                                JOIN events e ON e.run_id=r.id AND e.kind='run.succeeded'
                                WHERE r.schedule_id=? AND r.state='succeeded'""",
                            (schedule,),
                        ).fetchone()
                        if record:
                            break
                        time.sleep(0.025)
                assert record, "Scheduled recipe did not finish"
                raw, started, done = record
                observed = json.loads(raw)
                assert observed["summary"] == brief
                scoped_report(
                    binary,
                    state,
                    observed["report"],
                    expected,
                    output / f"scheduled-{index}.json",
                )
            baseline.append(
                {
                    "runner": runner,
                    "trial": index,
                    "rows": 300,
                    "due_to_dispatch_ms": started - due,
                    "due_to_report_ms": done - due,
                    "dispatch_to_report_ms": done - started,
                    "model_tokens": 0,
                    "exact": True,
                }
            )
    return baseline


class Queue:
    """Small fixture queue. SQLite transactions own dedupe, claims and acknowledgements."""

    def __init__(self, path):
        self.path = path
        with sqlite3.connect(path) as db:
            db.execute("""CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY, fixture TEXT NOT NULL, status TEXT NOT NULL,
                lease TEXT, expires INTEGER, receipt TEXT
            )""")

    def transact(self, action, data):
        with sqlite3.connect(self.path, timeout=3) as db:
            db.row_factory = sqlite3.Row
            db.execute("BEGIN IMMEDIATE")
            if action == "submit":
                identity, fixture_name = data.get("request_key"), data.get("fixture")
                if (
                    not isinstance(identity, str)
                    or not re.fullmatch(r"[a-z0-9-]{1,64}", identity)
                    or fixture_name not in ("sla-300", "sla-144")
                    or len(data) != 2
                ):
                    return 400, {"error": "invalid fixture request"}
                old = db.execute(
                    "SELECT fixture,status FROM jobs WHERE id=?", (identity,)
                ).fetchone()
                if old and old["fixture"] != fixture_name:
                    return 409, {"error": "request key conflict"}
                if not old:
                    db.execute(
                        "INSERT INTO jobs(id,fixture,status) VALUES(?,?,'queued')",
                        (identity, fixture_name),
                    )
                return 202, {
                    "job_id": identity,
                    "status": old["status"] if old else "queued",
                }
            if action == "claim":
                row = db.execute(
                    """SELECT id,fixture FROM jobs WHERE status='queued' OR
                                  (status='leased' AND expires<=?) ORDER BY rowid LIMIT 1""",
                    (wall_ms(),),
                ).fetchone()
                if row is None:
                    return 200, {"job": None}
                lease = secrets.token_hex(16)
                db.execute(
                    "UPDATE jobs SET status='leased',lease=?,expires=? WHERE id=?",
                    (lease, wall_ms() + 1000, row["id"]),
                )
                return 200, {"job": {**dict(row), "lease": lease}}
            row = db.execute(
                "SELECT * FROM jobs WHERE id=?", (data["job_id"],)
            ).fetchone()
            if row is None:
                return 404, {"error": "unknown job"}
            if action == "status":
                return 200, {
                    "job_id": row["id"],
                    "status": row["status"],
                    "receipt": json.loads(row["receipt"]) if row["receipt"] else None,
                }
            if (
                row["status"] != "leased"
                or row["lease"] != data["lease"]
                or row["expires"] <= wall_ms()
            ):
                return 409, {"error": "stale lease"}
            receipt = data["receipt"]
            if (
                not isinstance(receipt, dict)
                or set(receipt) != {"run_id", "counts"}
                or not isinstance(receipt["run_id"], str)
                or not re.fullmatch(r"run_[a-f0-9]{32}", receipt["run_id"])
                or not isinstance(receipt["counts"], dict)
                or set(receipt["counts"]) != {"checked", "breached", "unanswered"}
                or not all(
                    type(i) is int and 0 <= i <= 300 for i in receipt["counts"].values()
                )
            ):
                return 400, {"error": "invalid summary receipt"}
            db.execute(
                "UPDATE jobs SET status='completed',receipt=?,lease=NULL,expires=NULL WHERE id=?",
                (canonical(receipt).decode(), row["id"]),
            )
            return 200, {"status": "completed"}


@contextmanager
def local_relay(queue, producer, worker):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def done(self, status, data):
            body = canonical(data)
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def authorized(self, token):
            return hmac.compare_digest(
                self.headers.get("Authorization", ""), "Bearer " + token
            )

        def do_GET(self):
            key = self.path.removeprefix("/jobs/")
            if not self.path.startswith("/jobs/") or not re.fullmatch(
                r"[a-z0-9-]{1,64}", key
            ):
                return self.done(404, {"error": "unknown route"})
            if not self.authorized(producer):
                return self.done(403, {"error": "forbidden"})
            self.done(*queue.transact("status", {"job_id": key}))

        def do_POST(self):
            action = {
                "/jobs": "submit",
                "/claims": "claim",
                "/receipts": "complete",
            }.get(self.path)
            if action is None:
                return self.done(404, {"error": "unknown route"})
            if not self.authorized(producer if action == "submit" else worker):
                return self.done(403, {"error": "forbidden"})
            try:
                size = int(self.headers.get("Content-Length", "-1"))
                if not 0 <= size <= 4096:
                    return self.done(413, {"error": "input too large"})
                data = json.loads(self.rfile.read(size))
                if not isinstance(data, dict):
                    raise ValueError("object required")
                self.done(*queue.transact(action, data))
            except (ValueError, TypeError, KeyError):
                self.done(400, {"error": "invalid request"})

    with HTTPServer(("127.0.0.1", 0), Handler) as server:
        thread = threading.Thread(
            target=server.serve_forever, kwargs={"poll_interval": 0.025}
        )
        thread.start()
        try:
            yield f"http://127.0.0.1:{server.server_port}"
        finally:
            server.shutdown()
            thread.join()


def request(url, route, key, data=None, expected=200):
    req = Request(
        url + route,
        data=canonical(data) if data is not None else None,
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
    )
    try:
        with urlopen(req, timeout=3) as response:
            assert response.status == expected
            return json.load(response)
    except HTTPError as error:
        assert error.code == expected, (error.code, expected)
        return json.load(error)


def relay_probe(
    binary, state, attachment, version, inputs, counts, expected, output, log
):
    queue = Queue(output / "relay.sqlite3")
    producer, worker = secrets.token_hex(24), secrets.token_hex(24)
    payload = {"request_key": "synthetic-outage-sla", "fixture": "sla-300"}
    with local_relay(queue, producer, worker) as relay:
        start = time.monotonic()
        accepted = request(relay, "/jobs", producer, payload, 202)
        acceptance_ms = (time.monotonic() - start) * 1000
        assert accepted["status"] == "queued"
        assert request(relay, "/jobs", producer, payload, 202) == accepted
        request(relay, "/jobs", producer, {**payload, "fixture": "sla-144"}, 409)
        request(relay, "/claims", producer, {}, 403)
    with local_relay(queue, producer, worker) as relay:
        assert (
            request(relay, "/jobs/" + payload["request_key"], producer)["status"]
            == "queued"
        )
        # The worker stays offline for a deliberate interval after the relay restart.
        time.sleep(0.65)
        with supervisor(binary, state, log):
            worker_ready = time.monotonic()
            bridge = Mcp(binary, state, attachment, log)
            try:
                job = request(relay, "/claims", worker, {})["job"]
                assert job["fixture"] == "sla-300"
                args = {
                    "process_ref": version,
                    "input": inputs,
                    "request_key": "relay-" + job["id"],
                    "mode": "start",
                    "result_view": "output",
                    "wait_ms": 5000,
                }
                first = bridge.tool("apr_run", args)
                first_done = time.monotonic()
                assert first["output"]["summary"] == counts
                scoped_report(
                    binary,
                    state,
                    first["output"]["report"],
                    expected,
                    output / "relay-report.json",
                )
                receipt = {
                    "run_id": first["run_id"],
                    "counts": first["output"]["summary"],
                }
                # Simulate a lost acknowledgement after durable local completion.
                time.sleep(1.05)
                new = request(relay, "/claims", worker, {})["job"]
                assert new["id"] == job["id"] and new["lease"] != job["lease"]
                request(
                    relay,
                    "/receipts",
                    worker,
                    {"job_id": job["id"], "lease": job["lease"], "receipt": receipt},
                    409,
                )
                replay = bridge.tool("apr_run", args)
                assert (
                    replay["run_id"] == first["run_id"]
                    and replay["output"] == first["output"]
                )
                request(
                    relay,
                    "/receipts",
                    worker,
                    {"job_id": job["id"], "lease": new["lease"], "receipt": receipt},
                )
                final = request(relay, "/jobs/" + job["id"], producer)
                assert final["status"] == "completed" and final["receipt"] == receipt
                with sqlite3.connect(
                    f"file:{state/'state.sqlite3'}?mode=ro", uri=True
                ) as db:
                    assert db.execute(
                        "SELECT count(*) FROM effect_intents WHERE run_id=?",
                        (first["run_id"],),
                    ).fetchone() == (1,)
            finally:
                bridge.close()
    return {
        "accepted_while_apr_offline": True,
        "accepted_ms": round(acceptance_ms, 3),
        "accepted_to_worker_ready_ms": round((worker_ready - start) * 1000, 1),
        "worker_ready_to_first_result_ms": round((first_done - worker_ready) * 1000, 1),
        "durable_after_relay_restart": True,
        "same_run_after_lost_ack": True,
        "native_effects": 1,
        "model_tokens": 0,
    }


def run(output):
    output.mkdir(mode=0o700)
    project, state = output / "project", output / "runtime"
    project.mkdir()
    source = ROOT / "examples/operation-reports"
    for name in ("sla.py", "sla-brief.json"):
        shutil.copyfile(source / name, project / name)
    csv, summary, expected, _ = fixture("sla", 300, project)
    _, binary, _ = installed_entry()
    project_id = cli(binary, state, "project", "attach", project)["project_id"]
    assignment = cli(
        binary,
        state,
        "assignment",
        "create",
        "--project",
        project_id,
        "--name",
        "Synthetic relay fixture",
    )["assignment_id"]
    attachment = state / "attachments/author.json"
    cli(
        binary,
        state,
        "attachment",
        "create",
        "--assignment",
        assignment,
        "--mode",
        "author",
        "--output-file",
        attachment,
    )
    cli(
        binary,
        state,
        "execution",
        "grant-python",
        "--assignment",
        assignment,
        "--interpreter",
        Path(sys.executable).resolve(),
    )
    with (output / "service.log").open("x") as log:
        with supervisor(binary, state, log):
            bridge = Mcp(binary, state, attachment, log)
            try:
                version = bridge.tool(
                    "apr_manage",
                    {"action": "process.register", "path": "sla-brief.json"},
                )["process_ref"]
                reference = bridge.tool(
                    "apr_manage",
                    {
                        "action": "artifact.import",
                        "path": csv.name,
                        "media_type": "text/csv",
                    },
                )
            finally:
                bridge.close()
            inputs = {"source": reference}
            save(output / "sla-input.json", inputs)
            local = scheduled(
                binary,
                state,
                attachment,
                version,
                project / "sla.py",
                csv,
                summary,
                expected,
                output,
            )
        relay = relay_probe(
            binary, state, attachment, version, inputs, summary, expected, output, log
        )
    evidence = {"local_schedule": local, "relay": relay}
    save(output / "metrics.json", evidence)
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    run(parser.parse_args().output.resolve())
