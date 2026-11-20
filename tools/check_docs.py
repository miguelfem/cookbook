#!/usr/bin/env python3
"""Check this design package. No runtime behavior is executed or validated."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
errors: list[str] = []
link_count = 0


def require(condition: bool, message: str) -> None:
    if not condition:
        errors.append(message)


def unique_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON key: {key}")
        result[key] = value
    return result


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(), object_pairs_hook=unique_object)


def local_link(source: Path, target: str) -> None:
    global link_count
    target = target.strip().strip("<>")
    parsed = urlsplit(target)
    if parsed.scheme or parsed.netloc or not parsed.path:
        return
    resolved = (source.parent / unquote(parsed.path)).resolve()
    require(resolved.is_relative_to(ROOT), f"{source.relative_to(ROOT)}: link escapes repo: {target}")
    require(resolved.exists(), f"{source.relative_to(ROOT)}: missing local link: {target}")
    link_count += 1


class HtmlLinks(HTMLParser):
    def __init__(self, path: Path) -> None:
        super().__init__()
        self.path = path
        self.ids: set[str] = set()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        if values.get("id"):
            require(values["id"] not in self.ids, f"Duplicate HTML id: {values['id']}")
            self.ids.add(values["id"])
        for attr in ("href", "src"):
            if values.get(attr):
                local_link(self.path, values[attr])


def walk_bindings(value: object):
    if isinstance(value, dict):
        if "step" in value and "pointer" in value:
            yield value["step"]
        for item in value.values():
            yield from walk_bindings(item)
    elif isinstance(value, list):
        for item in value:
            yield from walk_bindings(item)


def check_semantics(manifest: dict, waiting: dict, submission: dict, receipt: dict) -> None:
    steps = manifest["steps"]
    ids = [step["id"] for step in steps]
    require(len(ids) == len(set(ids)), "Duplicate process step IDs")
    by_id = {step["id"]: step for step in steps}
    visiting: set[str] = set()
    ancestor_cache: dict[str, set[str]] = {}

    def visit(step_id: str) -> set[str]:
        if step_id in ancestor_cache:
            return ancestor_cache[step_id]
        if step_id in visiting:
            errors.append(f"Dependency cycle at {step_id}")
            return set()
        if step_id not in by_id:
            errors.append(f"Unknown dependency {step_id}")
            return set()
        visiting.add(step_id)
        ancestors = set(by_id[step_id]["needs"])
        for dependency in list(ancestors):
            ancestors.update(visit(dependency))
        visiting.remove(step_id)
        ancestor_cache[step_id] = ancestors
        return ancestors

    for step in steps:
        ancestors = visit(step["id"])
        for reference in walk_bindings(step.get("inputs", {})):
            require(reference in ancestors, f"{step['id']}: binding is not an ancestor: {reference}")
        expansion = step.get("foreach")
        if expansion:
            for reference in walk_bindings(expansion["items"]):
                require(reference in ancestors, f"{step['id']}: invalid expansion source")
            require(1 <= expansion["concurrency"] <= expansion["max_items"], "Invalid expansion bounds")
        if step["effect"] in {"write", "agent"}:
            require(step["retry"]["max_attempts"] == 1, "Fixture must not auto-retry uncertain/agent effects")
    for reference in walk_bindings(manifest["output"]):
        require(reference in by_id, f"Unknown final output reference: {reference}")

    request = waiting["request"]
    definition_id = request["step_instance_id"].split(":", 1)[0]
    definition = by_id.get(definition_id)
    require(definition is not None, "Handoff step is absent from the example process")
    if definition is not None:
        require(definition["kind"] == "agent", "Handoff must refer to an agent step")
        require(request["capability"] == definition.get("capability"), "Handoff capability differs from its process")
        require(request["result_schema"] == definition.get("result_schema"), "Handoff result schema differs from its process")
    require(set(waiting["completed_steps"]).issubset(by_id), "Completed step is absent from the example process")
    for field in ("run_id",):
        require(waiting[field] == submission[field] == receipt[field], f"Mismatched {field}")
    for field in ("request_id", "step_instance_id", "generation", "continuation_token"):
        require(request[field] == submission[field], f"Mismatched handoff {field}")
    require(receipt["request_id"] == request["request_id"], "Receipt request mismatch")
    require(receipt["submission_key"] == submission["submission_key"], "Receipt submission mismatch")
    require(receipt["revision"] > waiting["revision"], "Receipt revision did not advance")
    require(submission["continuation_token"].startswith("DEMO_ONLY"), "Fixture contains a non-demo token")
    states = read_json(ROOT / "schemas/run-envelope.schema.json")["properties"]["state"]["enum"]
    for payload in (waiting, receipt):
        require(payload["state"] in states, "Unknown fixture run state")


def full_schema_check(manifest: dict, waiting: dict, submission: dict, receipt: dict) -> None:
    try:
        from jsonschema import Draft202012Validator
    except ImportError:
        errors.append("--full-schema requires jsonschema; install requirements-check.txt in a local venv")
        return
    for path in (ROOT / "schemas").glob("*.json"):
        Draft202012Validator.check_schema(read_json(path))
    for path in sorted((ROOT / "examples").rglob("process.json")):
        process = read_json(path)
        version = process.get("schema_version")
        require(version in {"0.1", "0.2"}, f"{path.relative_to(ROOT)}: unknown manifest version")
        schema = "process-v0.2.schema.json" if version == "0.2" else "process.schema.json"
        Draft202012Validator(read_json(ROOT / "schemas" / schema)).validate(process)
        for name in ("input_schema", "output_schema"):
            Draft202012Validator.check_schema(process[name])
        input_path = path.with_name("input.json")
        if input_path.exists():
            Draft202012Validator(process["input_schema"]).validate(read_json(input_path))
        for step in process["steps"]:
            if "result_schema" in step:
                Draft202012Validator.check_schema(step["result_schema"])
        for source in process.get("files", []):
            source_path = (path.parent / source["path"]).resolve()
            require(source_path.is_relative_to(path.parent.resolve()), f"{path.relative_to(ROOT)}: source escapes example")
            if source_path.is_relative_to(path.parent.resolve()):
                require(hashlib.sha256(source_path.read_bytes()).hexdigest() == source["sha256"], f"{path.relative_to(ROOT)}: source digest differs")
    envelope = Draft202012Validator(read_json(ROOT / "schemas/run-envelope.schema.json"))
    envelope.validate(waiting)
    envelope.validate(receipt)
    Draft202012Validator(waiting["request"]["result_schema"]).validate(submission["output"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--full-schema", action="store_true", help="also validate with jsonschema draft 2020-12")
    args = parser.parse_args()
    markdown = list(ROOT.rglob("*.md"))
    markdown = [p for p in markdown if not any(part in {".git", ".venv", "node_modules", "work", "target"} for part in p.relative_to(ROOT).parts)]
    for path in markdown:
        text = re.sub(r"^```.*?^```\s*$", "", path.read_text(), flags=re.M | re.S)
        for target in re.findall(r"\[[^\]]*\]\(([^)]+)\)", text):
            local_link(path, target)
    html_path = ROOT / "design/inspector.html"
    HtmlLinks(html_path).feed(html_path.read_text())
    json_paths = list((ROOT / "schemas").glob("*.json")) + list((ROOT / "examples").rglob("*.json"))
    json_paths.append(ROOT / "integrations/claude/mcp.json.template")
    for path in json_paths:
        read_json(path)
    manifest = read_json(ROOT / "examples/local-review/process.json")
    waiting = read_json(ROOT / "examples/contracts/waiting-for-agent.json")
    submission = read_json(ROOT / "examples/contracts/submit-result.json")
    receipt = read_json(ROOT / "examples/contracts/resume-receipt.json")
    check_semantics(manifest, waiting, submission, receipt)
    if args.full_schema:
        full_schema_check(manifest, waiting, submission, receipt)
    if sys.version_info >= (3, 11):
        import tomllib
        tomllib.loads((ROOT / "integrations/codex/config.toml.template").read_text())
    if errors:
        print("\n".join("FAIL: " + error for error in errors), file=sys.stderr)
        return 1
    print(f"PASS: {len(markdown)} Markdown files, {link_count} local links, HTML links/IDs, {len(json_paths)} JSON files, fixture identities and DAG.")
    print("PASS: draft 2020-12 schemas and fixture values." if args.full_schema else "Full JSON Schema validation skipped; use --full-schema with the optional dependency.")
    print("No runtime execution, security enforcement, client integration, or savings were tested.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, KeyError, OSError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
