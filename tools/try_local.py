#!/usr/bin/env python3
"""Open CookBook APR with fresh synthetic data and no provider account."""

import argparse
import json
import os
from pathlib import Path
import shutil
import runpy
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import webbrowser


REPOSITORY = Path(__file__).resolve().parents[1]


class DemoError(RuntimeError):
    """A safe launcher diagnostic containing no remote response bodies or tokens."""


def demo_depth(value):
    depth = int(value)
    if not 1 <= depth <= 32:
        raise argparse.ArgumentTypeError(
            "demo depth must be 1–32 (default owner capacity); deeper owner-configured workflows support up to 128"
        )
    return depth


def create_workspace(path):
    if path is None:
        return Path(tempfile.mkdtemp(prefix="cookbook-apr-", dir="/tmp")).resolve()
    path = path.expanduser().absolute()
    if path.is_symlink():
        raise DemoError("Workspace must be a real directory, not a symlink.")
    if path.exists():
        if not path.is_dir() or any(path.iterdir()):
            raise DemoError("Workspace must be new or empty. Reopen an existing instance with its printed inspector command.")
        if path.stat().st_uid != os.getuid():
            raise DemoError("Workspace must be owned by the current user.")
    else:
        path.mkdir(mode=0o700, parents=True)
    path.chmod(0o700)
    return path.resolve()


