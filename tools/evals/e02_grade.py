#!/usr/bin/env python3
"""Grade frozen E02 outputs and export sanitized case-level measurements.

Requires an explicit disposable fixture root, never a default user runtime.
"""
import argparse
import csv
import datetime as dt
import hashlib
import json
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "work/evals-2026-09-22/e02"
PROTOCOL = ROOT / "docs/evaluations/fixtures/e02/protocol.json"
PUBLIC = ROOT / "docs/evaluations/fixtures/e02"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def verify_freezes(fixture_root):
    lead = json.loads((ROOT / "work/evals-2026-09-22/e02-freeze.json").read_text())
    author = ROOT / "work/evals-2026-09-22/e02-authoring-freeze.json"
    authored = json.loads(author.read_text())
    prompts = json.loads((WORK / "prompt-freeze.json").read_text())
    source_key = next(name for name in lead["sha256"]
                      if name.endswith("/e02-shared/service.py"))
    original_fixture_root = Path(source_key).parent.parent
    protocol_key = next(name for name in lead["sha256"]
                        if name.endswith("/docs/evaluations/fixtures/e02/protocol.json"))
    original_repo_root = Path(protocol_key).parents[4]
    for freeze in (lead, authored):
        for name, checksum in freeze["sha256"].items():
            original = Path(name)
            try:
                relative = original.relative_to(original_fixture_root)
            except ValueError:
                try:
                    current = ROOT / original.relative_to(original_repo_root)
                except ValueError as error:
                    raise ValueError(f"frozen path is outside the declared roots: {original}") from error
            else:
                current = fixture_root / relative
            assert sha(current) == checksum, f"frozen material changed: {current}"
    assert prompts["authoring_freeze_sha256"] == sha(author)
    assert prompts["protocol_sha256"] == sha(PROTOCOL)
    for name, checksum in prompts["prompts"].items():
        assert sha(ROOT / name) == checksum, f"measured prompt changed: {name}"
    return lead, authored, prompts, original_fixture_root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture-root", required=True, type=Path,
                        help="Absolute disposable E02 fixture root containing e02-shared, e02-oracle, and e02-runtime")
    args = parser.parse_args()
    fixture_root = args.fixture_root.resolve(strict=True)
    lead, authored, prompt_freeze, original_fixture_root = verify_freezes(fixture_root)
    protocol = json.loads(PROTOCOL.read_text())
    oracle = json.loads((fixture_root / "e02-oracle/expected.json").read_text())
    accesses = [json.loads(line) for line in (fixture_root / "e02-shared/access.jsonl").read_text().splitlines()]
    ids = ("held-cedar", "held-willow", "held-maple")
    rows = []
    for index, (record_id, conditions) in enumerate(zip(ids, protocol["ordering"], strict=True), 1):
        for order, condition in enumerate(conditions, 1):
            evidence = WORK / f"measure-{index}-{condition}"
            summary = json.loads((evidence / "summary.json").read_text())
            final = (evidence / "final.txt").read_text().strip()
            try:
                output = json.loads(final)
                parse_error = None
            except json.JSONDecodeError:
                output = None
                parse_error = "final_not_json"
            accepted = output == oracle[record_id]
            start = dt.datetime.fromisoformat(summary["started_utc"])
            end = start + dt.timedelta(seconds=summary["elapsed_seconds"])
            seen = [entry for entry in accesses
                    if entry["record_id"] == record_id and
                    start <= dt.datetime.fromisoformat(entry["utc"]) <= end]
            source_counts = {name: sum(a["source"] == name for a in seen)
                             for name in ("profile", "ledger")}
            usage = summary.get("usage") or {}
            calls = summary.get("tool_calls") or []
            row = {"case_index": index, "order_in_case": order, "condition": condition,
                   "accepted": accepted, "failure_reason": parse_error or (None if accepted else "output_mismatch"),
                   "session_exit_code": summary["exit_code"], "stop_reason": summary["stop_reason"],
                   "elapsed_seconds": summary["elapsed_seconds"],
                   "input_tokens": usage.get("input_tokens"),
                   "cached_input_tokens": usage.get("cached_input_tokens"),
                   "output_tokens": usage.get("output_tokens"),
                   "reasoning_output_tokens": usage.get("reasoning_output_tokens"),
                   "tool_calls": len(calls),
                   "tool_result_event_bytes": sum(call["result_bytes"] for call in calls),
                   "event_bytes": summary["event_bytes"],
                   "profile_reads": source_counts["profile"],
                   "ledger_reads": source_counts["ledger"],
                   "source_response_bytes": sum(a["response_bytes"] for a in seen),
                   "client_billed_cost_usd": None,
                   "provider_cost_usd": None}
            if condition == "cookbook":
                db = fixture_root / "e02-runtime/state.sqlite3"
                with sqlite3.connect(f"file:{db}?mode=ro", uri=True) as con:
                    matching = []
                    for run_id, state, process_ref, raw_input, raw_output in con.execute(
                            "SELECT id,state,version,input,output FROM runs WHERE version=?",
                            (authored["cookbook_process_ref"],)):
                        if json.loads(raw_input).get("record_id") == record_id:
                            matching.append((run_id, state, json.loads(raw_output) if raw_output else None))
                row["recipe_run_count"] = len(matching)
                row["recipe_run_state"] = matching[0][1] if len(matching) == 1 else None
                row["recipe_output_exact"] = matching[0][2] == oracle[record_id] if len(matching) == 1 else None
            else:
                row["recipe_run_count"] = None
                row["recipe_run_state"] = None
                row["recipe_output_exact"] = None
            rows.append(row)
    assert len(rows) == 9
    public_fields = ("case_index", "order_in_case", "condition", "accepted", "failure_reason",
        "session_exit_code", "stop_reason", "elapsed_seconds", "input_tokens", "cached_input_tokens",
        "output_tokens", "reasoning_output_tokens", "tool_calls", "tool_result_event_bytes",
        "event_bytes", "profile_reads", "ledger_reads", "source_response_bytes", "recipe_run_count",
        "recipe_run_state", "recipe_output_exact", "client_billed_cost_usd", "provider_cost_usd")
    with (PUBLIC / "case-metrics.csv").open("w", newline="") as destination:
        writer = csv.DictWriter(destination, fieldnames=public_fields, lineterminator='\n')
        writer.writeheader()
        writer.writerows(rows)
    with (PUBLIC / "case-metrics.json").open("w") as destination:
        json.dump({"study": "E02", "quality_scope": "synthetic exact-match internal pilot",
                   "labels_exported": False, "protocol_sha256": sha(PROTOCOL),
                   "source_sha256": lead["sha256"][str(original_fixture_root / "e02-shared/sources.json")],
                   "oracle_sha256": lead["sha256"][str(original_fixture_root / "e02-oracle/expected.json")],
                   "authoring_freeze_sha256": sha(ROOT / "work/evals-2026-09-22/e02-authoring-freeze.json"),
                   "prompt_freeze_sha256": sha(WORK / "prompt-freeze.json"),
                   "cases": rows}, destination, indent=2)
        destination.write("\n")
    print(json.dumps({"attempts": len(rows), "accepted": sum(r["accepted"] for r in rows),
        "by_condition": {name: {"accepted": sum(r["accepted"] for r in rows if r["condition"] == name),
                                "attempted": sum(r["condition"] == name for r in rows)}
                         for name in protocol["conditions"]},
        "source_read_pairs": [(r["profile_reads"], r["ledger_reads"]) for r in rows]}))


if __name__ == "__main__":
    main()
