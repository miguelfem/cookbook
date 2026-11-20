import csv
import hashlib
import json
import sys
from pathlib import Path


HEADER = ["id", "queue", "priority", "opened_min", "first_response_min"]
LIMITS = {"P1": 30, "P2": 120, "P3": 480}


def integer(value):
    if not value or not value.lstrip("-").isascii() or not value.lstrip("-").isdecimal():
        raise ValueError("minutes must be integers")
    return int(value)


def process(path):
    with open(path, encoding="utf-8", newline="") as source:
        reader = csv.reader(source, strict=True)
        if next(reader, None) != HEADER:
            raise ValueError("bad header")
        rows = []
        seen = set()
        for record in reader:
            if len(rows) >= 300 or len(record) != 5 or not record[0] or not record[1] or record[0] in seen or record[2] not in LIMITS:
                raise ValueError("malformed row, duplicate ID, or row limit exceeded")
            seen.add(record[0])
            opened = integer(record[3])
            elapsed = None if record[4] == "" else integer(record[4]) - opened
            if elapsed is not None and elapsed < 0:
                raise ValueError("negative elapsed minutes")
            limit = LIMITS[record[2]]
            status = "unanswered" if elapsed is None else "breach" if elapsed > limit else "within"
            rows.append({"id": record[0], "queue": record[1], "priority": record[2],
                         "elapsed_minutes": elapsed, "limit_minutes": limit, "status": status})
    summary = {"checked": len(rows), "breached": sum(row["status"] == "breach" for row in rows),
               "unanswered": sum(row["status"] == "unanswered" for row in rows)}
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
        raise SystemExit("usage: sla.py [input_csv new_report_path]")
