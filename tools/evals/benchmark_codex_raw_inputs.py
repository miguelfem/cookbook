#!/usr/bin/env python3
"""Compare saved recipes with fresh Codex threads given raw inputs and task rules."""

import argparse
from contextlib import ExitStack
import csv
import json
from pathlib import Path
import shutil
import subprocess
import sys

from benchmark_codex_fresh_threads import TraceClient, tool_inventory
from benchmark_codex_head_to_head import (
    ROOT,
    SIZES,
    TASKS,
    app_server,
    cli,
    connect_project,
    digest,
    execute_row,
    fixture,
    installed_entry,
    parse_reply,
    profiles_fixture,
    ready,
    save,
)

APR_TOOLS = ["apr_discover", "apr_inspect", "apr_manage", "apr_project", "apr_resume", "apr_run"]
RULES = {
    "inventory": (
        "Compute delta as counted minus expected. Keep one row per input in original order with "
        "fields id, warehouse, expected, counted, delta, status. The numeric fields are numbers. "
        "Status is 'short' for negative delta, 'over' for positive delta and 'match' otherwise. "
        "The summary keys are checked (total rows), mismatched (nonzero delta rows), and net_delta "
        "(sum of deltas)."
    ),
    "sla": (
        "SLA limits in minutes are P1=30, P2=120 and P3=480. Blank first_response_min is unanswered. "
        "Otherwise elapsed minutes is first_response_min minus opened_min. Breach means elapsed "
        "is strictly greater than the limit. Keep one row per input in original order with fields "
        "id, queue, priority, elapsed_minutes (null for unanswered), limit_minutes, and status "
        "('unanswered', 'breach', or 'within'). The summary keys are checked (total rows), breached, "
        "and unanswered."
    ),
}


def raw_prompt(task, work, source, emails):
    fixture_path = work / source.name
    shutil.copyfile(source, fixture_path)
    guard = "Use only the fixture in your current folder and local computation. Do not use the network or any saved script, skill, or recipe. You may write or run whatever code you need from scratch."
    if task == "profiles":
        return (
            f"Use {source.name} as the fictional profile directory. Look up these emails in order: {', '.join(emails)}. "
            "Reply only with plain JSON with one key profiles. The value is the ordered array of "
            "objects with key (email) and output (object with name, organization, role). "
            + guard
        )
    return (
        f"Process the synthetic CSV {source.name}. {RULES[task]} "
        'Save the complete report as report.json with the shape {"rows":[...]}. Reply only with the '
        "summary counts as a plain JSON object. " + guard
    )


def run_raw(client, thread, prompt, task, work, expected, rows):
    final, calls, result = client.turn(thread, prompt)
    chat_exact = parse_reply(final) == expected
    if task == "profiles":
        # The ordered list is the deliverable. A terminal-generated version is optional.
        output_exact = chat_exact
    else:
        try:
            output_exact = json.loads((work / "report.json").read_text()) == {"rows": rows}
        except (OSError, ValueError):
            output_exact = False
    commands = [item.get("command", "") for item, _ in calls if item["type"] == "commandExecution"]
    result.update(
        output_exact=output_exact, chat_reply_exact=chat_exact,
        raw_command_count=len(commands),
        outside_fixture_reference=any(
            "SKILL.md" in command or ".agents/skills" in command or ".codex/skills" in command
            or "/source/examples/" in command or "/prepared/" in command for command in commands
        ),
    )
    return result


def profile_directory(path):
    _, all_profiles = profiles_fixture(3)
    with path.open("w", newline="") as stream:
        file = csv.DictWriter(stream, fieldnames=["email", "name", "organization", "role"])
        file.writeheader()
        for row in sorted(all_profiles["profiles"], key=lambda row: row["key"]):
            file.writerow({"email": row["key"], **row["output"]})
    return path


