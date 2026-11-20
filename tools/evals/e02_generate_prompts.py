#!/usr/bin/env python3
"""Generate the nine fixed, private E02 measured prompts after authoring freeze."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "work/evals-2026-09-22/e02"
PROTOCOL = ROOT / "docs/evaluations/fixtures/e02/protocol.json"
AUTHORED = ROOT / "work/evals-2026-09-22/e02-authoring-freeze.json"
assert AUTHORED.exists(), "authoring must be frozen first"
protocol = json.loads(PROTOCOL.read_text())
assert protocol["ordering"] == [
    ["direct_agent", "reusable_script", "cookbook"],
    ["reusable_script", "cookbook", "direct_agent"],
    ["cookbook", "direct_agent", "reusable_script"],
]
ids = ["held-cedar", "held-willow", "held-maple"]
prompts = WORK / "prompts"
prompts.mkdir(parents=True, exist_ok=True)
common = "Read TASK.md. Work only on the supplied record ID. Do not inspect sources.json, the oracle, or other record IDs. Return only the complete, exact JSON object with no Markdown. "
for index, (record_id, ordered) in enumerate(zip(ids, protocol["ordering"], strict=True), 1):
    for condition in ordered:
        if condition == "direct_agent":
            body = ("Use the two synthetic source commands in TASK.md directly to complete this task. "
                    "You may use ordinary scripting if helpful. ")
        elif condition == "reusable_script":
            body = ("Use the frozen wrapper.py in this project. Do not modify the wrapper. "
                    "Run it for this ID and return its result. ")
        else:
            body = ("Use the connected CookBook MCP server. Connect this project if needed, "
                    "run the already registered Recipe sha256:6579352a14ba0e3e11fa6e80b4892041d5b8957f775d6f7baa2a4d3552d40ce7 "
                    "with the supplied ID, then inspect the finished run and return its output. "
                    "Do not modify the manifest/worker or use the source commands directly. "
                    "If the Recipe fails, report failure without a replacement approach. ")
        (prompts / f"case-{index}-{condition}.txt").write_text(
            common + body + f"Record ID: {record_id}.\n")
manifest = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(prompts.glob("case-*.txt"))}
(WORK / "prompt-freeze.json").write_text(json.dumps({"prompts": manifest,
    "authoring_freeze_sha256": hashlib.sha256(AUTHORED.read_bytes()).hexdigest(),
    "protocol_sha256": hashlib.sha256(PROTOCOL.read_bytes()).hexdigest()}, indent=2) + "\n")
print(json.dumps({"prompts": len(manifest),
                  "prompt_freeze_sha256": hashlib.sha256((WORK / "prompt-freeze.json").read_bytes()).hexdigest()}))
