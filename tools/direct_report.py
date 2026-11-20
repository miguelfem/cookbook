#!/usr/bin/env python3
"""Run a prepared CookBook report through Codex without starting a model turn."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import sys
import threading
import time
import tomllib


def toml_value(value):
    """Encode the configured APR entry for a process-local Codex override."""
    if isinstance(value, dict):
        return "{" + ",".join(f"{json.dumps(k)}={toml_value(v)}" for k, v in value.items()) + "}"
    if isinstance(value, list):
        return "[" + ",".join(toml_value(v) for v in value) + "]"
    if isinstance(value, (str, bool, int, float)):
        return json.dumps(value, ensure_ascii=False, allow_nan=False)
    raise ValueError("Unsupported setting in the installed APR entry")


def installed_entry():
    config = Path.home() / ".codex" / "config.toml"
    entry = tomllib.loads(config.read_text())["mcp_servers"]["apr"]
    managed_fields = {"command", "args", "enabled", "startup_timeout_sec", "tool_timeout_sec", "tools"}
    if entry.keys() - managed_fields:
        raise ValueError("The installed APR entry has settings outside the managed connection")
    command = Path(entry["command"])
    args = entry["args"]
    if not command.is_absolute() or not command.is_file() or entry.get("enabled") is False:
        raise ValueError("CookBook's managed Codex connection must be installed and enabled")
    if len(args) != 4 or args[0] != "--state-dir" or args[2:] != ["mcp", "--projects"]:
        raise ValueError("Use CookBook's managed project connection")
    if entry.get("tools", {}).get("apr_run", {}).get("approval_mode") != "approve":
        raise ValueError("Direct runs require the owner-approved apr_run policy in Codex")
    state = Path(args[1])
    if not state.is_absolute() or not state.is_dir():
        raise ValueError("The installed CookBook runtime directory is unavailable")
    return entry, command, state


class CodexSession:
    def __init__(self, process):
        self.process = process
        self.messages = queue.Queue()
        self.request_id = 0
        threading.Thread(target=self.read_messages, daemon=True).start()

    def read_messages(self):
        try:
            for line in self.process.stdout:
                self.messages.put(json.loads(line))
        finally:
            self.messages.put(None)

    def send(self, message):
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()

    def call(self, method, params):
        self.request_id += 1
        request_id = self.request_id
        self.send({"id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            try:
                message = self.messages.get(timeout=max(0, deadline - time.monotonic()))
            except queue.Empty:
                break
            if message is None:
                raise RuntimeError("The Codex App Server stopped before replying")
            if message.get("method") == "thread/tokenUsage/updated":
                raise RuntimeError("Unexpected model activity in a direct session")
            if "id" in message and "method" in message:
                self.send({"id": message["id"], "error": {
                    "code": -32000, "message": "Interactive approval is required"
                }})
                raise RuntimeError("Codex requested interactive approval")
            if message.get("id") == request_id:
                if "error" in message:
                    raise RuntimeError(f"Codex rejected {method}")
                return message["result"]
        raise TimeoutError(f"Timed out waiting for {method}")

    def tool(self, thread, name, arguments):
        result = self.call("mcpServer/tool/call", {
            "threadId": thread, "server": "apr", "tool": name, "arguments": arguments
        })
        value = result.get("structuredContent")
        if result.get("isError") or not isinstance(value, dict):
            raise RuntimeError(f"CookBook could not complete {name}")
        return value


@contextmanager
def connected_codex(project, entry, executable="codex"):
    process = subprocess.Popen(
        [executable, "app-server", "--stdio", "-c", "plugins={}", "-c", "analytics.enabled=false",
         "-c", "mcp_servers=" + toml_value({"apr": entry})],
        cwd=project, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        text=True, bufsize=1, env={k: v for k, v in os.environ.items()
                                   if k not in ("OPENAI_API_KEY", "CODEX_API_KEY")},
    )
    try:
        session = CodexSession(process)
        session.call("initialize", {
            "clientInfo": {"name": "cookbook-direct-report", "version": "1"},
            "capabilities": {"experimentalApi": True},
        })
        session.send({"method": "initialized", "params": {}})
        thread = session.call("thread/start", {
            "cwd": str(project), "ephemeral": True, "approvalPolicy": "never",
            "sandbox": "workspace-write",
        })["thread"]["id"]
        yield session, thread
    finally:
        process.terminate()
        try:
            process.wait(timeout=4)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def connect_project(session, thread, project):
    proof = session.tool(thread, "apr_project", {"action": "prepare", "root": str(project)})
    path = Path(proof["verification_file"])
    if path.parent != project or not re.fullmatch(r"\.cookbook-connect-[a-f0-9]+\.json", path.name):
        raise RuntimeError("CookBook returned a proof outside the selected project")
    with path.open("x") as proof_file:
        proof_file.write(proof["verification_contents"])
    try:
        return session.tool(thread, "apr_project", {
            "action": "connect", "challenge_id": proof["challenge_id"]
        })
    finally:
        path.unlink()


def run_report(session, thread, workspace, process_ref, inputs, request_key):
    result = session.tool(thread, "apr_run", {
        "workspace": workspace, "process_ref": process_ref, "input": inputs,
        "request_key": request_key, "mode": "start", "wait_ms": 5000, "result_view": "output",
    })
    if result.get("state") != "succeeded" or result.get("output_complete") is not True:
        raise RuntimeError(f"Report is unavailable. Inspect CookBook run {result.get('run_id', '(unknown)')}.")
    output = result["output"]
    counts, reference = output.get("summary"), output.get("report")
    if not isinstance(counts, dict) or not counts or not all(
        re.fullmatch(r"[a-z][a-z0-9_]{0,63}", name) and type(value) is int
        for name, value in counts.items()
    ):
        raise ValueError("The prepared recipe must return a summary of integer counts")
    if (not isinstance(reference, dict) or reference.get("media_type") != "application/json"
            or re.fullmatch(r"[a-f0-9]{64}", reference.get("sha256", "")) is None):
        raise ValueError("The prepared recipe must return a CookBook JSON report reference")
    authorization = session.tool(thread, "apr_inspect", {
        "workspace": workspace, "view": "artifact", "reference": reference, "detail": True
    })
    if authorization.get("artifact") != reference:
        raise RuntimeError("The report was not authorized by the active CookBook workspace")
    return counts, reference


def render_report(command, state, counts, reference, destination):
    destination = destination.resolve()
    # Codex renders absolute Markdown file links. Refuse paths whose Markdown
    # delimiters could turn a local destination into another kind of link.
    if any(char in str(destination) for char in "<>\r\n"):
        raise ValueError("Choose an output directory without Markdown delimiters")
    destination.mkdir(mode=0o700)
    ref_file = destination / "report-reference.json"
    report = destination / "full-report.json"
    ref_file.write_text(json.dumps(reference), encoding="utf-8")
    try:
        subprocess.run([command, "--state-dir", state, "artifact", "read", "--reference-file", ref_file,
                        "--output-file", report, "--max-bytes", "1048576"],
                       stdout=subprocess.DEVNULL, check=True)
    finally:
        ref_file.unlink()
    if hashlib.sha256(report.read_bytes()).hexdigest() != reference["sha256"]:
        raise RuntimeError("Exported report did not match the verified artifact")
    summary = "\n".join(f"- {name.replace('_', ' ').capitalize()} **{value:,}**"
                        for name, value in counts.items())
    markdown = f"{summary}\n\n[Open full report](<{report}>)\n"
    (destination / "summary.md").write_text(markdown, encoding="utf-8")
    return markdown


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project", type=Path, required=True)
    parser.add_argument("--process-ref", required=True)
    parser.add_argument("--input-file", type=Path, required=True)
    parser.add_argument("--request-key", required=True)
    parser.add_argument("--output", type=Path, required=True, help="New private output directory")
    args = parser.parse_args()
    project = args.project.resolve(strict=True)
    if re.fullmatch(r"sha256:[a-f0-9]{64}", args.process_ref) is None:
        parser.error("--process-ref must be an immutable CookBook recipe version")
    destination = args.output.resolve()
    if destination.exists() or not destination.parent.is_dir():
        parser.error("--output must name a new directory under an existing parent")
    with args.input_file.open("rb") as inputs:
        raw = inputs.read(65537)
    if len(raw) > 65536:
        parser.error("--input-file exceeds 64 KiB")
    entry, command, state = installed_entry()
    with connected_codex(project, entry) as (session, thread):
        workspace = connect_project(session, thread, project)["workspace"]
        counts, reference = run_report(session, thread, workspace, args.process_ref,
                                       json.loads(raw), args.request_key)
    sys.stdout.write(render_report(command, state, counts, reference, destination))


if __name__ == "__main__":
    main()