def run(output, model, rounds):
    output.mkdir(mode=0o700)
    entry, binary, _ = installed_entry()
    config = Path.home() / ".codex" / "config.toml"
    before = digest(config)
    login = subprocess.run(["codex", "login", "status"], capture_output=True, text=True)
    if "Logged in using ChatGPT" not in login.stdout + login.stderr:
        raise RuntimeError("Use the existing ChatGPT Codex sign-in for this study")
    state, project, raw_root = (output / name for name in ("runtime", "prepared", "raw"))
    for directory in (state, project, raw_root):
        directory.mkdir(mode=0o700)
    package = {"profiles": "process.json", "inventory": "inventory-brief.json", "sla": "sla-brief.json"}
    for task, manifest in package.items():
        example = ROOT / "examples" / ("profile-batch" if task == "profiles" else "operation-reports")
        for file in (manifest, f"{task}.py"):
            shutil.copyfile(example / file, project / file)
    directory = profile_directory(output / "profiles-directory.csv")
    entry = {**entry, "args": ["--state-dir", str(state), "mcp", "--projects"]}
    rows = []
    with ExitStack() as stack:
        logs = {name: stack.enter_context((output / (name + ".log")).open("x"))
                for name in ("runtime", "cookbook", "raw")}
        service = subprocess.Popen([binary, "--state-dir", state, "service"],
                                   stdout=logs["runtime"], stderr=logs["runtime"])
        try:
            ready(binary, state, service)
            full = stack.enter_context(app_server("codex", project, {"apr": entry}, logs["cookbook"], TraceClient))
            solo = stack.enter_context(app_server("codex", raw_root, {}, logs["raw"], TraceClient))
            setup = full.thread(project, model)
            scope = connect_project(full, setup, project)
            setup_ws = scope["workspace"]
            cli(binary, state, "execution", "grant-python", "--assignment", scope["assignment_id"],
                "--interpreter", Path(sys.executable).resolve())
            refs = {task: full.tool(setup, "apr_manage",
                        {"workspace": setup_ws, "action": "process.register", "path": manifest})["process_ref"]
                    for task, manifest in package.items()}
            save(output / "protocol.json", {
                "model": model, "reasoning": "low", "rounds": rounds,
                "binary_sha256": digest(binary), "recipe_pins": refs,
                "case_setup": "fresh ephemeral thread and new raw-only directory per pair",
                "install_config_sha_verified_on_exit": True,
            })
            fixtures = {}
            for task in ("inventory", "sla"):
                for size in SIZES[:rounds]:
                    source, expected, details, _ = fixture(task, size, project)
                    reference = full.tool(setup, "apr_manage", {
                        "workspace": setup_ws, "action": "artifact.import", "path": source.name,
                        "media_type": "text/csv",
                    })
                    fixtures[task, size] = source, expected, details, reference
            for index in range(rounds):
                for number, task in enumerate(TASKS):
                    if task == "profiles":
                        emails, expected = profiles_fixture(index)
                        source, details, inputs, size = directory, None, {"emails": emails}, len(emails)
                    else:
                        size = SIZES[index]
                        source, expected, details, reference = fixtures[task, size]
                        inputs, emails = {"source": reference}, []
                    work = raw_root / f"{task}-{size}"
                    work.mkdir()
                    prompt = raw_prompt(task, work, source, emails)
                    order = ("raw", "cookbook") if (index + number) % 2 == 0 else ("cookbook", "raw")
                    for arm in order:
                        if arm == "raw":
                            assert list(work.iterdir()) == [work / source.name]
                            thread = solo.thread(work, model)
                            tool_inventory(solo, thread, [])
                            case = run_raw(solo, thread, prompt, task, work, expected, details)
                        else:
                            thread = full.thread(project, model)
                            tool_inventory(full, thread, APR_TOOLS)
                            connected = connect_project(full, thread, project)
                            assert connected["assignment_id"] == scope["assignment_id"]
                            case = execute_row("cookbook", task, index, full, thread, connected["workspace"],
                                refs[task], inputs, expected, details, None, binary, state,
                                project / f"verify-{task}-{size}.json", audit=(full, setup, setup_ws),
                                request_prefix="raw-comparison")
                            case["output_exact"] = case.pop("tool_output_exact")
                        case.update(arm=arm, task=task, round=index, input_rows=size,
                                    end_to_end_exact=case["output_exact"] and case["chat_reply_exact"])
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
    assert digest(config) == before


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--model", default="gpt-6-sol")
    parser.add_argument("--rounds", type=int, default=4, choices=(1, 2, 3, 4))
    args = parser.parse_args()
    run(args.output.resolve(), args.model, args.rounds)
