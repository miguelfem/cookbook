# Prepare one reviewed CLI adapter

Use this route when an approved MCP tool or bounded direct JSON HTTP connection cannot perform the required operation, and official CLI/SDK documentation identifies a narrower local operation. The agent prepares code with its already authorized file and terminal tools. The owner separately approves the exact CookBook connection and supplies any credential through the protected native field. This scaffold does not install software, obtain credentials, register a connection, or grant execution.

Use the absolute `cli_executable` returned alongside this installed guide for the `APR` command below. It must point to the installed `apr` binary; the adapter templates are embedded in that binary and do not require a source checkout. Choose an absolute Python interpreter with the stdlib available under `-I -S`. The generated MCP binding pins its interpreter, source and spec. Its target is one fixed absolute CLI with fixed arguments. Every declared executable and dependency file is hashed. The wrapper sends one bounded JSON object to the CLI's stdin and requires one bounded JSON object on stdout. The target runs with a cleared environment, private working directory and timeout. No shell, package installation, dynamic command arguments or dynamic destination is supplied by the scaffold.

First inspect the exact target CLI, its arguments, imports, data destinations, effects, authentication and installed dependency versions. Declare all file dependencies, including helper scripts. Hashes cannot discover transitive native libraries, SDK packages, login stores or later network downloads; review and pin those separately or report the unsupported dependency. A Python SDK helper, CLI translator or API client can be the fixed target when it accepts this JSON-line contract. You must write and test its service-specific translation yourself. The default scaffold is a transport wrapper, not an API renderer or automatic OpenAPI importer. Local native code has the owner's OS privileges and is not sandboxed.

## Account-free synthetic example

This example creates only disposable files. Use it to verify the scaffold and protocol; replace it with a separately reviewed service helper. Substitute the guide's actual absolute `cli_executable` for `APR`. The `PYTHON` discovery is preparation only; the generated binding uses its resulting absolute path.

```sh
APR='/absolute/apr/path/from/this/guide'
WORK="$(mktemp -d)"
PYTHON="$(python3 -I -S -c 'import os,sys; print(os.path.realpath(sys.executable))')"
cat > "$WORK/normalize.py" <<'PY'
import json
import sys

value = json.load(sys.stdin)
if not isinstance(value, dict) or not isinstance(value.get("text"), str):
    raise SystemExit("expected text")
print(json.dumps({"text": value["text"].strip().lower()}))
PY
"$PYTHON" -I -S - "$WORK" "$PYTHON" <<'PY'
import json
import pathlib
import sys

work = pathlib.Path(sys.argv[1])
python = sys.argv[2]
text = {"type": "object", "properties": {"text": {"type": "string"}},
        "required": ["text"], "additionalProperties": False}
spec = {
    "schema_version": 1,
    "name": "normalize_text",
    "cli_executable": python,
    "cli_args": ["-I", "-S", "-u", str(work / "normalize.py")],
    "dependency_files": [str(work / "normalize.py")],
    "credential_env": None,
    "input_schema": text,
    "output_schema": text,
    "timeout_ms": 2000,
    "max_input_bytes": 4096,
    "max_output_bytes": 4096,
}
(work / "adapter-spec.json").write_text(json.dumps(spec), encoding="utf8")
PY
"$APR" adapter prepare --spec "$WORK/adapter-spec.json" --output "$WORK/bundle" --python "$PYTHON" > "$WORK/prepared.json"
"$APR" adapter check --directory "$WORK/bundle"
```

Both commands return JSON. Require `ready:true`, empty `issues`, and `grant_created:false`. `binding_proposal` is a concrete modern `2026-07-28` stdio MCP binding. If `ready:false`, fix the reported file, digest or executable problem, prepare a **new** bundle and review its new binding; do not edit a frozen bundle in place. `check` only verifies files and hashes. It does not invoke the CLI or prove service behavior.

