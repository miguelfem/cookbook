#!/usr/bin/env python3
"""Repeat the matched comparison in fresh threads and trace the turn phases."""

import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import subprocess
import sys
import time

from benchmark_codex_head_to_head import (
    Client,
    SIZES,
    TASKS,
    app_server,
    canonical,
    check_artifact,
    cli,
    connect_project,
    create_projects,
    digest,
    execute_row,
    fixture,
    installed_entry,
    profiles_fixture,
    ready,
    save,
)

ARMS = ("codex", "cookbook", "lean")


class TraceClient(Client):
    def next(self, deadline):
        now, message = super().next(deadline)
        trace = getattr(self, "active_trace", None)
        if trace is not None:
            method = message.get("method")
            body = message.get("params", {})
            item = body.get("item", {})
            if body.get("threadId") == trace["thread"]:
                offset = round(now - trace["started"], 3)
                if method == "thread/tokenUsage/updated":
                    trace["usage_events"].append(
                        {"at": offset, **body["tokenUsage"]["last"]}
                    )
                if item.get("type") in ("mcpToolCall", "commandExecution"):
                    if method == "item/started":
                        trace["starts"].append(offset)
                    if method == "item/completed":
                        trace["ends"].append(offset)
                        result = item.get("result", item.get("aggregatedOutput"))
                        trace["result_bytes"] += len(canonical(result))
        return now, message

    def turn(self, thread, prompt):
        self.active_trace = {
            "thread": thread,
            "started": time.monotonic(),
            "usage_events": [],
            "starts": [],
            "ends": [],
            "result_bytes": 0,
        }
        try:
            answer, calls, row = super().turn(thread, prompt)
            trace = self.active_trace
            start = min(trace["starts"]) if trace["starts"] else None
            end = max(trace["ends"]) if trace["ends"] else None
            row["trace"] = {
                "before_tool_s": start,
                "tool_window_s": (
                    round(end - start, 3)
                    if start is not None and end is not None
                    else None
                ),
                "after_tool_s": (
                    round(row["seconds"] - end, 3) if end is not None else None
                ),
                "tool_result_bytes": trace["result_bytes"],
                "prompt_bytes": len(prompt.encode()),
                "usage_events": trace["usage_events"],
            }
            return answer, calls, row
        finally:
            self.active_trace = None


def tool_inventory(client, thread, expected):
    discovered = client.call(
        "mcpServerStatus/list", {"threadId": thread, "detail": "toolsAndAuthOnly"}
    )["data"]
    servers = {row["name"]: row["tools"] for row in discovered if row.get("tools")}
    if set(servers) != ({"apr"} if expected else set()):
        raise RuntimeError(f"Unexpected servers: {sorted(servers)}")
    tools = servers.get("apr", {})
    names = (
        sorted(tools)
        if isinstance(tools, dict)
        else sorted(item["name"] for item in tools)
    )
    if set(names) != set(expected):
        raise RuntimeError(f"Unexpected tools: {names}")
    return {"names": names, "serialized_bytes": len(canonical(tools)) if tools else 0}


def typed_row(
    client,
    thread,
    workspace,
    task,
    index,
    ref,
    inputs,
    expected,
    report_rows,
    binary,
    state,
    path,
):
    arguments = {
        "workspace": workspace,
        "process_ref": ref,
        "input": inputs,
        "mode": "start",
        "request_key": f"typed-{task}-{index}",
        "wait_ms": 5000,
        "result_view": "output",
    }
    started = time.monotonic()
    reply = client.tool(thread, "apr_run", arguments)
    seconds = time.monotonic() - started
    usage = client.usage.get(thread, {})
    if any(usage.values()):
        raise RuntimeError("A typed tool dispatch unexpectedly used model tokens")
    value = reply.get("output")
    exact = reply.get("output_complete") is True and (
        value == expected
        if task == "profiles"
        else (
            value.get("summary") == expected
            and check_artifact(
                binary,
                state,
                client,
                thread,
                workspace,
                value["report"],
                report_rows,
                path,
            )
        )
    )
    return {
        "seconds": round(seconds, 3),
        "status": reply["state"],
        "usage": {"totalTokens": 0},
        "arm": "typed",
        "task": task,
        "round": index,
        "tool_output_exact": exact,
        "tool_result_bytes": len(canonical(reply)),
    }


