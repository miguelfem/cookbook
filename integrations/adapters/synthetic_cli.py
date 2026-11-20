"""Account-free JSON-line fixture for an owner-reviewed fixed CLI adapter."""
import json
import pathlib
import sys

if len(sys.argv) != 2:
    raise SystemExit("expected one fixed call-log path")
value = json.load(sys.stdin)
pathlib.Path(sys.argv[1]).write_text("called", encoding="utf8")
print(json.dumps({"text": value["text"].strip().lower()}))
