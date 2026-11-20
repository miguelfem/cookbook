#!/usr/bin/env python3
"""Measure a synthetic profile batch using local workers and optional ChatGPT Codex."""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

from measure_batch_workers import Mcp, cli, digest, ready, save


FIXTURE = Path(__file__).resolve().parents[2] / "examples" / "profile-batch"
# Frozen input ordering and oracle. Do not import the worker to compute the expected output.
EMAILS = [f"person{number:02d}@example.test" for number in (3, 1, 8, 5, 2, 4, 7, 6)]
EXPECTED = {
    "profiles": [
        {"key": email, "output": dict(zip(("name", "organization", "role"), fields, strict=True))}
        for email, fields in zip(EMAILS, (
            ("Example Person 03", "Cedar Lab", "Writer"),
            ("Example Person 01", "Fable Studio", "Engineer"),
            ("Example Person 08", "Harbor Works", "Designer"),
            ("Example Person 05", "Harbor Works", "Engineer"),
            ("Example Person 02", "Harbor Works", "Researcher"),
            ("Example Person 04", "Fable Studio", "Designer"),
            ("Example Person 07", "Fable Studio", "Writer"),
            ("Example Person 06", "Cedar Lab", "Researcher"),
        ), strict=True)
    ]
}


@contextmanager
def prepared(binary, folder, workers):
    started = time.perf_counter()
    project, state = folder / "project", folder / "state"
    shutil.copytree(FIXTURE, project)
    state.mkdir(mode=0o700)
    project_id = cli(binary, state, "project", "attach", project)["project_id"]
    assignment = cli(binary, state, "assignment", "create", "--project", project_id,
                     "--name", "Synthetic profile batch")["assignment_id"]
    author, runner = state / "attachments" / "author.json", state / "attachments" / "runner.json"
    for mode, output in (("author", author), ("run", runner)):
        cli(binary, state, "attachment", "create", "--assignment", assignment,
            "--mode", mode, "--output-file", output)
    cli(binary, state, "execution", "grant-python", "--assignment", assignment,
        "--interpreter", Path(sys.executable).resolve())
    policy_file = folder / "policy.json"
    save(policy_file, {"revision": 0, "max_workers": workers, "max_local_workers": workers,
                       "per_connection": {}})
    cli(binary, state, "execution", "worker-policy-set", "--policy-file", policy_file,
        "--expected-revision", 0)

    with (folder / "service.log").open("x") as log:
        service = subprocess.Popen([binary, "--state-dir", state, "service"],
                                   stdout=log, stderr=log)
        try:
            ready(binary, state, service)
            author_mcp = Mcp(binary, state, author, log)
            try:
                ref = author_mcp.tool("apr_manage", {"action": "process.register",
                                                     "path": "process.json"})["process_ref"]
            finally:
                author_mcp.close()
            mcp = Mcp(binary, state, runner, log)
            try:
                setup = time.perf_counter() - started
                yield mcp, ref, state, runner, setup
            finally:
                mcp.close()
        finally:
            service.terminate()
            try:
                service.wait(timeout=5)
            except subprocess.TimeoutExpired:
                service.kill()
                service.wait()


def request(ref, key):
    return {"process_ref": ref, "mode": "start", "input": {"emails": EMAILS},
            "request_key": key, "wait_ms": 5000, "result_view": "compact"}


def compact(value):
    return json.dumps(value, separators=(",", ":"))


def runtime_trials(binary, output):
    rows = []
    with prepared(binary, output / "serial", 1) as serial, prepared(binary, output / "parallel", 4) as parallel:
        arms = ("apr-1", "apr-4", "script-1", "script-4")
        for trial in range(4):
            for arm in arms[trial:] + arms[:trial]:
                if arm.startswith("apr"):
                    mcp, ref, _, _, setup = serial if arm == "apr-1" else parallel
                    started = time.perf_counter()
                    result = mcp.tool("apr_run", request(ref, f"runtime-{trial}"))
                    seconds = time.perf_counter() - started
                    assert result["state"] == "succeeded" and result.get("output_complete") is True, result
                    observed = result["output"]
                    response_bytes = len(compact(mcp.events[-1]["response"]["result"]).encode())
                    same = mcp.tool("apr_run", request(ref, f"runtime-{trial}"))
                    assert same["run_id"] == result["run_id"] and same["output"] == observed
                else:
                    setup = None
                    started = time.perf_counter()
                    result = subprocess.check_output(
                        [sys.executable, FIXTURE / "profiles.py", "--workers", arm[-1], *EMAILS]
                    )
                    seconds = time.perf_counter() - started
                    response_bytes = len(result)
                    observed = json.loads(result)
                assert observed == EXPECTED, (arm, observed)
                rows.append({"trial": trial, "arm": arm, "seconds": seconds, "setup_seconds": setup,
                             "result_bytes": response_bytes, "calls": 1, "exact": True})
    save(output / "runtime.json", rows)


def direct_lookup(project):
    """Provide the same primitive lookup to the direct-agent arm."""
    worker = (FIXTURE / "profiles.py").read_text()
    functions = worker[:worker.index("\ndef main()")]
    (project / "lookup.py").write_text(functions + """
if __name__ == "__main__":
    for address in sys.argv[1:]:
        print(json.dumps({"email": address, **profile(address)}))
""")


