# Look up a list of fictional profiles in one call

This recipe takes up to 16 distinct fixture addresses, from `person01@example.test`
through `person16@example.test`. It runs up to four independent Python workers
and returns one profile per address in the original order. The worker looks up
fictional records and adds a short, deterministic wait to simulate I/O. It makes
zero external requests or model calls.

The [manifest](process.json) uses an empty JSON Pointer (`""`) as the
`foreach.key`, so CookBook keys each job by the whole address. A list of strings
can go straight in without a wrapper object or a second copy of each address.
The worker uses the address as an input and the output list keeps it as the item
key.

## Try it

From the repository root, using Python 3.11 or newer and the pinned Rust toolchain:

```sh
cargo build --locked -p apr-cli
python3 tools/evals/benchmark_profiles.py --apr target/debug/apr --output work/profile-batch
```

Use a new output directory on each run. The harness creates its own project and
state, grants the fixture interpreter there, registers the manifest, and runs the
same batch at worker limits of 1 and 4. It also runs the equivalent script at both
limits. The oracle checks all eight profiles and their order for every run. For
each CookBook batch it checks one complete reply and replays the same request key
to verify that the returned run ID and output match.

For normal use, connect an agent to a project and register the example through
the [first-project setup](../../docs/runbooks/first-project.md). Use the registered
`process_ref` and an [approved interpreter](../../docs/architecture/execution.md).
The prepared request uses one `apr_run` call with new inputs and a new
`request_key`.

```json
{
  "process_ref": "<registered recipe version>",
  "mode": "start",
  "input": {"emails": ["person03@example.test", "person01@example.test"]},
  "request_key": "example-profile-batch-1",
  "result_view": "output",
  "wait_ms": 5000
}
```

Include your connected workspace handle when using project-scoped MCP. Use the
successful complete output directly. Choose `receipt` when the caller will open the saved profiles in the inspector. If the short wait ends first, keep the run ID
and inspect it when you need the result. [Batch runbook](../../docs/runbooks/batch-jobs.md)
