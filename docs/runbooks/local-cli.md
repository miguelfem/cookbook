# Local CLI: M0

Updated 2026-09-16. This runbook describes the implemented synchronous CLI. It uses SQLite and a local content-addressed artifact store. It can execute supported deterministic steps, save a request for the caller, exit, and resume from a supplied result in a new process. It requires no inference-provider account.

These owner commands preserve the original synchronous M0 path. A separate scoped service and MCP bridge are now available through the [client setup runbook](client-integration.md); that path enforces assignment attachment checks and background ownership. Credential brokers, scripts, provider connectors, routing, automatic client installers and the live inspector remain planned. Do not treat the owner CLI as a sandbox for untrusted users; possession of the state directory is local authority.

## Build and test

The workspace pins Rust 1.94.0 in `rust-toolchain.toml`; install the Rust toolchain separately if needed. The current filesystem implementation targets Unix platforms. Local verification is on macOS; this runbook does not assert a completed Linux release qualification.

```sh
cargo build --locked --workspace
cargo test --locked --workspace
./target/debug/apr --help
```

Commands emit one JSON value on stdout and structured errors on stderr. Help/version output is plain text. The global `--state-dir PATH` defaults to `.apr/local` and can appear before or after a subcommand. Keep state on a local filesystem with working locks and synchronization semantics.

## Run the complete fixture

```sh
./target/debug/apr process validate examples/local-review/process.json
python3 examples/local-review/demo.py start
python3 examples/local-review/demo.py submit --approved true
python3 examples/local-review/demo.py export
```

The first helper call imports a text source, constructs input from its real artifact reference, and runs until `waiting_for_agent`. The second returns an explicitly supplied review answer; it does not invoke a model. The third writes a new ZIP at `work/local-review/client/package.zip`. See the [fixture README](../../examples/local-review/README.md) for storage paths, independent runs and replay behavior.

## Command reference

| Command | Behavior |
| --- | --- |
| `apr process validate MANIFEST` | Parse/admit the supported manifest and report its process version without opening runtime state. |
| `apr run MANIFEST --input-file FILE --request-key KEY` | Freeze the admitted version and input, then execute until completed, failed or waiting. A reused key must identify the same input and version. |
| `apr inspect RUN` | Read the saved envelope, completed steps and current caller request. |
| `apr continue RUN` | Advance recoverable work. It does not invent a caller result. |
| `apr renew RUN` | Explicitly rotate the generation/token of a pending caller request and refresh its 24-hour expiry. Previous unaccepted tokens become stale. |
| `apr resume RUN --result-file FILE` | Submit the complete caller envelope; returns `{ "receipt": ..., "run": ... }`. The argument must match the file's `run_id`. |
| `apr events RUN --after SEQUENCE --limit COUNT` | Read ordered events after the sequence. Default and runtime maximum page size are 100. Continue from the last returned sequence. |
| `apr artifact import PATH --media-type TYPE --max-bytes BYTES` | Snapshot a permitted regular file and return its hash, media type and size. The byte-limit default is 64 MiB. |
| `apr artifact read --reference-file FILE --output-file FILE --max-bytes BYTES` | Verify an immutable object and write to a new file. Existing files/symlinks are never overwritten. The byte-limit default is 64 MiB. |

JSON manifests are limited to 1 MiB before parsing; input, submission and reference files are limited to 64 KiB. Large content belongs in artifacts. File input does not accept ordinary symlinks, directories, devices or pipes. The CLI's file checks are local input hygiene, not an adversarial filesystem isolation boundary.

An artifact's reported media type is a declared label, not content sniffing or malware inspection. SHA-256 detects a changed object; it does not validate its meaning.

## Return a real caller result

Inspect the run and copy the current `request_id`, `step_instance_id`, `generation` and `continuation_token` into a private JSON file. Supply actual observed output that conforms to `result_schema`; do not fabricate a tool result. The shape is:

```json
{
  "protocol_version": "0.1",
  "action": "submit",
  "run_id": "COPY_FROM_RUN",
  "request_id": "COPY_FROM_REQUEST",
  "step_instance_id": "COPY_FROM_REQUEST",
  "generation": 1,
  "submission_key": "A_STABLE_KEY_FOR_THIS_RESULT",
  "continuation_token": "COPY_FROM_REQUEST",
  "output": { "approved": true },
  "provenance": { "source": "caller_tool", "client": "YOUR_CLIENT" }
}
```

`generation: 1` and the output above are illustrative; use the saved generation and requested schema. Run `apr resume RUN --result-file FILE` with the same state directory. If a response is lost, resend the same semantic result and submission key: APR returns the accepted receipt and advances remaining work. A changed result under the same key is a conflict. Renewal does not revoke an already accepted receipt.

APR does not reach into Codex or Claude Code to call a native tool. The caller performs that operation itself, imports any resulting files, and returns observed artifact references through this explicit handoff. Do not put tokens or complete request bodies in ordinary logs, shared documents or Git.

## Supported operations and failure boundaries

The supported pure builtins are `json.select`, `json.write`, `artifact.manifest`, `artifact.validate_set` and `artifact.bundle`. The caller `agent` step requires exactly one attempt and declared budget. Admission rejects unsupported execution kinds, scripts, external connections, foreach and bundled source files. A stored run retains its original process snapshot if the source manifest is later edited.

Artifact publication verifies existing content and uses exclusive publication. Bundles use stable ordering and ZIP metadata; equivalent inputs and report values yield the same hash. Missing or changed referenced artifacts prevent safe use. Preserve evidence when investigating corruption rather than editing database records or replacing content-addressed files manually.

Error responses expose public codes and safe messages rather than raw input, parser values or local paths. `REQUEST_CONFLICT` and `SUBMISSION_CONFLICT` require distinct work or restoration of the original request. For `STALE_HANDOFF`, inspect the current request and only renew when intentionally replacing an unaccepted caller assignment. `LIMIT_EXCEEDED` means an input/artifact/execution bound was exceeded.

The CLI's success exit status means the command was accepted; inspect the returned run `state` for workflow success. Caller waits are durable. This walkthrough does not start the service: use `continue` or a valid `resume` to advance local work. The MCP path queues work for the separate supervisor.
