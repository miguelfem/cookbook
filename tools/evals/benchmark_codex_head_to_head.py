#!/usr/bin/env python3
"""Matched warm Codex turns with a prepared script or installed CookBook."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shlex
import shutil
import subprocess
import sys
import threading
import time
import tomllib

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "examples" / "operation-reports"))

from datasets import canonical, fixture
from direct_report import connect_project, installed_entry, toml_value
from measure_batch_workers import cli, digest, ready, save

TASKS = ("profiles", "inventory", "sla")
SIZES = (72, 96, 144, 300)
ARMS = ("codex", "cookbook")


class Client:
    def __init__(self, process):
        self.process = process
        self.incoming = queue.Queue()
        self.number = 0
        self.usage = {}
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        try:
            for line in self.process.stdout:
                self.incoming.put((time.monotonic(), json.loads(line)))
        finally:
            self.incoming.put((time.monotonic(), None))

    def send(self, message):
        self.process.stdin.write(canonical(message).decode() + "\n")
        self.process.stdin.flush()

    def request(self, method, params):
        self.number += 1
        self.send({"id": self.number, "method": method, "params": params})
        return self.number

    def next(self, deadline):
        moment, message = self.incoming.get(timeout=max(0.01, deadline - time.monotonic()))
        if message is None:
            raise RuntimeError("Codex App Server ended before replying")
        if message.get("method") == "thread/tokenUsage/updated":
            update = message["params"]
            self.usage[update["threadId"]] = update["tokenUsage"]["total"]
        if "method" in message and "id" in message:
            self.send({"id": message["id"], "error": {
                "code": -32000, "message": "Benchmark does not grant interactive approval"
            }})
        return moment, message

    def call(self, method, params):
        request = self.request(method, params)
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            _, message = self.next(deadline)
            if message.get("id") == request and "method" not in message:
                if "error" in message:
                    raise RuntimeError(f"{method}: {message['error'].get('message')}")
                return message["result"]
        raise TimeoutError(method)

    def thread(self, project, model):
        return self.call("thread/start", {"cwd": str(project), "ephemeral": True, "model": model,
            "approvalPolicy": "never", "sandbox": "workspace-write"})["thread"]["id"]

    def tool(self, thread, name, arguments):
        result = self.call("mcpServer/tool/call", {
            "threadId": thread, "server": "apr", "tool": name, "arguments": arguments
        })
        if result.get("isError"):
            raise RuntimeError(f"{name} refused: {result.get('structuredContent')}")
        return result["structuredContent"]

    def turn(self, thread, prompt):
        previous = self.usage.get(thread, {})
        started = time.monotonic()
        deadline = started + 90
        request = self.request("turn/start", {
            "threadId": thread, "effort": "low", "input": [{"type": "text", "text": prompt}]
        })
        turn_id = None
        calls, final, timestamps = [], "", {}
        while time.monotonic() < deadline:
            now, message = self.next(deadline)
            method = message.get("method")
            if not method and message.get("id") == request:
                if "error" in message:
                    raise RuntimeError("Codex rejected the benchmark turn")
                turn_id = message["result"]["turn"]["id"]
            params = message.get("params", {})
            if params.get("threadId") != thread:
                continue
            item = params.get("item", {})
            if method == "item/started":
                timestamps[item["id"]] = now
            elif method == "item/completed":
                if item["type"] in ("mcpToolCall", "commandExecution", "fileChange", "webSearch"):
                    calls.append((item, now - timestamps.get(item["id"], now)))
                if item["type"] == "agentMessage":
                    final = item["text"]
            elif method == "turn/completed" and params["turn"]["id"] == turn_id:
                usage = {key: value - previous.get(key, 0)
                         for key, value in self.usage.get(thread, {}).items()}
                return final, calls, {
                    "seconds": round(now - started, 3), "status": params["turn"]["status"],
                    "usage": usage or None,
                    "tool_calls": [{"type": x["type"], "tool": x.get("tool"),
                                    "seconds": round(elapsed, 3), "status": x.get("status")}
                                   for x, elapsed in calls],
                }
        raise TimeoutError("Benchmark turn timed out")


@contextmanager
def app_server(executable, project, entries, log, client_type=Client):
    config = tomllib.loads((Path.home() / ".codex" / "config.toml").read_text())
    options = ["--disable", "apps", "--disable", "plugins", "-c", "analytics.enabled=false",
               "-c", "mcp_servers=" + toml_value(entries)]
    for name in config.get("mcp_servers", {}).keys() - entries.keys():
        if not re.fullmatch(r"[a-zA-Z0-9_-]+", name):
            raise ValueError("Cannot safely disable an inherited MCP server name")
        options.extend(("-c", f"mcp_servers.{name}.enabled=false"))
    process = subprocess.Popen(
        [executable, "app-server", "--stdio", *options], cwd=project,
        env={k: v for k, v in os.environ.items() if k not in ("OPENAI_API_KEY", "CODEX_API_KEY")},
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, bufsize=1,
    )
    try:
        client = client_type(process)
        client.call("initialize", {"clientInfo": {"name": "cookbook-head-to-head", "version": "1"},
                    "capabilities": {"experimentalApi": True}})
        client.send({"method": "initialized", "params": {}})
        yield client
    finally:
        process.terminate()
        try:
            process.wait(timeout=4)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def profiles_fixture(index):
    size = (4, 8, 12, 16)[index]
    order = (3, 1, 8, 5, 2, 4, 7, 6, 12, 10, 16, 14, 9, 13, 11, 15)[:size]
    ids = order[index:] + order[:index]
    lookup = {
        1: ("Fable Studio", "Engineer"), 2: ("Harbor Works", "Researcher"),
        3: ("Cedar Lab", "Writer"), 4: ("Fable Studio", "Designer"),
        5: ("Harbor Works", "Engineer"), 6: ("Cedar Lab", "Researcher"),
        7: ("Fable Studio", "Writer"), 8: ("Harbor Works", "Designer"),
        9: ("Cedar Lab", "Engineer"), 10: ("Fable Studio", "Researcher"),
        11: ("Harbor Works", "Writer"), 12: ("Cedar Lab", "Designer"),
        13: ("Fable Studio", "Engineer"), 14: ("Harbor Works", "Researcher"),
        15: ("Cedar Lab", "Writer"), 16: ("Fable Studio", "Designer"),
    }
    emails = [f"person{number:02d}@example.test" for number in ids]
    return emails, {"profiles": [
        {"key": email, "output": {"name": f"Example Person {number:02d}",
                                  "organization": lookup[number][0], "role": lookup[number][1]}}
        for email, number in zip(emails, ids, strict=True)
    ]}


def parse_reply(text):
    raw = text.strip()
    if raw.startswith("```"):
        raw = "\n".join(raw.splitlines()[1:-1])
    try:
        return json.loads(raw)
    except ValueError:
        return None


def create_projects(root):
    projects = {}
    for arm in ARMS:
        project = root / arm
        project.mkdir()
        projects[arm] = project
        for filename in ("profiles.py", "process.json"):
            shutil.copyfile(ROOT / "examples" / "profile-batch" / filename, project / filename)
        for task in ("inventory", "sla"):
            for filename in (f"{task}.py", f"{task}-brief.json"):
                shutil.copyfile(ROOT / "examples" / "operation-reports" / filename, project / filename)
            for size in SIZES:
                fixture(task, size, project)
    return projects


def check_artifact(binary, state, client, thread, workspace, reference, expected, output):
    authorized = client.tool(thread, "apr_inspect", {
        "workspace": workspace, "view": "artifact", "reference": reference, "detail": True
    })
    if authorized.get("artifact") != reference:
        return False
    ref = output.with_suffix(".reference.json")
    save(ref, reference)
    cli(binary, state, "artifact", "read", "--reference-file", ref, "--output-file", output)
    return output.read_bytes() == canonical({"rows": expected})


def shell_json(calls):
    for item, _ in reversed(calls):
        if item["type"] != "commandExecution" or item.get("status") != "completed":
            continue
        for line in reversed((item.get("aggregatedOutput") or "").splitlines()):
            value = parse_reply(line)
            if isinstance(value, dict):
                return value
    return None


def execute_row(arm, task, index, client, thread, workspace, ref, inputs,
                expected, report_rows, script_path, binary, state, inspection_path, audit=None, request_prefix="head-to-head"):
    if task == "profiles":
        common = ("Look up the fictional addresses in the requested order. Return the complete "
                  "ordered profiles as plain JSON. Use only observed results and no network. ")
    else:
        common = (f"Process the synthetic {'inventory reconciliation' if task=='inventory' else 'ticket SLA'} "
                  "batch from the indicated input. Preserve its full report and return only the three "
                  "summary counts as a plain JSON object. Use only observed results and no network. ")
    if arm == "cookbook":
        args = {"workspace": workspace, "process_ref": ref, "input": inputs, "mode": "start",
                "request_key": f"{request_prefix}-{task}-{index}", "wait_ms": 5000, "result_view": "output"}
        prompt = common + "Use the prepared CookBook recipe with apr_run. Arguments: " + canonical(args).decode()
    else:
        prompt = common + "The equivalent prepared local command is " + shlex.join(script_path) + "."
    final, calls, row = client.turn(thread, prompt)
    output_exact = False
    boundary_error = None
    try:
        if arm == "cookbook":
            execution = [item for item, _ in calls if item.get("tool") == "apr_run"]
            if len(execution) == 1:
                result = execution[0].get("result") or {}
                envelope = result.get("structuredContent") or result.get("structured_content") or {}
                if envelope.get("output_complete") is True:
                    value = envelope["output"]
                    audit_client, audit_thread, audit_workspace = audit or (client, thread, workspace)
                    output_exact = value == expected if task == "profiles" else (
                        value.get("summary") == expected and check_artifact(
                            binary, state, audit_client, audit_thread, audit_workspace,
                            value["report"], report_rows, inspection_path
                        )
                    )
        elif task == "profiles":
            output_exact = shell_json(calls) == expected
        else:
            path = inspection_path.parent / script_path[-1]
            output_exact = path.is_file() and path.read_bytes() == canonical({"rows": report_rows})
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        boundary_error = type(error).__name__
    row.update(arm=arm, task=task, round=index, tool_output_exact=output_exact,
               chat_reply_exact=parse_reply(final) == expected, grading_error=boundary_error)
    return row


def run(args):
    args.output.mkdir(mode=0o700)
    projects = create_projects(args.output)
    entry, binary, _ = installed_entry()
    state = args.output / "runtime"
    state.mkdir(mode=0o700)
    cfg = {**entry, "args": ["--state-dir", str(state), "mcp", "--projects"]}
    lock_config = Path.home() / ".codex" / "config.toml"
    config_before = digest(lock_config)
    authorization = subprocess.run([args.codex, "login", "status"],capture_output=True,text=True)
    if "Logged in using ChatGPT" not in authorization.stdout + authorization.stderr:
        raise RuntimeError("The benchmark requires an existing ChatGPT Codex sign-in")
    setup_started = time.perf_counter()
    rows = []
    with (args.output / "service.log").open("x") as service_log, (
         args.output / "codex-alone.log").open("x") as direct_log, (
         args.output / "codex-cookbook.log").open("x") as cookbook_log:
        service = subprocess.Popen([binary, "--state-dir", state, "service"], stdout=service_log,
                                   stderr=service_log)
        try:
            ready(binary, state, service)
            with app_server(args.codex, projects["codex"], {}, direct_log) as solo, app_server(
                    args.codex, projects["cookbook"], {"apr": cfg}, cookbook_log) as apr:
                setup_thread = apr.thread(projects["cookbook"], args.model)
                connection = connect_project(apr, setup_thread, projects["cookbook"])
                setup_workspace = connection["workspace"]
                cli(binary, state, "execution", "grant-python", "--assignment", connection["assignment_id"],
                    "--interpreter", Path(sys.executable).resolve())
                manifest = {"profiles": "process.json", "inventory": "inventory-brief.json",
                            "sla": "sla-brief.json"}
                refs = {task: apr.tool(setup_thread, "apr_manage", {
                    "workspace": setup_workspace, "action": "process.register", "path": manifest[task]
                })["process_ref"] for task in TASKS}
                source_artifacts = {(task, size): apr.tool(setup_thread, "apr_manage", {
                    "workspace": setup_workspace, "action": "artifact.import",
                    "path": f"{task}-{size}.csv", "media_type": "text/csv"
                }) for task in ("inventory", "sla") for size in SIZES[:args.rounds]}
                clients = {"codex": solo, "cookbook": apr}
                threads = {(task, arm): clients[arm].thread(projects[arm], args.model)
                           for task in TASKS for arm in ARMS}
                workspaces = {task: connect_project(apr, threads[task, "cookbook"], projects["cookbook"])
                              for task in TASKS}
                assert all(v["assignment_id"] == connection["assignment_id"] for v in workspaces.values())
                servers = {}
                for task in TASKS:
                    for arm in ARMS:
                        inventory = clients[arm].call("mcpServerStatus/list", {
                            "threadId": threads[task, arm], "detail": "toolsAndAuthOnly"
                        })["data"]
                        enabled = sorted(row["name"] for row in inventory if row.get("tools"))
                        if enabled != ([] if arm == "codex" else ["apr"]):
                            raise RuntimeError(f"Unexpected MCP tools in {arm}: {enabled}")
                        servers[arm] = enabled
                save(args.output / "protocol.json", {
                    "verified_mcp_servers": servers,
                    "model": args.model, "effort": "low", "rounds": args.rounds,
                    "installed_binary_sha256": digest(binary), "recipe_pins": refs,
                    "codex_version": subprocess.check_output([args.codex, "--version"], text=True).strip(),
                    "setup_seconds": round(time.perf_counter() - setup_started, 3),
                    "comparison": "Both arms prepared before timing; one warm thread per task and arm.",
                    "usage": "Per-turn delta in cumulative client counts; includes cached input.",
                })
                for index in range(args.rounds):
                    for task_index in range(len(TASKS)):
                        task = TASKS[(index + task_index) % len(TASKS)]
                        if task == "profiles":
                            emails, expected = profiles_fixture(index)
                            inputs, report_rows = {"emails": emails}, None
                            script = ["python3", "profiles.py", *emails]
                        else:
                            size = SIZES[index]
                            _, expected, report_rows, _ = fixture(task, size, projects["codex"])
                            inputs = {"source": source_artifacts[task, size]}
                            script = ["python3", f"{task}.py", f"{task}-{size}.csv", f"{task}-{size}-output.json"]
                        start = (index + TASKS.index(task)) % 2
                        for arm in ARMS[start:] + ARMS[:start]:
                            row = execute_row(
                                arm, task, index, clients[arm], threads[task, arm],
                                workspaces[task]["workspace"], refs[task], inputs, expected, report_rows,
                                script, binary, state,
                                projects[arm] / f"{task}-{index}-verified.json",
                            )
                            row["input_rows"] = len(inputs["emails"]) if task == "profiles" else size
                            rows.append(row)
                            save(args.output / "metrics.json", rows)
                            print(json.dumps(row), flush=True)
        finally:
            service.terminate()
            try:
                service.wait(timeout=5)
            except subprocess.TimeoutExpired:
                service.kill()
                service.wait()
    assert digest(lock_config) == config_before, "Codex configuration moved during the study"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", default="codex")
    parser.add_argument("--model", default="gpt-6-sol")
    parser.add_argument("--rounds", type=int, choices=(1, 2, 3, 4), default=4)
    parser.add_argument("--output", type=Path, required=True, help="New private directory")
    arguments = parser.parse_args()
    arguments.output = arguments.output.resolve()
    run(arguments)
