#!/usr/bin/env python3
"""Synthetic local source endpoint; no network, no artificial delay."""
import json
import sys
from pathlib import Path

if len(sys.argv) != 3 or sys.argv[1] not in ("profile", "ledger"):
    raise SystemExit("usage: service.py profile|ledger RECORD_ID")
data = json.loads((Path(__file__).parent / "sources.json").read_text())
source, record_id = sys.argv[1:]
if record_id not in data:
    raise SystemExit("unknown record ID")
with (Path(__file__).parent / "source-audit.jsonl").open("a") as stream:
    stream.write(json.dumps({"source": source, "record_id": record_id}) + "\n")
print(json.dumps(data[record_id][source], ensure_ascii=False))