def run(output, model, rounds, typed_only=False):
    output.mkdir(mode=0o700)
    projects = create_projects(output)
    entry, binary, _ = installed_entry()
    state = output / "runtime"
    state.mkdir(mode=0o700)
    managed = {**entry, "args": ["--state-dir", str(state), "mcp", "--projects"]}
    lean = {**managed, "enabled_tools": ["apr_project", "apr_run"]}
    config = Path.home() / ".codex" / "config.toml"
    initial_config = digest(config)
    auth = subprocess.run(["codex", "login", "status"], capture_output=True, text=True)
    if "Logged in using ChatGPT" not in auth.stdout + auth.stderr:
        raise RuntimeError("The benchmark requires an existing ChatGPT sign-in")
    rows = []
    with ExitStack() as stack:
        service_log = stack.enter_context((output / "service.log").open("x"))
        service = subprocess.Popen(
            [binary, "--state-dir", state, "service"],
            stdout=service_log,
            stderr=service_log,
        )
        try:
            ready(binary, state, service)
            entries = {"codex": {}, "cookbook": {"apr": managed}, "lean": {"apr": lean}}
            arms = ("cookbook",) if typed_only else ARMS

            def path_for(arm):
                return projects["codex" if arm == "codex" else "cookbook"]

            clients = {
                arm: stack.enter_context(
                    app_server(
                        "codex",
                        path_for(arm),
                        entries[arm],
                        stack.enter_context((output / f"{arm}.log").open("x")),
                        TraceClient,
                    )
                )
                for arm in arms
            }
            full = clients["cookbook"]
            setup = full.thread(projects["cookbook"], model)
            connected = connect_project(full, setup, projects["cookbook"])
            ws = connected["workspace"]
            cli(
                binary,
                state,
                "execution",
                "grant-python",
                "--assignment",
                connected["assignment_id"],
                "--interpreter",
                Path(sys.executable).resolve(),
            )
            manifests = {
                "profiles": "process.json",
                "inventory": "inventory-brief.json",
                "sla": "sla-brief.json",
            }
            refs = {
                task: full.tool(
                    setup,
                    "apr_manage",
                    {"workspace": ws, "action": "process.register", "path": file},
                )["process_ref"]
                for task, file in manifests.items()
            }
            sources = {
                (task, size): full.tool(
                    setup,
                    "apr_manage",
                    {
                        "workspace": ws,
                        "action": "artifact.import",
                        "path": f"{task}-{size}.csv",
                        "media_type": "text/csv",
                    },
                )
                for task in ("inventory", "sla")
                for size in SIZES[:rounds]
            }
            expected_tools = {
                "codex": [],
                "lean": ["apr_project", "apr_run"],
                "cookbook": [
                    "apr_discover",
                    "apr_inspect",
                    "apr_manage",
                    "apr_project",
                    "apr_resume",
                    "apr_run",
                ],
            }
            protocol = {
                "model": None if typed_only else model,
                "reasoning": None if typed_only else "low",
                "rounds": rounds,
                "thread_policy": "fresh per case",
                "binary_sha256": digest(binary),
                "inventories": {},
                "recipe_pins": refs,
                "installed_client_config_changed": False,
            }
            for index in range(rounds):
                for task_index in range(len(TASKS)):
                    task = TASKS[(index + task_index) % len(TASKS)]
                    if task == "profiles":
                        emails, expected = profiles_fixture(index)
                        inputs, details = {"emails": emails}, None
                        script = ["python3", "profiles.py", *emails]
                        size = len(emails)
                    else:
                        size = SIZES[index]
                        _, expected, details, _ = fixture(task, size, projects["codex"])
                        inputs = {"source": sources[task, size]}
                        script = [
                            "python3",
                            f"{task}.py",
                            f"{task}-{size}.csv",
                            f"{task}-{size}-output.json",
                        ]
                    start = (index + TASKS.index(task)) % len(arms)
                    for arm in arms[start:] + arms[:start]:
                        client = clients[arm]
                        thread = client.thread(path_for(arm), model)
                        inventory = tool_inventory(client, thread, expected_tools[arm])
                        protocol["inventories"][arm] = inventory
                        save(output / "protocol.json", protocol)
                        workspace = None
                        if arm != "codex":
                            scope = connect_project(client, thread, path_for(arm))
                            assert scope["assignment_id"] == connected["assignment_id"]
                            workspace = scope["workspace"]
                        if typed_only:
                            case = typed_row(
                                client,
                                thread,
                                workspace,
                                task,
                                index,
                                refs[task],
                                inputs,
                                expected,
                                details,
                                binary,
                                state,
                                path_for(arm) / f"verified-typed-{task}-{index}.json",
                            )
                        else:
                            case = execute_row(
                                "codex" if arm == "codex" else "cookbook",
                                task,
                                index,
                                client,
                                thread,
                                workspace,
                                refs[task],
                                inputs,
                                expected,
                                details,
                                script,
                                binary,
                                state,
                                path_for(arm) / f"verified-{arm}-{task}-{index}.json",
                                audit=(full, setup, ws),
                                request_prefix=f"fresh-{arm}",
                            )
                            case["arm"] = arm
                        case.update(
                            input_rows=size, catalog_bytes=inventory["serialized_bytes"]
                        )
                        rows.append(case)
                        save(output / "metrics.json", rows)
                        print(json.dumps(case), flush=True)
        finally:
            stack.close()
            service.terminate()
            try:
                service.wait(timeout=5)
            except subprocess.TimeoutExpired:
                service.kill()
                service.wait()
    assert digest(config) == initial_config


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--model", default="gpt-6-sol")
    parser.add_argument("--rounds", type=int, choices=(1, 2, 3, 4), default=4)
    parser.add_argument(
        "--typed-only",
        action="store_true",
        help="Run a single typed MCP call per fixture",
    )
    args = parser.parse_args()
    run(args.output.resolve(), args.model, args.rounds, args.typed_only)
