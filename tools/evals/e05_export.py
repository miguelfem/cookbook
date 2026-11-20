#!/usr/bin/env python3
"""Read-only export of E05 diagnostic case metrics; excludes expected/actual labels."""
import csv
import json
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "work/evals-2026-09-22/e05"
PUBLIC = ROOT / "docs/evaluations/fixtures/e05"
RAW = json.loads((WORK / "raw-result-private.json").read_text())
FIELDS = ("cycle", "split", "case_index", "arm", "run_state", "exact_spec_match",
          "required_fields_present", "completed_pure_steps", "acceptance_scope")
rows = []
for cycle in ("simpler", "cheaper_wrong", "missing_evidence"):
    db = WORK / "direct" / cycle / "state/state.sqlite3"
    with sqlite3.connect(f"file:{db}?mode=ro", uri=True) as con:
        runs = con.execute("SELECT id, request_key, input, output, state FROM runs").fetchall()
        for run_id, request_key, raw_input, raw_output, state in runs:
            request_key = json.loads(request_key)[1]
            arm, split, index = request_key.rsplit("-", 2)
            assert arm in ("baseline", cycle) and split in ("dev", "held")
            assert index in {str(i) for i in range(1, 7)}
            actual = json.loads(raw_output) if raw_output is not None else None
            expected_from_independent_rule = json.loads(raw_input)
            steps = con.execute("SELECT COUNT(*) FROM steps WHERE run_id=? AND state='succeeded'",
                                (run_id,)).fetchone()[0]
            rows.append({"cycle": cycle, "split": split, "case_index": index,
                         "arm": "baseline" if arm == "baseline" else "candidate",
                         "run_state": state,
                         "exact_spec_match": str(actual == expected_from_independent_rule).lower(),
                         "required_fields_present": str(isinstance(actual, dict) and
                             set(actual) == {"answer", "evidence"}).lower(),
                         "completed_pure_steps": steps,
                         "acceptance_scope": "diagnostic_only_nonhuman_labels"})
assert len(rows) == 72 and len({(r["cycle"], r["split"], r["case_index"], r["arm"])
                                   for r in rows}) == 72
rows.sort(key=lambda r: (r["cycle"], r["split"], int(r["case_index"]), r["arm"]))
with (PUBLIC / "diagnostic-case-metrics.csv").open("w", newline="") as target:
    writer = csv.DictWriter(target, fieldnames=FIELDS, lineterminator='\n')
    writer.writeheader()
    writer.writerows(rows)

with (PUBLIC / "blocked-cycle-status.csv").open("w", newline="") as target:
    writer = csv.DictWriter(target, fieldnames=("cycle", "state_after_attempt",
        "freeze_error_code", "evaluator_result", "adoption", "acceptance_scope"), lineterminator='\n')
    writer.writeheader()
    for cycle in ("simpler", "cheaper_wrong", "missing_evidence", "unsupported", "effect"):
        frozen = RAW["cycles"][cycle]
        assert frozen["freeze_rejected"]
        assert frozen["freeze_error"]["error"]["code"] == "INVALID_INPUT"
        writer.writerow({"cycle": cycle, "state_after_attempt": "proposed",
            "freeze_error_code": "INVALID_INPUT", "evaluator_result": "not_run",
            "adoption": "not_run", "acceptance_scope": "blocked_pending_human_review"})
    writer.writerow({"cycle": "no_label_closure", "state_after_attempt": "closed",
        "freeze_error_code": "not_applicable", "evaluator_result": "not_run",
        "adoption": "not_run", "acceptance_scope": "more_evidence_required"})
print("exported 72 non-acceptance diagnostic rows and six blocked/closed cycle rows")