def paged_runs(call, *, root_run_id=None):
    args = {"view": "tree", "run_id": root_run_id, "limit": 100} if root_run_id else {"view": "runs", "limit": 20}
    seen = set()
    while True:
        page = call("inspect", args)
        yield from page["runs"]
        if not page.get("truncated"):
            return
        cursor = page.get("next_cursor")
        if not cursor or cursor in seen:
            raise DemoError("Run inspection returned a missing or repeated pagination cursor.")
        seen.add(cursor)
        args["after"] = cursor


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPOSITORY / "target"))
    parser.add_argument("--apr", type=Path, default=target / "debug/apr")
    parser.add_argument("--no-browser", action="store_true", help="Print the local URL without opening it")
    parser.add_argument("--depth", type=demo_depth, default=12,
                        help="Nested call depth 1–32; root is depth 0 (default: 12). Deeper workflows require explicit owner capacity.")
    parser.add_argument("--workspace", type=Path,
                        help="New or empty private persistent instance directory (default: a temporary directory)")
    parser.add_argument("--verify-nesting", action="store_true",
                        help="Verify a synthetic nested handoff and replay, then close the inspector")
    args = parser.parse_args()
    binary = args.apr.resolve()
    if not binary.is_file():
        parser.error("Build first: cargo build --locked -p apr-cli (or select --apr PATH)")
    if os.name != "posix":
        parser.error("This development demo requires macOS or Linux.")

    root = create_workspace(args.workspace)
    print(f"Private instance directory: {root}", flush=True)
    project = root / "project"
    project.mkdir(mode=0o700)
    shutil.copytree(REPOSITORY / "examples/local-review", project / "local-review")
    state = root / "state"

    def cli(*command):
        result = subprocess.run(
            [str(binary), "--state-dir", str(state), *command],
            capture_output=True, text=True, timeout=30, check=True,
        )
        return json.loads(result.stdout)

    attached = cli("project", "attach", str(project))
    assignment = cli("assignment", "create", "--project", attached["project_id"],
                     "--name", "CookBook APR demo")
    attachment = state / "attachments/demo.json"
    cli("attachment", "create", "--assignment", assignment["assignment_id"],
        "--mode", "author", "--output-file", str(attachment))

    with (root / "inspector.log").open("w") as log, (root / "inspector-errors.log").open("w") as errors:
        inspector = subprocess.Popen(
            [str(binary), "inspector", "--attachment", str(attachment), "--port", "0"],
            stdin=subprocess.DEVNULL, stdout=log, stderr=errors,
        )
        try:
            deadline = time.monotonic() + 15
            while True:
                if inspector.poll() is not None:
                    raise RuntimeError("The inspector exited during startup.")
                try:
                    url = json.loads((root / "inspector.log").read_text())["url"]
                    break
                except (ValueError, KeyError):
                    if time.monotonic() >= deadline:
                        raise RuntimeError("The inspector did not start in time.") from None
                    time.sleep(0.1)
            parsed = urllib.parse.urlsplit(url)
            if parsed.scheme != "http" or parsed.hostname != "127.0.0.1":
                raise RuntimeError("Expected a local inspector URL.")
            origin = f"{parsed.scheme}://{parsed.netloc}"
            token = urllib.parse.parse_qs(parsed.fragment)["session"][0]
            # Local demo traffic must not be sent to a configured HTTP proxy.
            http = urllib.request.build_opener(urllib.request.ProxyHandler({}))

            def call(action, arguments):
                request = urllib.request.Request(
                    origin + "/api",
                    data=json.dumps({"action": action, "args": arguments, "detail": True}).encode(),
                    headers={"Origin": origin, "Content-Type": "application/json", "X-APR-Session": token},
                )
                try:
                    with http.open(request, timeout=30) as response:
                        return json.load(response)["result"]
                except urllib.error.HTTPError as error:
                    try:
                        code = json.load(error).get("error", {}).get("code", "REQUEST_FAILED")
                    except (ValueError, AttributeError):
                        code = "REQUEST_FAILED"
                    if not isinstance(code, str) or len(code) > 64 or not all(c.isupper() or c == "_" for c in code):
                        code = "REQUEST_FAILED"
                    raise DemoError(f"Runtime rejected demo {action}: {code}. Check this instance's owner capacity and use a current build; the launcher never raises owner policy.") from None

            process = call("manage", {"action": "process.register", "path": "local-review/process.json"})
            register_nested = runpy.run_path(str(REPOSITORY / "examples/nested-review/register.py"))["register_nested"]
            nested = register_nested(call, project, process["process_ref"], depth=args.depth)
            jev_sample = register_jev_sample(call, project)
            artifact = call("manage", {"action": "artifact.import", "path": "local-review/source.txt",
                                       "media_type": "text/plain"})
            input_value = {"record": {"title": "Synthetic document review"}, "sources": [artifact]}
            run = call("run", {"mode": "start", "process_ref": process["process_ref"],
                               "input": input_value, "config_overrides": {}, "request_key": "demo-review-1"})
            deadline = time.monotonic() + 15
            while True:
                run = call("inspect", {"view": "run", "run_id": run["run_id"]})
                if run["state"] == "waiting_for_agent":
                    break
                if run["state"] not in ("queued", "running") or time.monotonic() >= deadline:
                    raise RuntimeError("The sample workflow did not reach its review handoff.")
                time.sleep(0.1)
            nested_start = {"mode": "start", "process_ref": nested["root"],
                            "input": input_value, "config_overrides": {}, "request_key": "demo-nested-review-1"}
            nested_run = call("run", nested_start)
            nested_family = [nested_run["run_id"]]
            while True:
                deadline = time.monotonic() + 15
                while True:
                    current = call("inspect", {"view": "run", "run_id": nested_family[-1]})
                    if current["state"] in ("waiting_for_child", "waiting_for_agent"):
                        break
                    if current["state"] not in ("queued", "running") or time.monotonic() >= deadline:
                        raise RuntimeError("The nested workflow did not reach its child review handoff.")
                    time.sleep(0.1)
                if current["state"] == "waiting_for_agent":
                    break
                children = [step["child_run_id"] for step in current["steps"] if step.get("child_run_id")]
                if len(children) != 1 or children[0] in nested_family or len(nested_family) > args.depth:
                    raise DemoError("The demo did not produce the declared bounded child chain.")
                nested_family.append(children[0])
            leaf = current
            if len(nested_family) != args.depth + 1:
                raise DemoError("The saved nested chain differs from the requested depth.")
            # The sample schedule is paused before the page opens; it cannot
            # leave unattended recurring work merely from trying the demo.
            job = call("manage", {"action": "schedule.create", "name": "Sample hourly review",
                                  "process_ref": process["process_ref"], "input": input_value,
                                  "config_overrides": {}, "request_key": "demo-hourly-1",
                                  "trigger": {"kind": "interval", "every_seconds": 3600}})
            call("manage", {"action": "schedule.pause", "schedule_id": job["schedule_id"]})
            (root / "sample-input.json").write_text(json.dumps(input_value, indent=2) + "\n")
            if args.verify_nesting:
                verify_nesting(call, nested_start, nested_family, leaf, job["schedule_id"])
                history = paged_runs(call)
                if any(saved["process_version"] == jev_sample["process_ref"] for saved in history):
                    raise RuntimeError("The Jev tuning sample must never start during demo setup.")
                print(f"Nested verification passed: depth {args.depth}, {len(nested_family)} family runs; replay preserved identities and receipt.")
                print("Jev tuning sample registered only; no Jev run was started.")
                print(f"Private verification files: {root}", flush=True)
                return
            print("CookBook APR is ready: a standalone review, a nested review, and one paused hourly job.")
            print(f"Nested depth: {args.depth}; root: {nested_family[0]}; pending review leaf: {nested_family[-1]}")
            print("Discover demo.jev-intake-routing to view and tune Jev questions and routes without inference.")
            print("The Jev sample has not been started. Live execution requires a separately configured Jev adapter, binding, and grant.")
            print(f"Demo files: {root}")
            print(f"Sample input: {root / 'sample-input.json'}")
            print(f"Reopen later: {binary} inspector --attachment {attachment} --port 0")
            print("Private local session URL (keep it on this device):")
            print(url, flush=True)
            print("Keep this terminal open. Ctrl+C closes the inspector; saved demo state remains.", flush=True)
            if not args.no_browser:
                webbrowser.open(url)
            inspector.wait()
        finally:
            if inspector.poll() is None:
                inspector.terminate()
                try:
                    inspector.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    inspector.kill()
                    inspector.wait()


