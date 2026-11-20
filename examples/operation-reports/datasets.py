"""Deterministic synthetic sources and independent expected rows."""
import csv
import hashlib
import json
from pathlib import Path

TASKS = ("inventory", "sla")
SIZES = (72, 96, 144)

def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()

def fixture(task, count, target):
    rows, expected = [], []
    for i in range(count):
        record = f"{task[:3]}-{i+1:03d}"
        if task == "inventory":
            warehouse = ("north", "south", "west")[i % 3]
            planned = 40 + i % 17
            delta = {0: -3, 1: 2, 2: 0, 3: 0, 4: 0}[i % 5]
            status = ("short", "over", "match", "match", "match")[i % 5]
            rows.append((record, warehouse, planned, planned + delta))
            expected.append(dict(id=record, warehouse=warehouse, expected=planned,
                                 counted=planned + delta, delta=delta, status=status))
        else:
            queue = ("fulfillment", "billing", "support")[i % 3]
            priority, limit = (("P1", 30), ("P2", 120), ("P3", 480))[i % 3]
            pattern = i % 6
            age = {0: None, 1: limit+1, 2: limit, 3: 0, 4: limit+50, 5: 12}[pattern]
            status = ("unanswered", "breach", "within", "within", "breach", "within")[pattern]
            opened = i * 13
            rows.append((record, queue, priority, opened, "" if age is None else opened+age))
            expected.append(dict(id=record, queue=queue, priority=priority, elapsed_minutes=age,
                                 limit_minutes=limit, status=status))
    target.mkdir(parents=True, exist_ok=True)
    path = target / f"{task}-{count}.csv"
    with path.open("w", newline="") as fp:
        writer = csv.writer(fp, lineterminator="\n")
        if task == "inventory":
            writer.writerow(("id", "warehouse", "expected", "counted"))
        else:
            writer.writerow(("id", "queue", "priority", "opened_min", "first_response_min"))
        writer.writerows(rows)
    if task == "inventory":
        summary = dict(checked=count, mismatched=(count+4)//5+(count+3)//5,
                       net_delta=-3*((count+4)//5)+2*((count+3)//5))
    else:
        summary = dict(checked=count, breached=(count+4)//6+(count+1)//6, unanswered=(count+5)//6)
    report = canonical({"rows": expected})
    return path, summary, expected, hashlib.sha256(report).hexdigest()


if __name__ == "__main__":
    import sys
    for task in TASKS:
        for count in SIZES:
            _, summary, rows, sha = fixture(task, count, Path(sys.argv[1]))
            print(task, count, summary, len(canonical({"rows":rows})))
