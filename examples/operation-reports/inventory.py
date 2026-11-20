import csv
import hashlib
import json
import sys
from pathlib import Path


HEADER = ["id", "warehouse", "expected", "counted"]


def process(path):
    with open(path, encoding="utf-8", newline="") as source:
        reader = csv.reader(source, strict=True)
        if next(reader, None) != HEADER:
            raise ValueError("bad header")
        rows = []
        seen = set()
        for record in reader:
            if len(rows) >= 300 or len(record) != 4 or not record[0] or not record[1] or record[0] in seen:
                raise ValueError("malformed row, duplicate ID, or row limit exceeded")
            if any(not value.isascii() or not value.isdecimal() for value in record[2:]):
                raise ValueError("counts must be nonnegative integers")
            seen.add(record[0])
            expected, counted = map(int, record[2:])
            delta = counted - expected
            rows.append({"id": record[0], "warehouse": record[1], "expected": expected,
                         "counted": counted, "delta": delta,
                         "status": "match" if delta == 0 else "over" if delta > 0 else "short"})
    summary = {"checked": len(rows), "mismatched": sum(row["delta"] != 0 for row in rows),
               "net_delta": sum(row["delta"] for row in rows)}
    return rows, summary


def write_report(path, rows):
    data = json.dumps({"rows": rows}, sort_keys=True, separators=(",", ":")).encode("utf-8")
    with Path(path).open("xb") as output:
        output.write(data)
    return hashlib.sha256(data).hexdigest()


def worker():
    env = json.load(sys.stdin)
    if env["protocol_version"] != "0.1":
        raise ValueError("unsupported protocol")
    inputs = env["input"]
    rows, summary = process(env["artifacts"][inputs["source"]["artifact_id"]]["path"])
    write_report("report.json", rows)
    output = {"summary": summary, "report": {"worker_artifact": "full"},
              "rows": rows if inputs["include_rows"] is True else None}
    print(json.dumps({"protocol_version": "0.1", "output": output,
                      "artifacts": [{"name": "full", "path": "report.json", "media_type": "application/json"}]},
                     separators=(",", ":")))


if __name__ == "__main__":
    if len(sys.argv) == 1:
        worker()
    elif len(sys.argv) == 3:
        rows, summary = process(sys.argv[1])
        digest = write_report(sys.argv[2], rows)
        print(json.dumps({"summary": summary, "report_sha256": digest}, separators=(",", ":")))
    else:
        raise SystemExit("usage: inventory.py [input_csv new_report_path]")
