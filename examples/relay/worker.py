"""Poll one relay and start allowlisted recipes through a local run-only MCP attachment."""

import argparse
import json
import os
from pathlib import Path
from queue import Empty, Queue
import re
import subprocess
from threading import Thread
import time
from urllib.error import URLError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

MCP_TIMEOUT = 25


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def post(relay, token, endpoint, payload):
    request = Request(
        relay + endpoint,
        data=json.dumps(payload).encode(),
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
        method="POST",
    )
    with build_opener(NoRedirect()).open(request, timeout=10) as response:
        return json.loads(response.read(131072))


def collect_output(stream, responses):
    for line in stream:
        responses.put(line)
    responses.put("")


def start_recipe(config, job):
    pin = config["recipes"].get(job["recipe"])
    if pin is None or not re.fullmatch(r"sha256:[0-9a-f]{64}", pin):
        raise ValueError("Relay job has no locally approved recipe version")
    if not re.fullmatch(r"[0-9a-f]{32}", job["id"]):
        raise ValueError("Invalid relay job ID")
    command = [
        config.get("apr", "apr"),
        "--state-dir", str(Path(config["state_dir"]).expanduser()),
        "mcp", "--attachment-file", str(Path(config["attachment"]).expanduser()),
    ]
    env = {key: val for key, val in os.environ.items() if not key.startswith("APR_RELAY_")}
    responses = Queue()
    process = subprocess.Popen(
        command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, text=True, encoding="utf-8", env=env,
    )
    reader = Thread(target=collect_output, args=(process.stdout, responses), daemon=True)
    reader.start()
    deadline = time.monotonic() + MCP_TIMEOUT

    def send(method, params=None, number=None):
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        if number is not None:
            message["id"] = number
        process.stdin.write(json.dumps(message) + "\n")
        process.stdin.flush()
        if number is None:
            return None
        while time.monotonic() < deadline:
            line = responses.get(timeout=max(0, deadline - time.monotonic()))
            if not line:
                raise RuntimeError("CookBook MCP disconnected")
            reply = json.loads(line)
            if reply.get("id") == number:
                if "error" in reply:
                    raise RuntimeError("CookBook MCP rejected the request")
                return reply["result"]
        raise TimeoutError("CookBook MCP timed out")

    try:
        send("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "cookbook-relay-worker", "version": "1"},
        }, 1)
        send("notifications/initialized")
        result = send("tools/call", {
            "name": "apr_run",
            "arguments": {
                "process_ref": pin, "input": job["input"],
                "request_key": f"relay-{job['id']}", "mode": "start", "wait_ms": 0,
            },
        }, 2)
        run_id = (result.get("structuredContent") or {}).get("run_id")
        if not isinstance(run_id, str) or not re.fullmatch(r"run_[0-9a-f]{32}", run_id):
            raise RuntimeError("CookBook returned no accepted run ID; inspect it locally")
        return run_id
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        reader.join(timeout=2)
        process.stdin.close()
        process.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    args = parser.parse_args()
    config = json.loads(args.config.read_text(encoding="utf-8"))
    relay = config["relay_url"].rstrip("/")
    url = urlsplit(relay)
    if (
        url.scheme != "https" or not url.hostname or url.username
        or url.password or url.path or url.query or url.fragment
    ):
        parser.error("relay_url must be an HTTPS origin")
    token = os.environ["APR_RELAY_WORKER_TOKEN"]
    while True:
        try:
            job = post(relay, token, "/claims", {})["job"]
            if job is not None:
                run_id = start_recipe(config, job)
                post(relay, token, "/receipts", {
                    "job_id": job["id"], "lease": job["lease"], "run_id": run_id,
                })
                print(f"Submitted {job['id']} as {run_id}", flush=True)
                continue
        except (URLError, OSError, ValueError, RuntimeError, TimeoutError, Empty) as error:
            print(f"Relay attempt deferred ({type(error).__name__}).", flush=True)
        time.sleep(5)


if __name__ == "__main__":
    main()
