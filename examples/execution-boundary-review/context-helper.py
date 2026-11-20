"""Synthetic, dependency-free bounded page aggregation for a trusted APR worker."""
import hashlib
import json
import sys

# Leave room for APR to replace the artifact placeholder with its reference.
MAX_RESULT_BYTES = 1800
MAX_ITEMS = 5000
MAX_PAGES = 128


def emit(output, artifacts=None):
    envelope = {"protocol_version": "0.1", "output": output}
    if artifacts:
        envelope["artifacts"] = artifacts
    payload = json.dumps(envelope, separators=(",", ":"), sort_keys=True).encode()
    if len(payload) > MAX_RESULT_BYTES:
        raise ValueError("normal result exceeds declared byte budget")
    sys.stdout.buffer.write(payload)


request = json.load(sys.stdin)
data = request["input"]
count = data["item_count"]
page_size = data["page_size"]
guidance = data["guidance"]
missing_page = data.get("missing_page")
previous_guidance = data.get("previous_guidance", [])

if not (0 <= count <= MAX_ITEMS and 1 <= page_size <= 500):
    raise ValueError("dataset bounds")
page_count = (count + page_size - 1) // page_size
if page_count > MAX_PAGES:
    raise ValueError("page count")
if len({item["id"] for item in guidance}) != len(guidance):
    raise ValueError("duplicate guidance id")
if len({item["id"] for item in previous_guidance}) != len(previous_guidance):
    raise ValueError("duplicate previous guidance id")

current = {item["id"]: item["text"] for item in guidance}
previous = {item["id"]: item["text"] for item in previous_guidance}
revision_source = {
    "item_count": count,
    "page_size": page_size,
    "guidance": sorted(current.items()),
}
revision = hashlib.sha256(
    json.dumps(revision_source, separators=(",", ":"), sort_keys=True).encode()
).hexdigest()

# Completeness is checked before unchanged-context reuse.
if missing_page is not None and 0 <= missing_page < page_count:
    emit({"status": "incomplete", "missing_page": missing_page,
          "reason": "required source page unavailable"})
elif data.get("previous_revision") == revision:
    emit({"status": "unchanged", "revision": revision})
else:
    seen = 0
    digest = hashlib.sha256()
    with open("raw.jsonl", "wb") as raw:
        for page in range(page_count):
            start = page * page_size
            end = min(start + page_size, count)
            rows = [
                {"id": index, "body": "synthetic-" + str(index).zfill(6) + "-" + "x" * 48}
                for index in range(start, end)
            ]
            for row in rows:
                line = (json.dumps(row, separators=(",", ":")) + "\n").encode()
                raw.write(line)
                digest.update(line)
                seen += 1
    if seen != count:
        emit({"status": "incomplete", "reason": "source page coverage mismatch"})
    else:
        delta = {
            "added": sorted(key for key in current if key not in previous),
            "changed": sorted(key for key in current if key in previous and current[key] != previous[key]),
            "removed": sorted(key for key in previous if key not in current),
        }
        emit(
            {
                "status": "complete",
                "revision": revision,
                "raw_sha256": digest.hexdigest(),
                "item_count": seen,
                "delta": delta,
                "raw": {"worker_artifact": "full"},
            },
            [{"name": "full", "path": "raw.jsonl", "media_type": "application/x-ndjson"}],
        )
