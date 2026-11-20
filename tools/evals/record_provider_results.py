"""Write sanitized case-level E03/E04 records from the frozen synthetic protocol."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "work/evals-2026-09-22/providers"
PUBLIC = ROOT / "docs/evaluations/fixtures/providers/case-metrics.jsonl"
E03_PATH = ROOT / "docs/evaluations/fixtures/e03/protocol.json"
E03_AMENDMENT_PATH = ROOT / "docs/evaluations/fixtures/e03/amendment-1.json"
E04_PATH = ROOT / "docs/evaluations/fixtures/e04/protocol.json"
LEARNING_PATH = ROOT / "docs/evaluations/fixtures/e04/learning-amendment-1.json"
e03 = json.loads(E03_PATH.read_text())
e04 = json.loads(E04_PATH.read_text())
learning = json.loads(LEARNING_PATH.read_text())
e03_hash = hashlib.sha256(E03_PATH.read_bytes()).hexdigest()
e03_amendment_hash = hashlib.sha256(E03_AMENDMENT_PATH.read_bytes()).hexdigest()
e04_hash = hashlib.sha256(E04_PATH.read_bytes()).hexdigest()
learning_hash = hashlib.sha256(LEARNING_PATH.read_bytes()).hexdigest()


def rule(state: str) -> str:
    lower = state.lower()
    document = "supplied document" in lower
    records = "supplied record set" in lower
    if document and not records:
        return "documents"
    if records and not document:
        return "records"
    return "review"


rows: list[dict] = []
for provider in ("gemini", "openrouter"):
    for case in e03["cases"]:
        for rep in (1, 2):
            uncertain = case["id"] == "s01" and rep == 1
            rows.append({
                "study": "E03", "phase": "model_choice", "protocol_sha256": e03_hash,
                "amendment_sha256": (e03_amendment_hash if provider == "openrouter" and not uncertain else None),
                "condition": provider, "model_requested": e03["providers"][provider]["model"],
                "case_id": case["id"], "repetition": rep,
                "status": "needs_reconciliation" if uncertain else "not_started_after_stop",
                "provider_dispatch_count": 1 if uncertain else 0,
                "model_call_count": None if uncertain else 0,
                "output_accepted": None, "latency_s": None,
                "input_tokens": None, "output_tokens": None,
                "reported_cost_usd": None, "estimated_cost_usd": None,
                "model_reported": None,
                "run_id": ({"gemini": "run_3e335fbdfd5849b7a8ebf171990172e7",
                            "openrouter": "run_4ba20a3178e345a0992100c665f16b49"}[provider] if uncertain else None),
            })

caller_text = (OUT / "e04-caller-final.txt").read_text()
caller = json.loads(caller_text)["routes"]
caller_by_id = {r["id"]: r["route"] for r in caller}
if len(caller) != 24 or len(caller_by_id) != 24:
    raise ValueError("incomplete caller result")
raw = [json.loads(line) for line in (OUT / "e04-caller-raw.jsonl").read_text().splitlines() if line.strip()]
usage = [event["usage"] for event in raw if event.get("type") == "turn.completed"][-1]
if any("tool" in json.dumps(event).lower() for event in raw):
    raise ValueError("caller used a tool")
for case in e04["routing_cases"]:
    expected = case["label"]
    selected = rule(case["state"])
    mandatory = expected in e04["routing_policy"]["mandatory_review_labels"]
    rows.append({"study": "E04", "phase": "routing", "protocol_sha256": e04_hash,
                 "condition": "exact_rules", "case_id": case["id"], "expected": expected,
                 "prediction": selected, "status": "completed", "route_correct": selected == expected,
                 "mandatory_review": mandatory, "review_requested": selected == "review",
                 "policy_safe": (selected == "review") if mandatory else selected == expected,
                 "latency_s": None, "input_tokens": 0, "output_tokens": 0,
                 "reported_cost_usd": None, "model_call_count": 0,
                 "model_requested": None, "model_reported": None})
    selected = caller_by_id[case["id"]]
    rows.append({"study": "E04", "phase": "routing", "protocol_sha256": e04_hash,
                 "condition": "actual_codex_caller_batch", "case_id": case["id"], "expected": expected,
                 "prediction": selected, "status": "completed", "route_correct": selected == expected,
                 "mandatory_review": mandatory, "review_requested": selected in ("unclear", "other"),
                 "policy_safe": selected == expected, "latency_s": None,
                 "batch_id": "codex-caller-24", "batch_elapsed_s": 8.911,
                 "batch_input_tokens": usage["input_tokens"],
                 "batch_cached_input_tokens": usage["cached_input_tokens"],
                 "batch_output_tokens": usage["output_tokens"],
                 "batch_reasoning_output_tokens": usage["reasoning_output_tokens"],
                 "input_tokens": None, "output_tokens": None,
                 "reported_cost_usd": None, "model_call_count": None,
                 "batch_model_call_count": 1,
                 "model_requested": "gpt-5.6-luna",
                 "model_reported": None})
    rows.append({"study": "E04", "phase": "routing", "protocol_sha256": e04_hash,
                 "condition": "jev", "case_id": case["id"], "expected": expected,
                 "prediction": None, "status": "not_started_after_uncertain_smoke",
                 "route_correct": None, "mandatory_review": mandatory,
                 "review_requested": None, "policy_safe": None, "latency_s": None,
                 "input_tokens": None, "output_tokens": None,
                 "reported_cost_usd": None, "model_call_count": 0,
                 "model_requested": e04["model"],
                 "model_reported": None})

rows.append({"study": "E04", "phase": "jev_smoke", "protocol_sha256": e04_hash,
             "condition": "jev", "case_id": "smoke_clear_01",
             "status": "dispatched_unresolved", "provider_dispatch_count": 1,
             "model_call_count": None,
             "run_id": "run_fe3a6a5910d74b1ca2844d456f32477d",
             "prediction": None, "latency_s": None, "input_tokens": None,
             "output_tokens": None, "reported_cost_usd": None,
             "model_requested": e04["model"], "model_reported": None})
amended_heldout = [case for case in e04["heldout"] if case["id"] != "h15"]
amended_heldout.extend(learning["new_heldout"])
if len(amended_heldout) != 17 or len({case["id"] for case in amended_heldout}) != 17:
    raise ValueError("learning amendment does not have 17 distinct heldout cases")
for case in amended_heldout:
    for arm in ("baseline", "candidate"):
        rows.append({"study": "E04", "phase": "other_learning", "protocol_sha256": e04_hash,
                     "amendment_sha256": learning_hash,
                     "condition": arm, "case_id": case["id"], "expected": case["expected"],
                     "prediction": None, "status": "not_started_after_uncertain_smoke",
                     "provider_dispatch_count": 0, "model_call_count": 0,
                     "input_tokens": None,
                     "output_tokens": None, "reported_cost_usd": None,
                     "model_requested": e04["model"], "model_reported": None})

path = OUT / "case_results.jsonl"
with path.open("w") as stream:
    for row in rows:
        stream.write(json.dumps(row, sort_keys=True) + "\n")
PUBLIC.parent.mkdir(parents=True, exist_ok=True)
public_fields = {
    "study", "phase", "protocol_sha256", "amendment_sha256", "condition",
    "model_requested", "model_reported", "case_id", "repetition", "status",
    "provider_dispatch_count", "model_call_count", "batch_model_call_count",
    "output_accepted", "route_correct", "mandatory_review", "review_requested",
    "policy_safe", "latency_s", "batch_id", "batch_elapsed_s",
    "input_tokens", "output_tokens", "batch_input_tokens", "batch_cached_input_tokens",
    "batch_output_tokens", "batch_reasoning_output_tokens", "reported_cost_usd",
    "estimated_cost_usd",
}
with PUBLIC.open("w") as stream:
    for row in rows:
        metric = {key: value for key, value in row.items() if key in public_fields}
        stream.write(json.dumps(metric, sort_keys=True) + "\n")
print(json.dumps({"path": str(path), "rows": len(rows),
                  "public_path": str(PUBLIC),
                  "e03_dispatched": sum(x.get("provider_dispatch_count", 0) for x in rows if x["study"] == "E03"),
                  "jev_dispatched": sum(x.get("provider_dispatch_count", 0) for x in rows if x.get("condition") == "jev"),
                  "caller_correct": sum(x.get("route_correct") is True for x in rows if x.get("condition") == "actual_codex_caller_batch")}))
