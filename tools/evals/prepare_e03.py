"""Freeze synthetic E03 inputs and the two connection-only recipe variants."""

from __future__ import annotations

import hashlib
import json
import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PROTOCOL = ROOT / "docs/evaluations/fixtures/e03/protocol.json"
DATA = json.loads(PROTOCOL.read_text())


def write(path: Path, value: dict) -> str:
    path.parent.mkdir(parents=True, exist_ok=True)
    raw = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    path.write_bytes(raw)
    return hashlib.sha256(raw).hexdigest()


def manifest(connection: str, model: str) -> dict:
    return {
        "schema_version": "0.2", "name": "synthetic.e03-grounded-extraction",
        "description": "One approved model API extracts labeled fields from a synthetic memo.",
        "input_schema": {"type": "object", "additionalProperties": False,
                         "required": ["payload_file", "request_key"],
                         "properties": {"payload_file": {"type": "object"}, "request_key": {"type": "string"}}},
        "output_schema": {"type": "object"},
        "config": {"schema": {"type": "object", "additionalProperties": False,
                              "required": ["model"], "properties": {"model": {"type": "string"}}},
                   "defaults": {"model": model}, "run_override_keys": []},
        "capabilities": [{"kind": "connection", "name": connection}],
        "limits": {"max_step_ms": 45000, "max_artifact_bytes": 1048576,
                   "max_provider_calls": 1, "max_agent_requests": 0},
        "steps": [{"id": "extract", "kind": "connector", "operation": "mcp.call",
                   "connection": connection, "needs": [], "effect": "read",
                   "retry": {"max_attempts": 1},
                   "inputs": {"action": {"literal": "generate" if connection == "eval_gemini" else "complete"},
                              "authorization_ref": {"literal": "Synthetic E03 inference authorized by owner on 2026-09-22; maximum 40 dispatches."},
                              "model": {"config": "/model"},
                              "payload_file": {"input": "/payload_file"},
                              "request_key": {"input": "/request_key"}},
                   "result_schema": {"type": "object", "required": ["structuredContent"]}}],
        "output": {"result": {"step": "extract", "pointer": ""}},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project-root", required=True, type=Path,
                        help="Explicit disposable synthetic evaluation directory")
    project = parser.parse_args().project_root.resolve()
    index = []
    for provider, settings in DATA["providers"].items():
        connection = settings["tool"]
        mpath = project / f"e03-{provider}-process.json"
        manifest_sha = write(mpath, manifest(connection, settings["model"]))
        for case in DATA["cases"]:
            for rep in (1, 2):
                content = DATA["prompt"] + "\n\nMemo:\n" + case["memo"]
                if provider == "gemini":
                    payload = {"contents": [{"role": "user", "parts": [{"text": content}]}],
                               "generationConfig": {"temperature": 0,
                                                    "maxOutputTokens": settings["max_output_tokens"],
                                                    "responseMimeType": "application/json"}}
                else:
                    payload = {"messages": [{"role": "user", "content": content}],
                               "temperature": 0, "max_tokens": settings["max_output_tokens"],
                               "response_format": {"type": "json_object"}}
                ppath = project / "e03-payloads" / f"{provider}-{case['id']}-r{rep}.json"
                payload_sha = write(ppath, payload)
                index.append({"provider": provider, "case_id": case["id"], "rep": rep,
                              "model": settings["model"], "manifest_path": str(mpath),
                              "manifest_sha256": manifest_sha, "payload_path": str(ppath),
                              "payload_sha256": payload_sha})
    write(project / "e03-index.json", {"protocol_sha256": hashlib.sha256(PROTOCOL.read_bytes()).hexdigest(),
                                       "dispatches": index})
    print(json.dumps({"protocol_sha256": hashlib.sha256(PROTOCOL.read_bytes()).hexdigest(),
                      "dispatch_count": len(index), "project": str(project)}))


if __name__ == "__main__":
    main()
