# Batch execution — installed capability check

Date: 2026-09-22. The installed app accepted one JSON input for a saved batch
recipe and produced ordered, keyed output. **Batching is supported; simultaneous
native/API dispatch is not.** There is no general endpoint that accepts arbitrary
recipe IDs as jobs. See the [batch usage guide](../runbooks/batch-jobs.md).

## Observed calls

| Fixture | Jobs | Start calls | Later inspections | First response | Start-call time |
| --- | ---: | ---: | ---: | --- | ---: |
| Pure selection with foreach | 2 | 1 | 0 | Complete combined output | 122.261 ms |
| Two pinned child recipes, each repeated twice | 4 | 1 | 2 | `waiting_for_child` | 363.057 ms |

Both final outputs matched the complete expected JSON, including item keys and
order. Repeating the exact request key returned the same run identities; the
composed family still contained one parent and four children. A job missing a
required field was rejected. No model/provider call, network operation or native
execution grant was involved. All processes in the successful disposable fixture
were stopped afterward.

These timings measure the individual MCP start calls, exclude authoring/setup,
and come from one successful capability check per fixture. The composed timing
is **not** its total completion time. This is neither a latency benchmark nor a
model-driven client comparison. There is no measured token-saving percentage.

## Why batching is not parallel dispatch

The [MCP run arguments](../../crates/apr-mcp/src/lib.rs) accept one process version,
one input and one request key. A saved recipe can declare bounded `foreach` work
and collect `{items: [{key, output}]}`. Different child recipes require separate
statically pinned call steps; a job's input cannot dynamically select a recipe ID.

The runtime chooses one eligible step and executes its dispatch inline. The
service advances one run at a time across dispatch boundaries. Several child
runs or caller handoffs can be outstanding, but native and HTTP work remains
serialized. `foreach.concurrency` limits logical in-flight work; it does not
create simultaneous workers. This finding comes from source inspection, consistent
with the [routing contract](../architecture/workflow-routing.md#bounds-and-controls).
Pure operations were too short to provide a meaningful overlap benchmark, so no
native overlap timing was performed or inferred.

The start call waits while its run is queued or running. It returns a durable
`waiting_for_child` response without continuing that wait. Consequently, even a
short composed batch may need a later inspection. Failures are not successful
partial batches: a failed child fails the parent, and already committed sibling
effects are not automatically rolled back.

## Attempts and evidence

The first harness attempt incorrectly asserted that composed work must finish
inside `apr_run`. It observed `waiting_for_child` and stopped. That finding is
retained in the attempt ledger, but its disposable state was removed before the
corrected attempt and its full raw trace is unavailable. The second attempt
accepted the documented waiting state, collected the final result and performed
the replay/input checks above. Neither attempt is presented as a matched
performance study or a replacement successful benchmark case.

Source: `e7de5595be5ec16e4fb6e973a9cb1c389d186fc2`, with documentation-only edits.
Installed APR SHA-256:
`df9b08eae1d36d10b2dded2047d63e9cb1e2239845417e535ab05842b36b37b8`.
Host: macOS ARM64 26.6.2. No runtime source change, app rebuild or installation
was needed. The focused existing restart/order regression also passed:

```sh
cargo test -p apr-runtime --test foreach \
  expansion_keeps_item_order_and_checkpoints_across_a_restart --locked -- --exact
```

That one test establishes order/checkpoint behavior, not simultaneous execution.
Public [metrics](fixtures/batch-execution/metrics.json),
[requests and outputs](fixtures/batch-execution/requests-and-outputs.json),
[direct recipe](fixtures/batch-execution/direct-foreach.json) and
[parent recipe](fixtures/batch-execution/batch-parent.json) retain synthetic,
reproducible contracts. Register the supplied
[selection child](fixtures/batch-execution/select-child.json) and
[echo child](fixtures/batch-execution/echo-child.json) before the parent, whose
references pin those exact definitions. Registering all four definitions is setup,
not part of the measured run calls. The request snapshots use a fixed attachment;
project-aware clients must also supply the workspace handle returned by setup.

Private helper, MCP responses, hashes, attempt ledger and disposable state remain
under ignored `work/evals-2026-09-22/batch-execution/`. Public exports contain no
attachment tokens, grants or user-project data.

## Token-efficiency implication

One authored batch can avoid repeated caller planning and intermediate results
entering the conversation. The jobs still perform their required work, and any
LLM API calls inside recipes still consume provider tokens. Concurrent dispatch
would primarily change elapsed time. A future comparison must permit the direct
agent to batch as well, verify every output and count caller plus internal-provider
usage separately. Current evidence establishes the batching contract, not savings.
