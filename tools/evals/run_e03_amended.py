"""Execute only untouched amended OpenRouter E03 cells, stopping on uncertainty.

Each started cell is journaled before waiting for its result. The script refuses
to run a second time against an existing journal; inspect the scoped run instead.
"""

from __future__ import annotations

import json
import hashlib
import os
import re
import time
import argparse
from pathlib import Path

from provider_runtime import AprMcp

ROOT = Path(__file__).resolve().parents[2]
PROTOCOL = json.loads((ROOT / "docs/evaluations/fixtures/e03/protocol.json").read_text())
MODEL = "openai/gpt-4.1-mini"
RESERVED_PER_CALL_USD = (2500 * 0.40 + 240 * 1.60) / 1_000_000


def save(journal: Path, row: dict) -> None:
    with journal.open("a") as stream:
        stream.write(json.dumps(row, sort_keys=True) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def grade(case_id: str, preview: str) -> bool:
    case = next(item for item in PROTOCOL["cases"] if item["id"] == case_id)
    match = re.search(r"\{.*\}", preview, re.DOTALL)
    if not match:
        return False
    try:
        answer = json.loads(match.group())
    except json.JSONDecodeError:
        return False
    mapping = {"project": "Project", "owner": "Owner", "due_date": "Due date", "status": "Status"}
    if set(answer) != set(mapping) | {"evidence"} or not isinstance(answer["evidence"], dict):
        return False
    if set(answer["evidence"]) != set(mapping):
        return False
    for field, label in mapping.items():
        lines = [line for line in case["memo"].splitlines() if line.startswith(label + ": ")]
        expected_line = lines[0] if lines else None
        expected_value = expected_line.partition(": ")[2] if expected_line else None
        if answer[field] != expected_value or answer["evidence"][field] != expected_line:
            return False
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apr", required=True, type=Path)
    parser.add_argument("--state", required=True, type=Path)
    parser.add_argument("--attachment", required=True, type=Path)
    parser.add_argument("--project-root", required=True, type=Path)
    parser.add_argument("--journal", required=True, type=Path)
    parser.add_argument("--library-id", required=True,
                        help="Previously approved current OpenRouter library entry")
    parser.add_argument("--library-revision", required=True, type=int)
    args = parser.parse_args()
    journal = args.journal.resolve()
    index = json.loads((args.project_root / "e03-amended-index.json").read_text())
    if journal.exists():
        raise SystemExit("Journal exists: inspect prior attempted cells; refusing replay")
    items = index["dispatches"]
    expected_cells = {(case['id'], rep) for case in PROTOCOL['cases'] for rep in (1, 2)} - {('s01', 1)}
    if len(items) != 19 or {(i['case_id'], i['repetition']) for i in items} != expected_cells:
        raise SystemExit("Amended cell list does not preserve attempted cell")
    manifest_path = args.project_root / 'e03-openrouter-current-process.json'
    if hashlib.sha256(manifest_path.read_bytes()).hexdigest() != index['manifest_sha256']:
        raise SystemExit('Frozen amended manifest changed')
    if 2 + len(items) > 40 or RESERVED_PER_CALL_USD * 20 >= 1.0:
        raise SystemExit("Budget reservation invalid")
    # Claim the evidence journal before any request can create a run. A timeout
    # must leave a durable stop marker even when no run ID comes back.
    with journal.open('x') as stream:
        stream.write(json.dumps({'status':'invocation_started',
                                 'amendment_sha256':index['amendment_sha256']})+'\n')
        stream.flush()
        os.fsync(stream.fileno())
    with AprMcp(apr=args.apr, state=args.state, attachment=args.attachment) as client:
        bound = client.call("apr_manage", {"action": "connection.use",
                                           "library_id": args.library_id,
                                           "revision": args.library_revision,
                                           "name": "eval_openrouter_current"})
        bound_data = bound.get("structuredContent", {})
        if bound.get("isError") or bound_data.get("error"):
            raise SystemExit(f"Binding unavailable: {bound_data.get('error', 'unknown')}")
        for action in ("process.validate", "process.register"):
            result = client.call("apr_manage", {"action": action, "path": "e03-openrouter-current-process.json"})
            data = result.get("structuredContent", {})
            if data.get("error") or not data.get("process_ref"):
                raise SystemExit(f"{action} unavailable: {data.get('error', 'unknown')}")
            process_ref = data["process_ref"]
        readiness = client.call("apr_inspect", {"view": "readiness", "process_ref": process_ref})
        if readiness.get("structuredContent", {}).get("blockers"):
            raise SystemExit("Readiness has blockers")
        first = items[0]
        first_input = {"payload_file": {"path": first["relative_payload_path"],
                                        "sha256": first["payload_sha256"]},
                       "request_key": first["request_key"]}
        check = client.call("apr_run", {"process_ref": process_ref, "input": first_input,
                                        "request_key": first["request_key"] + "-preflight",
                                        "mode": "preflight"})
        if not check.get("structuredContent", {}).get("valid"):
            raise SystemExit("Preflight failed")

        for item in items:
            key = item["request_key"]
            args = {"process_ref": process_ref,
                    "input": {"payload_file": {"path": item["relative_payload_path"],
                                               "sha256": item["payload_sha256"]},
                              "request_key": key},
                    "request_key": key, "mode": "start"}
            started = time.monotonic()
            save(journal, {'provider':'openrouter', 'case_id':item['case_id'],
                           'repetition':item['repetition'], 'request_key':key,
                           'status':'start_requested'})
            result = client.call("apr_run", args)
            data = result.get("structuredContent", {})
            run_id = data.get("run_id")
            row = {"provider": "openrouter", "model_requested": MODEL,
                   "case_id": item["case_id"], "repetition": item["repetition"],
                   "request_key": key, "run_id": run_id, "status": "started" if run_id else "start_uncertain",
                   "amendment_sha256": index["amendment_sha256"],
                   "payload_sha256": item["payload_sha256"]}
            save(journal, row)
            if not run_id:
                raise SystemExit("Start had no run ID; stop before another cell")
            # One continuation request may dispatch the one intended provider step.
            try:
                continued = client.call("apr_resume", {"action": "continue", "run_id": run_id}, timeout=55)
            except Exception as exc:
                save(journal, {**row, "status": "continuation_uncertain", "error_class": type(exc).__name__})
                raise SystemExit("Continuation uncertain; stop before another cell")
            if continued.get("isError") or continued.get("structuredContent", {}).get("error"):
                save(journal, {**row, "status": "continuation_uncertain", "error_class": "service_error"})
                raise SystemExit("Continuation service error; stop before another cell")
            deadline = time.monotonic() + 55
            while True:
                view = client.call("apr_inspect", {"view": "run", "run_id": run_id, "detail": True})
                run = view.get("structuredContent", {})
                state = run.get("state")
                if state not in ("queued", "running") or time.monotonic() > deadline:
                    break
                time.sleep(1)
            if state != "succeeded":
                save(journal, {**row, "status": state or "unknown", "elapsed_s": round(time.monotonic() - started, 3)})
                raise SystemExit("Non-success or uncertain run; stop before another cell")
            result = run.get("output", {}).get("result", {})
            body = result.get("structuredContent", {})
            summary = body.get("data", {})
            usage = summary.get("usage")
            accepted = body.get("status") == "completed" and not summary.get("truncated") and grade(item["case_id"], summary.get("text_preview", ""))
            save(journal, {**row, "status": "completed", "elapsed_s": round(time.monotonic() - started, 3),
                  "provider_status": body.get("status"), "accepted": accepted,
                  "usage": usage if isinstance(usage, dict) else None,
                  "reported_cost": summary.get("reported_cost"),
                  "model_reported": None})
            if body.get("status") != "completed":
                raise SystemExit("Provider did not complete; stop before another cell")
            print(json.dumps({"case_id": item["case_id"], "repetition": item["repetition"],
                              "accepted": accepted, "elapsed_s": round(time.monotonic() - started, 3)}), flush=True)


if __name__ == "__main__":
    main()
