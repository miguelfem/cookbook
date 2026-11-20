"""Freeze a one-call Jev routing recipe and a caller-escalation smoke recipe."""

from __future__ import annotations

import hashlib
import json
import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PROTOCOL = ROOT / "docs/evaluations/fixtures/e04/protocol.json"
DATA = json.loads(PROTOCOL.read_text())


def write(path: Path, value: dict) -> str:
    raw = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    path.write_bytes(raw)
    return hashlib.sha256(raw).hexdigest()


def base() -> dict:
    return {
        "schema_version": "0.2", "name": "synthetic.e04-routing",
        "description": "Classify synthetic workflow requests through one protected Jev call.",
        "input_schema": {"type": "object", "required": ["state"],
                         "additionalProperties": False,
                         "properties": {"state": {"type": "string", "maxLength": 8000}}},
        "output_schema": {"type": "object"},
        "config": {"schema": {"type": "object", "additionalProperties": False,
                              "required": ["model"], "properties": {"model": {"type": "string"}}},
                   "defaults": {"model": DATA["model"]}, "run_override_keys": []},
        "capabilities": [{"kind": "connection", "name": "eval_jev"}],
        "limits": {"max_step_ms": 30000, "max_artifact_bytes": 1048576,
                   "max_provider_calls": 1, "max_agent_requests": 0},
        "steps": [{"id": "observe", "kind": "connector", "operation": "jev.evaluate",
                   "connection": "eval_jev", "needs": [], "effect": "read",
                   "retry": {"max_attempts": 1},
                   "inputs": {"state": {"input": "/state"},
                              "model": {"config": "/model"},
                              "questions": {"literal": DATA["routing_question"]}},
                   "result_schema": {"type": "object",
                                     "required": ["provider", "requested_model", "reported_model", "answers", "usage"]}}],
        "output": {"observation": {"step": "observe", "pointer": ""}},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project-root", required=True, type=Path,
                        help="Explicit disposable synthetic evaluation directory")
    project = parser.parse_args().project_root.resolve()
    project.mkdir(parents=True, exist_ok=True)
    observation = base()
    obs_sha = write(project / "e04-observe-process.json", observation)
    smoke = base()
    smoke["name"] = "synthetic.e04-routing-smoke"
    smoke["description"] = "Observe, apply a fixed route policy, and request caller review for unsupported or uncertain work."
    # Admission counts both mutually exclusive review branches.
    smoke["limits"]["max_agent_requests"] = 2
    smoke["limits"]["max_step_instances"] = 8
    smoke["capabilities"].extend([{"kind": "builtin"}, {"kind": "agent", "capability": "review"}])
    smoke["steps"].append({
        "id": "route", "kind": "decision", "effect": "pure", "needs": ["observe"],
        "inputs": {}, "retry": {"max_attempts": 1},
        "decision": {"source": "choice", "routes": ["documents", "records", "unclear", "other"],
                     "value": {"step": "observe", "pointer": "/answers/route/choice"},
                     "confidence": {"step": "observe", "pointer": "/answers/route/confidence"},
                     "min_confidence": DATA["routing_policy"]["min_confidence"],
                     "no_match": "unclear", "needs_clarification": "unclear"},
    })
    for route in ("documents", "records"):
        smoke["steps"].append({
            "id": route, "kind": "builtin", "operation": "json.select", "effect": "pure",
            "needs": ["route"], "when": {"decision": "route", "route": route},
            "retry": {"max_attempts": 1},
            "inputs": {"value": {"literal": {"route": route}}, "pointer": {"literal": ""}},
        })
    for route in ("unclear", "other"):
        smoke["steps"].append({
            "id": route, "kind": "agent", "effect": "agent", "needs": ["route"],
            "when": {"decision": "route", "route": route}, "retry": {"max_attempts": 1},
            "capability": "review",
            "instructions": "Review this synthetic request and observation. Return documents, records, unclear, or other. Do not take an external action.",
            "inputs": {"state": {"input": "/state"}, "observation": {"step": "observe", "pointer": ""}},
            "result_schema": {"type": "object", "required": ["route"],
                              "additionalProperties": False,
                              "properties": {"route": {"enum": ["documents", "records", "unclear", "other"]}}},
        })
    smoke["steps"].append({
        "id": "join", "kind": "join", "effect": "pure",
        "needs": ["route", "documents", "records", "unclear", "other"],
        "inputs": {}, "retry": {"max_attempts": 1},
        "join": {"decision": "route", "branches": {
            "documents": {"step": "documents", "pointer": "/value"},
            "records": {"step": "records", "pointer": "/value"},
            "unclear": {"step": "unclear", "pointer": ""},
            "other": {"step": "other", "pointer": ""},
        }},
    })
    smoke["output"] = {"observation": {"step": "observe", "pointer": ""},
                       "route": {"step": "route", "pointer": ""},
                       "result": {"step": "join", "pointer": "/output"}}
    smoke_sha = write(project / "e04-smoke-process.json", smoke)
    print(json.dumps({"protocol_sha256": hashlib.sha256(PROTOCOL.read_bytes()).hexdigest(),
                      "observe_manifest_sha256": obs_sha, "smoke_manifest_sha256": smoke_sha}))


if __name__ == "__main__":
    main()
