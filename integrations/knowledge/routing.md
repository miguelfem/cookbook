# Routing, composition and review

Use manifest 0.2. Exact decisions use `kind:"decision"`, `effect:"pure"` and `decision.source:"rules"`, with declared routes, ordered cases and a default route. A branch uses `when:{decision:"route_step",route:"declared_label"}` and depends on its decision. A join collects the selected branch and returns `{route,output}`; reference `/output` to use the branch value alone. The attached example demonstrates these fields and a real caller review; the example's critical input waits for the agent, while its routine input completes locally.

For semantic routing, read `guide:"jev"` before using the optional provider. A Choice result feeds a separate `decision.source:"choice"` step; the model cannot choose undeclared processes, change permissions or rewrite the workflow. Missing/malformed observations are failures, not an accepted negative result. Low or missing confidence follows the declared clarification route when a confidence threshold is configured.

For nesting, register children first and pin their returned `sha256:` process references in `call` steps; register parents last. Read the exact call schema from `guide:"schema"`. Use task-appropriate shared bounds for `max_call_depth`, `max_child_runs` and `max_step_instances`; the authoring defaults of 32/1024/10000 are ceilings to choose within, not a reason to allocate large workflows. Owner capacity can be stricter. Changing a child requires new child and parent versions; saved runs retain their frozen graph.

A `waiting_for_child` parent is not an agent handoff. Follow the returned child ID or inspect `pending`, fetch the actual leaf request completely, and resume that leaf. Preserve the root ID for final inspection. Do not manually recreate or rerun children. Parent controls affect descendants; state which run a pause/cancel targets. Forward only needed child output fields instead of growing nested envelopes.

For a prepared batch, accept one jobs object, expand bounded `foreach` items with
stable unique keys, and return the required keyed results together. Pin different
child recipes in the definition; job input cannot select arbitrary process refs.
When the running service advertises `parallel_execution`, one coordinator fills
up to the owner's `max_parallel_workers` ready native/API operations, refilling
as each finishes. Default is four; the owner can choose 1–64 in Settings →
Execution, with lower local-script and named-connection caps. Inspect
`apr_inspect {view:"workers"}` for policy and assignment-scoped counts. MCP is
read-only for this policy; do not edit it as an authoring operation. v0.2
`foreach.concurrency` accepts 1–64 and still limits items separately. Limits by
connection name do not combine different aliases for the same provider. Provider
rate limits, grants and all shared budgets still apply.

External work releases the coordinator lock. Same-account OAuth requests may
overlap; only token refresh is serialized. Pure/control work and optional model
sampling remain serial. Declare real dependencies in `needs`; independent writes
must not rely on an incidental completion order.

The start call briefly follows a constant-size status of the complete composed
family, including up to 4,096 children. Use a complete successful output directly.
An actionable handoff, held/uncertain tree or exhausted wait budget returns the
durable run. Older services fall back to a complete page of up to 25 runs.
Do not poll by restarting. Every launched observation is settled; successful
full outputs survive failed siblings, but completed effects are not rolled back.
Cancellation interrupts matching native process groups; HTTP requests cannot be
unsent. Setup and authoring costs are separate from recurring reuse. Parallel
execution alone establishes neither speed nor token savings.
