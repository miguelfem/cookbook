# Send several jobs as one recipe input

CookBook can accept a list of jobs and return their combined results when a saved
recipe declares that input contract. Your agent authors and registers the recipe
once; later requests supply the jobs. This avoids asking the agent to coordinate
each individual step on every repetition.

Current behavior:

| Need | Available today |
| --- | --- |
| Send a jobs JSON to one recipe | Yes, using a bounded keyed `foreach`. |
| Combine results with stable job identifiers | Yes, in input order. |
| Use different saved recipes in one parent | Yes, with explicit version-pinned child calls. |
| Choose arbitrary recipe IDs from job input | No; child targets belong to the admitted definition. |
| Run native/API workers simultaneously | Yes; the service overlaps 1–64 ready workers selected by the owner (default four), subject to lower recipe/local/connection limits. |
| Always receive the final batch in the first call | No; child waits, handoffs or longer work can return a run ID. |

For v0.2 manifests, a list of unique strings or integers can use an empty
`foreach.key` JSON Pointer to key each job by the whole value. An item-input
binding of `{"item": ""}` passes that value to the worker. The
[profile batch recipe](../../examples/profile-batch/README.md) demonstrates this
with fictional addresses. For lists of objects, use a stable property such as
`/id`.

## Example input and result

The [tested selection recipe](../evaluations/fixtures/batch-execution/direct-foreach.json)
accepts this input inside one ordinary `apr_run` request:

```json
{
  "jobs": [
    {"id": "owner", "payload": {"owner": "Ada"}, "pointer": "/owner"},
    {"id": "count", "payload": {"count": 7}, "pointer": "/count"}
  ]
}
```

Its complete recipe output is:

```json
{
  "results": [
    {"key": "owner", "output": {"value": "Ada"}},
    {"key": "count", "output": {"value": 7}}
  ]
}
```

This is a small contract example. A real recipe can compose approved API tools,
native processing or explicit caller handoffs using the same admitted workflow
mechanisms. Accounts, code pins, operation permissions and root budgets still
apply to every item. Grouping work does not grant additional authority.

Use the discovered input schema and saved recipe reference. Supply one stable
request key per logical batch, and reuse that key only when retrying the same
request. Choose `result_view: "output"` for the complete values and run ID.
Choose `"receipt"` when the caller will display the saved output through the
local inspector. On older bridges use an advertised view such as `"compact"`.
`wait_ms` is bounded to 5,000 and never cancels the underlying work. Use returned
values directly and keep unfinished run IDs for later inspection.
For a composed run, the short wait checks a constant-size summary of the whole
run family, including up to 4,096 children. An actionable handoff, held or uncertain
work, or exhausted wait budget returns the durable run for later inspection.
Older services fall back to the earlier complete-page check of at most 25 runs.

The recipe must define a bounded item count and stable unique keys. Aggregate
results have a 64 KiB runtime bound; use artifacts and explicit references for
larger data. A failed required item fails the batch, and committed external effects
are not rolled back automatically. A caller handoff must be resolved on its actual
child run before the parent can finish.

Choose simultaneous jobs in **Settings → Execution**: 4, 8, 16, 32, or a custom
number from 1 to 64. Local scripts and named connections can have lower limits.
The [capacity guide](capacity.md) explains defaults, counts and owner CLI controls.
A lower recipe `foreach.concurrency` remains the item cap; v0.2 permits up to 64.

The service fills a free slot as soon as a job finishes. One coordinator reserves
provider calls and money before launch, releases its writer lock during external
work and commits every observation. Same-account OAuth calls can overlap; only
token refresh is serialized and cannot overwrite a concurrent owner change.
Successful sibling outputs remain complete if another sibling fails or needs
reconciliation. Lowering limits drains existing jobs before admitting more.

Pure/control steps and optional model-sampling workers still advance serially.
Authenticated cancellation signals every matching native worker, whose process
group is killed and reaped; an HTTP request can only observe its bounded
cancellation/timeout path and cannot be unsent. Cancellation does not roll back
an effect that already happened.

## Run a prepared batch from a connected client

Connect the project and register the recipe during setup. During repeats, reuse
the live session's workspace handle and the frozen `process_ref`. Make one
`apr_run` call with the new inputs. Discovery and review belong to setup and to
changes in the recipe or its grants.

A host that already has typed inputs can invoke the installed MCP tool directly.
For example, Codex App Server can send `mcpServer/tool/call` for `apr_run` on the
connected thread and display its structured output. That path requires no model
turn. Natural-language chat uses a model to interpret the request and to produce
a reply. The local execution itself uses only the workers and providers declared
by the recipe. Codex's optional owner-approved per-tool policy is described in
[client setup](../architecture/client-integrations.md).
For a ready-to-run counts-and-link example, use the
[direct report helper](direct-codex-report.md).

## Where savings could come from

Potential model-token savings come from fewer recurring caller turns and less
intermediate data in the conversation. Recipe authoring, registration, grants,
connection setup and the first preparation are separate setup costs and must be
reported separately from later reuse. Keep complete required results available;
compact output must not remove evidence needed for correctness. Internal LLM
calls still count toward the workflow's cost. Parallel execution alone does not
establish faster completion or lower token usage.

The [installed capability check](../evaluations/2026-09-22-batch-execution.md)
verified keyed results, retry identity and invalid-input rejection. The later
[packaged worker comparison](../evaluations/2026-09-22-parallel-batches.md) verified
complete composed replies and four overlapping native workers: all 18 batches
passed exact output checks. Neither measured model-token savings. For a fair
client comparison, allow both CookBook and the direct agent to batch and retain
setup costs separately from reuse.