def agent_case(codex, model, binary, output, arm, trial, ref, state, attachment):
    folder = output / f"trial-{trial}-{arm}"
    project = folder / "project"
    project.mkdir(parents=True)
    common = ("Look up these eight fictional addresses using the supplied local tool. "
              "Return only plain JSON with shape "
              '{"profiles":[{"key":"<email>","output":{"name":"<name>","organization":"<organization>","role":"<role>"}}]} '
              "in the original email order. Use only observed lookup results. No network.")
    command = [
        codex, "exec", "--ignore-user-config", "--ephemeral",
        "--skip-git-repo-check", "--approve-for-me", "--json", "--color", "never",
        "-C", project, "-m", model, "-c", 'model_reasoning_effort="low"',
        "-o", folder / "final.json",
    ]
    if arm == "apr":
        command += [
            "-c", f"mcp_servers.cookbook.command={json.dumps(str(binary))}",
            "-c", "mcp_servers.cookbook.args=" + json.dumps(
                ["--state-dir", str(state), "mcp", "--attachment-file", str(attachment)]),
        ]
        prompt = common + "\nUse the prepared CookBook tool apr_run with these arguments:\n" + compact(
            request(ref, f"agent-{trial}"))
    elif arm == "script":
        shutil.copyfile(FIXTURE / "profiles.py", project / "profiles.py")
        prompt = (common + "\nRun python3 profiles.py followed by the addresses. It returns the full JSON.\n"
                  + compact(EMAILS))
    else:
        direct_lookup(project)
        prompt = (common + "\npython3 lookup.py accepts one or several addresses and prints one record "
                  "per line. You can batch and use shell or inline Python to transform the records.\n"
                  + compact(EMAILS))
    started = time.perf_counter()
    tool_types = {"command_execution", "mcp_tool_call", "web_search"}
    with (folder / "events.jsonl").open("w") as events, (folder / "stderr.log").open("w") as stderr:
        try:
            exit_code = subprocess.run(command + ["-"], input=prompt, text=True, stdout=events,
                                       stderr=stderr, timeout=100).returncode
        except subprocess.TimeoutExpired:
            exit_code = None
    elapsed = time.perf_counter() - started
    events = [json.loads(line) for line in (folder / "events.jsonl").read_text().splitlines() if line]
    completed = [event["item"] for event in events if event["type"] == "item.completed"
                 and event["item"]["type"] in tool_types]
    errors = [item["error"]["message"] for item in completed if item.get("error")]
    usage = next((event["usage"] for event in reversed(events)
                  if event["type"] == "turn.completed"), None)
    try:
        exact = json.loads((folder / "final.json").read_text()) == EXPECTED
    except (OSError, ValueError):
        exact = False
    return {"trial": trial, "arm": arm, "seconds": elapsed, "exit_code": exit_code,
            "exact": exact, "usage": usage,
            "calls": [item.get("tool", item["type"]) for item in completed], "errors": errors}


def agent_trials(args, binary, output):
    identity = subprocess.run([args.codex, "login", "status"], capture_output=True, text=True)
    if "Logged in using ChatGPT" not in identity.stdout + identity.stderr:
        raise SystemExit("Optional agent benchmark requires ChatGPT sign-in.")
    # Avoid falling back to separate API billing through inherited credentials.
    os.environ.pop("OPENAI_API_KEY", None)
    os.environ.pop("CODEX_API_KEY", None)
    rows = []
    with prepared(binary, output / "prepared", 4) as (_, ref, state, attachment, setup):
        save(output / "agent-setup.json", {"elapsed_seconds": setup, "model": args.model})
        arms = ("direct", "apr", "script")
        for trial in range(args.agent_trials):
            for arm in arms[trial:] + arms[:trial]:
                row = agent_case(args.codex, args.model, binary, output, arm, trial, ref, state, attachment)
                rows.append(row)
                save(output / "agent.json", rows)
                print(compact(row), flush=True)
            if any(row["exit_code"] != 0 for row in rows):
                break


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apr", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New private scratch directory")
    parser.add_argument("--agent-only", action="store_true", help="Skip the local worker trials")
    parser.add_argument("--codex", help="Explicit CLI binary for optional agent trials")
    parser.add_argument("--model", default="gpt-6-sol")
    parser.add_argument("--agent-trials", type=int, choices=(1, 2, 3), default=3)
    args = parser.parse_args()
    if args.agent_only and not args.codex:
        parser.error("--agent-only requires --codex")
    binary = args.apr.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(mode=0o700)
    save(output / "protocol.json", {"binary_sha256": digest(binary), "emails": EMAILS,
                                    "oracle": EXPECTED, "manifest_sha256": digest(FIXTURE / "process.json"),
                                    "fixture_sha256": digest(FIXTURE / "profiles.py")})
    if not args.agent_only:
        runtime_trials(binary, output)
    if args.codex:
        agent_trials(args, binary, output)


if __name__ == "__main__":
    main()