Run this separate synthetic end-to-end check before proposing access. It launches only the disposable normalizer through the prepared, pinned MCP command. The `tools/call` result's `structuredContent` must match the spec's `output_schema` (`{"text":"example"}`); `content` is an empty array and the modern result also has `resultType:"complete"`.

```sh
"$PYTHON" -I -S - "$WORK/prepared.json" <<'PY'
import json
import subprocess
import sys
import tempfile

prepared = json.load(open(sys.argv[1], encoding="utf8"))
assert prepared["ready"] is True and prepared["grant_created"] is False
transport = prepared["binding_proposal"]["transport"]
meta = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}}
requests = [
    {"jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {"_meta": meta}},
    {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {"_meta": meta}},
    {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
     "params": {"name": "normalize_text", "arguments": {"text": "  EXAMPLE  "}, "_meta": meta}},
]
with tempfile.TemporaryDirectory() as private_dir:
    result = subprocess.run(
        [transport["executable"], *transport["args"]],
        input="".join(json.dumps(item) + "\n" for item in requests),
        text=True, capture_output=True, timeout=10, env={}, cwd=private_dir,
        check=True,
    )
replies = [json.loads(line) for line in result.stdout.splitlines()]
assert len(replies) == 3 and [item["id"] for item in replies] == [1, 2, 3]
assert replies[0]["result"]["supportedVersions"] == ["2026-07-28"]
assert replies[1]["result"]["tools"][0]["name"] == "normalize_text"
assert replies[2]["result"]["resultType"] == "complete"
assert replies[2]["result"]["structuredContent"] == {"text": "example"}
print("synthetic adapter contract passed; no connection grant created")
PY
```

For a real helper, test a harmless representative operation only when the user has authorized that operation and account. A successful synthetic test proves the wrapper and fixed protocol shape, not provider access, permission, cost, correctness or safety. A lost response after a target invocation may mean an effect happened; inspect and reconcile before retrying.

## Submit the exact reviewed connection

Construct one `Connection` from the returned `binding_proposal` without changing its pinned executable, arguments, hashes, protocol or selected tool contract. The synthetic connection shape below illustrates the fields; replace the label, schemas and authentication only with the target's reviewed contract. The outer `output_schema` describes the full MCP result. The tool's `output_schema` inside `binding_proposal` describes `structuredContent`.

```json
{
  "label": "Synthetic text normalizer",
  "endpoint": "stdio:local",
  "method": "POST",
  "auth": {"kind": "none"},
  "input_schema": {"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false},
  "output_schema": {"type":"object","properties":{"resultType":{"const":"complete"},"structuredContent":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}},"required":["resultType","structuredContent"]},
  "timeout_ms": 5000,
  "max_response_bytes": 16384,
  "loopback_test": false,
  "mcp": "INSERT THE UNCHANGED binding_proposal OBJECT HERE"
}
```

In the actual `apr_manage` `connection.propose` call, `mcp` must be the JSON object, not the explanatory string above. Use `{action:"connection.propose",name,request_key,rationale,connection}` with a new request key and the exact scoped connection. `auth.kind:"none"` is only for a truly unauthenticated target. For one protected key, declare one safe uppercase `credential_env` in the source spec, use `auth.kind:"bearer"` or the supported named header with literal `credential_ref:"pending"`, and give the service's exact nonsecret `credential_label`. Never put the key in source, args, spec, synthetic input, model-visible output or this conversation. Extra credentials or a CLI-owned login store need separate reviewed support; the cleared environment will not inherit your shell login.

Proposal creation is not authorization. Ask the owner to review the access request in **Settings & access** and provide any key in CookBook's protected field. Do not approve your own proposal through owner controls. After approval, perform the permitted bounded discovery/call, inspect the returned contract and recipe readiness, and report remaining setup blockers precisely. Tool output may contain sensitive data: keep it within the authorized CookBook result/artifact path and do not echo it arbitrarily to the calling agent or logs. A changed helper, dependency, account, destination or effect contract needs a new bundle and owner review.