def register_jev_sample(call, project):
    """Expose a synthetic editable diagram without running a provider."""
    manifest = json.loads((REPOSITORY / "examples/jev-intake/process.json").read_text())
    manifest.update({
        "schema_version": "0.2", "name": "demo.jev-intake-routing",
        "description": "Synthetic Jev questions and routing for offline visual tuning; execution requires separate Jev setup.",
    })
    manifest["capabilities"].append({"kind": "builtin"})
    manifest["limits"].update({"max_call_depth": 32, "max_child_runs": 1024,
                               "max_step_instances": 10000})
    routes = ["operations", "engineering", "unclear"]
    manifest["steps"].append({
        "id": "route", "kind": "decision", "effect": "pure", "needs": ["observe"],
        "inputs": {}, "retry": {"max_attempts": 1},
        "decision": {
            "source": "choice", "routes": routes,
            "value": {"step": "observe", "pointer": "/answers/team/choice"},
            "confidence": {"step": "observe", "pointer": "/answers/team/confidence"},
            "min_confidence": 0.75, "no_match": "unclear", "needs_clarification": "unclear",
        },
    })
    for route in routes:
        manifest["steps"].append({
            "id": route, "kind": "builtin", "effect": "pure", "operation": "json.select",
            "needs": ["route"], "when": {"decision": "route", "route": route},
            "retry": {"max_attempts": 1},
            "inputs": {"value": {"literal": {"team": route, "synthetic": True}},
                       "pointer": {"literal": ""}},
        })
    manifest["steps"].append({
        "id": "result", "kind": "join", "effect": "pure", "needs": ["route", *routes],
        "inputs": {}, "retry": {"max_attempts": 1},
        "join": {"decision": "route", "branches": {
            route: {"step": route, "pointer": "/value"} for route in routes
        }},
    })
    manifest["output"] = {"result": {"step": "result", "pointer": ""}}
    directory = project / "jev-tuning"
    directory.mkdir(mode=0o700)
    (directory / "process.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return call("manage", {"action": "process.register", "path": "jev-tuning/process.json"})


def verify_nesting(call, start, family, leaf, schedule_id):
    """Exercise real persisted child identities and receipts, with no model calls."""
    def require(condition, message):
        if not condition:
            raise RuntimeError(message)

    require(len(set(family)) == len(family) and len(family) > 1, "Nested runs must have distinct identities.")
    saved_family = list(paged_runs(call, root_run_id=family[0]))
    require({run["run_id"] for run in saved_family} == set(family), "Saved family differs from traversed child chain.")
    replay = call("run", start)
    require(replay["run_id"] == family[0], "Same start key created another root.")
    request = leaf["request"]
    submission = {key: request[key] for key in (
        "request_id", "step_instance_id", "generation", "continuation_token",
    )}
    submission.update({
        "protocol_version": "0.1", "action": "submit", "run_id": leaf["run_id"],
        "submission_key": "demo-nested-fixture-answer-1", "output": {"approved": True},
        "provenance": {"source": "synthetic_fixture", "client": "try-local-verification",
                       "verification": "explicit_fixture_answer_not_model_review"},
    })
    accepted = call("resume", submission)
    repeated = call("resume", submission)
    require(accepted["receipt"]["status"] == "accepted", "Leaf submission was not accepted.")
    require(accepted["receipt"] == repeated["receipt"], "Submission replay changed the receipt.")
    deadline = time.monotonic() + 30
    while True:
        root = call("inspect", {"view": "run", "run_id": family[0]})
        if root["state"] == "succeeded":
            break
        require(root["state"] in ("running", "waiting_for_child") and time.monotonic() < deadline,
                "Nested root failed to finish after the leaf result.")
        time.sleep(0.1)
    for index, run_id in enumerate(family):
        saved = call("inspect", {"view": "run", "run_id": run_id})
        require(saved["state"] == "succeeded", "A nested family member did not succeed.")
        require(saved["root_run_id"] == family[0], "Nested root identity differs.")
        require(saved["output"] == root["output"], "Nested output or package reference changed.")
        if index:
            require(saved["parent_run_id"] == family[index - 1], "Nested parent identity differs.")
        if index < len(family) - 1:
            require(saved["steps"][0]["child_run_id"] == family[index + 1], "Child identity changed.")
    require(root["output"]["approved"] is True, "Final approval differs from fixture answer.")
    require(call("run", start)["run_id"] == family[0], "Completed start replay created a duplicate.")
    history = paged_runs(call, root_run_id=family[0])
    require({run["run_id"] for run in history} == set(family),
            "Unexpected duplicate nested family member.")
    schedule = call("inspect", {"view": "schedule", "schedule_id": schedule_id})
    require(schedule["state"] == "paused", "Sample schedule must remain paused.")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
    except DemoError as error:
        print(f"Demo could not start: {error}")
        raise SystemExit(1) from None
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as error:
        # Do not print response bodies or bearer-bearing request objects.
        print(f"Demo could not start ({type(error).__name__}). Check the build and local service.")
        raise SystemExit(1) from None
