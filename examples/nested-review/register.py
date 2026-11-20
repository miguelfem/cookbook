"""Register a bounded synthetic nested composition using immutable references.

The transport callback is the same (action, arguments) API used by the inspector.
Generated manifests belong in the private attached project, never the checkout.
"""

from copy import deepcopy
import json


def register_nested(call, project, leaf_ref, depth=12):
    if isinstance(depth, bool) or not isinstance(depth, int) or not 1 <= depth <= 128:
        raise ValueError("depth must be an integer between 1 and 128")
    leaf = json.loads((project / "local-review/process.json").read_text())
    directory = project / "nested-review"
    directory.mkdir(mode=0o700, exist_ok=True)
    target = leaf_ref
    references = {"leaf": leaf_ref, "depth": depth}
    for level in range(1, depth + 1):
        filename = "process" if level == depth else f"layer-{level:02}"
        name = "nested.document-delivery" if level == depth else f"nested.review-layer-{level:02}"
        step_id = "review_document" if level == 1 else "prepare_review"
        limits = deepcopy(leaf["limits"])
        limits.update({"max_call_depth": max(32, depth), "max_child_runs": 1024,
                       "max_step_instances": 10000})
        manifest = {
            "schema_version": "0.2",
            "name": name,
            "description": "Synthetic reusable document review with a pinned child process",
            "input_schema": deepcopy(leaf["input_schema"]),
            "output_schema": deepcopy(leaf["output_schema"]),
            "config": deepcopy(leaf["config"]),
            "capabilities": [],
            "limits": limits,
            "steps": [{
                "id": step_id, "kind": "call", "effect": "control", "needs": [],
                "retry": {"max_attempts": 1},
                "call": {"process_ref": target, "output_pointer": ""},
                "inputs": {"record": {"input": "/record"}, "sources": {"input": "/sources"}},
            }],
            "output": {
                "approved": {"step": step_id, "pointer": "/approved"},
                "package": {"step": step_id, "pointer": "/package"},
            },
        }
        path = directory / f"{filename}.json"
        path.write_text(json.dumps(manifest, indent=2) + "\n")
        registered = call("manage", {
            "action": "process.register", "path": str(path.relative_to(project)),
        })
        if level == depth:
            references["child"] = target
        target = registered["process_ref"]
    references["root"] = target
    return references
