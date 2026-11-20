# Local review fixture

This executable fixture uses all five M0 builtins plus one explicit caller review. It selects a record, writes a JSON artifact, records and validates source artifacts, waits for a review result, and creates a deterministic ZIP containing the sources and that result.

The helper calls the real `apr` binary. It does not invoke AI or simulate runtime persistence. The supplied boolean is a demonstration answer; both `true` and `false` are recorded and packaged. Conditional routing and approval gates are later work.

From the repository root, with Rust 1.94.0 and Python 3 available:

```sh
cargo build --locked --workspace
./target/debug/apr process validate examples/local-review/process.json
python3 examples/local-review/demo.py start
```

The run stops at `waiting_for_agent`. Read its `request` and the source text, then return your chosen answer explicitly:

```sh
python3 examples/local-review/demo.py submit --approved true
python3 examples/local-review/demo.py export
```

The default output is `work/local-review/client/package.zip`. Runtime state lives in `work/local-review/state`; helper snapshots live in `work/local-review/client`. The helper uses private files for envelopes containing a continuation token. Do not add these local files to Git.

Each helper invocation and CLI call starts a new process. Repeating `start` with the same input/request key returns the saved run. Repeating the same `submit` returns its saved receipt. Repeating `export` fails instead of overwriting the existing file; choose a new path with `export --output-file work/another-package.zip`.

To keep a separate demo, pass `--state-dir` and `--demo-dir` before the subcommand. `--apr` selects a built binary, including `target/release/apr`. Use `start --request-key another-key` to create another run in the same store.

See the [CLI runbook](../../docs/runbooks/local-cli.md) for direct commands, result envelopes, renewal, limits and the current M0 boundaries.
