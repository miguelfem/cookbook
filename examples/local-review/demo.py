#!/usr/bin/env python3
"""Exercise real APR CLI handoffs with an explicitly supplied review answer.

This helper invokes no model, provider, or external service. It keeps the input,
run envelope, submission and artifact reference in a private local directory.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


REPOSITORY = Path(__file__).resolve().parents[2]
FIXTURE = Path(__file__).resolve().parent


def private_json(path, value):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=path.parent, delete=False
        ) as handle:
            temporary = Path(handle.name)
            json.dump(value, handle, indent=2)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        temporary.replace(path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apr", type=Path, default=REPOSITORY / "target/debug/apr")
    parser.add_argument("--state-dir", type=Path, default=REPOSITORY / "work/local-review/state")
    parser.add_argument("--demo-dir", type=Path, default=REPOSITORY / "work/local-review/client")
    commands = parser.add_subparsers(dest="command", required=True)
    start = commands.add_parser("start", help="Import the fixture and run until review is needed")
    start.add_argument("--request-key", default="local-review-demo-1")
    submit = commands.add_parser("submit", help="Return an explicitly supplied review answer")
    submit.add_argument("--approved", choices=("true", "false"), required=True)
    export = commands.add_parser("export", help="Export the completed bundle to a new file")
    export.add_argument("--output-file", type=Path)
    args = parser.parse_args()
    directory = args.demo_dir.resolve()
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)

    def apr(*command):
        result = subprocess.run(
            [str(args.apr.resolve()), "--state-dir", str(args.state_dir.resolve()), *command],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode:
            sys.stderr.write(result.stderr)
            raise SystemExit(result.returncode)
        return json.loads(result.stdout)

    if args.command == "start":
        artifact = apr("artifact", "import", str(FIXTURE / "source.txt"), "--media-type", "text/plain")
        private_json(directory / "input.json", {"record": {"title": "Local review fixture"}, "sources": [artifact]})
        run = apr(
            "run", str(FIXTURE / "process.json"), "--input-file", str(directory / "input.json"),
            "--request-key", args.request_key,
        )
        private_json(directory / "run.json", run)
        result = run
    elif args.command == "submit":
        saved = json.loads((directory / "run.json").read_text())
        current = apr("inspect", saved["run_id"])
        if "request" in current:
            request = current["request"]
            submission = {
                "protocol_version": "0.1", "action": "submit", "run_id": current["run_id"],
                "request_id": request["request_id"], "step_instance_id": request["step_instance_id"],
                "generation": request["generation"], "continuation_token": request["continuation_token"],
                "submission_key": "local-review-answer-1",
                "provenance": {
                    "source": "human_cli_demo", "client": "apr-cli",
                    "verification": "explicit_demo_answer",
                },
            }
        else:
            # Retry the same saved submission after an accepted response was lost.
            # A changed answer keeps the key and is rejected as a conflict by APR.
            submission = json.loads((directory / "submission.json").read_text())
        submission["output"] = {"approved": args.approved == "true"}
        private_json(directory / "submission.json", submission)
        result = apr("resume", current["run_id"], "--result-file", str(directory / "submission.json"))
        private_json(directory / "run.json", result["run"])
    else:
        saved = json.loads((directory / "run.json").read_text())
        current = apr("inspect", saved["run_id"])
        if current["state"] != "succeeded":
            raise ValueError("run has not completed")
        private_json(directory / "package-reference.json", current["output"]["package"])
        # Keep the chosen directory entry intact so the CLI can reject a
        # pre-existing symlink as well as an ordinary file.
        target = (args.output_file or directory / "package.zip").absolute()
        result = apr(
            "artifact", "read", "--reference-file", str(directory / "package-reference.json"),
            "--output-file", str(target),
        )
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError):
        print(json.dumps({"error": {
            "code": "DEMO_ERROR",
            "message": "The demo needs valid local files and a completed prior step. Check the README and chosen paths.",
        }}), file=sys.stderr)
        raise SystemExit(1)
