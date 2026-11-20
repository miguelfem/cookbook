"""Pinned stdlib bootstrap embedded literally in the reviewed MCP command args."""
import hashlib
import sys


def checked(path, expected, limit):
    with open(path, "rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit or hashlib.sha256(data).hexdigest() != expected:
        raise SystemExit("adapter source or spec changed; prepare and review again")
    return data


if len(sys.argv) != 5:
    raise SystemExit("invalid adapter bootstrap arguments")
spec_path, source_path, spec_hash, source_hash = sys.argv[1:]
verified_spec = checked(spec_path, spec_hash, 65536)
source = checked(source_path, source_hash, 65536)
sys.argv = [source_path, spec_path]
exec(compile(source, source_path, "exec"), {
    "__name__": "__main__", "__file__": source_path,
    "PINNED_SPEC_BYTES": verified_spec,
})
