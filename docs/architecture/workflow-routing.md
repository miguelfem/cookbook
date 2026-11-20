# General workflow routing and composition

Updated 2026-09-16. Manifest **0.2** implements exact-rule and typed-choice decisions, guarded branches, selected-output joins and pinned child processes. Manifest 0.1 retains its existing behavior. [Protocol](protocol.md) describes the unchanged 0.1 service wire, [execution](execution.md) covers effects, and the [completion plan](../plans/remaining-implementation.md) records implementation sequencing and acceptance checks.

## Keys for scalar batch items

A v0.2 `foreach` key is a JSON Pointer relative to the current item.
An empty pointer uses the item itself. The runtime accepts unique nonempty
strings or integers as keys and keeps results in input order. The v0.1 schema
continues to require a property pointer.

## Responsibilities

APR executes admitted reusable work, keeps checkpoints, enforces assignment authority and bounds, and asks the calling agent for open-ended work through a durable handoff. A decision provider can supply a typed observation; deterministic policy decides whether to accept its choice, route to clarification, or return no match. The same primitives serve research, business operations, software delivery and creative production. No domain has privileged scheduler code.

Decision, join and call definitions use the ordinary `needs` graph. They never evaluate expression strings or run model-generated code merely because a route names it. Native code and network operations use separate owner grants.

## Decision grammar

A `kind: "decision"`, `effect: "pure"` step has empty `inputs`, no foreach, and one `decision` object. A rules policy tests cases in declared order using exact JSON equality, then uses its declared default:

```json
{
  "id": "classify",
  "kind": "decision",
  "effect": "pure",
  "needs": [],
  "inputs": {},
  "retry": {"max_attempts": 1},
  "decision": {
    "source": "rules",
    "routes": ["ready", "review"],
    "cases": [{"value": {"input": "/complete"}, "equals": true, "route": "ready"}],
    "default": "review"
  }
}
```

A `source: "choice"` policy instead declares `routes`, a `value` binding, optional `confidence` binding and `min_confidence`, plus declared `no_match` and `needs_clarification` routes. The value can refer to a prior caller result or optional provider observation. Unknown string choices select no match; non-string choices and malformed confidence fail validation. Absent or insufficient confidence when a threshold is required selects clarification. Missing confidence is never invented from a distribution maximum. No provider is called by the decision evaluator itself.

Each committed resolution records `source`, `route`, `reason`, `candidates`, input and policy digests, and the bounded observation. A restart reads that result; it does not reselect the route. The process version pins policy identity.

## Branches and joins

A guarded step declares `when: {"decision": "classify", "route": "ready"}` and depends on the decision. Non-selected work becomes `skipped` with no attempt and no fabricated output. Admission checks decision ancestry, route membership and consistent inherited guards. A step cannot read a possibly skipped output outside its matching branch.

A `kind: "join"`, `effect: "pure"` step uses empty `inputs` and `join: {"decision": "classify", "branches": {...}}`. The branches map covers every declared route exactly once, with one ordinary binding per route. `needs` includes the decision and applicable source steps. Only the selected binding is resolved; the output is `{"route": "ready", "output": ...}`. Failures and pending work do not count as successful joins. There is no implicit fallback to an absent branch.

## Child processes

A `kind: "call"`, `effect: "control"` step declares `call: {"process_ref": "sha256:...", "output_pointer": ""}`. Its ordinary input bindings form the child's input object; the pointer selects the child's final output, which becomes the call step's direct result. Calls can use bounded keyed foreach. Decisions and joins cannot expand foreach.

Before accepting the root, APR traverses every declared child target, verifies current visibility and execution grants, rejects cycles, validates limits, and freezes the complete graph's versions and configuration. Configuration changes or new registrations after acceptance cannot change a pending child's definition. The root transaction also freezes the union of required execution bindings and protects referenced configuration artifacts before child creation.

Creation commits the child run and its parent-step identity in one transaction. Restart finds the same child, never a duplicate. The parent uses `waiting_for_child`; run inspection exposes root/parent/step identities and the call's `child_run_id`. Assignment-wide pending inspection finds actionable child handoffs with their actual child run IDs. Submit to that child using its saved request and continuation identity. The parent joins only validated committed child output.

## Bounds and controls

New agent-authored workflows explicitly declare 32 call levels, 1,024 child runs and 10,000 total step instances. The device-owner policy defaults to those ceilings, plus 4,096 distinct reachable process versions including the root. Owners can explicitly raise execution ceilings up to 128 call levels, 4,096 child runs and 100,000 step instances. The graph-version ceiling is independently configurable up to 4,096. See [capacity controls](../runbooks/capacity.md).

Omitted fields in existing v0.2 manifests still mean 8 / 256 / 10,000. This preserves immutable definitions, schedules, saved identities and serialization; migrating a workflow to larger budgets creates a new explicit version. Legacy 0.1 behavior is retained.

Admission traverses the graph iteratively with memoized child-before-parent bounds, rejects cycles and checks each process's declared limits. It counts every stored step definition, including skipped branches, but mutually exclusive routes contribute only their largest potential child/expanded-step workload. Different decisions are conservatively summed. All possible referenced versions still count against the separate graph ceiling and must pass visibility/grant checks. Runtime accounting checks actual creation and foreach expansion again. Provider intents and caller requests consume shared root allowances as well as local allowances. Uncertain and retried provider attempts stay charged. A child's step-time and artifact-size allowances are capped by its root. Keyed foreach retains ordered results, with at most 64 in-flight items for v0.2 (32 for legacy v0.1) and 1,000 items per declared expansion; the owner selects 1–64 external workers (default four), with lower local, named-connection and declared item limits still applying. Large payloads belong in artifacts.

Execution is supervised by one coordinator with a continuously refilled pool.
Reservations commit before launch; external work releases the writer lock. The
coordinator settles each completion and admits the next eligible job without a
wave barrier. Same-account OAuth calls overlap while refresh alone is locked and
conditionally persisted. Pure/control work and optional model-sampling remain
serial. Full successful results remain saved beside failed or uncertain siblings;
completed effects are never rolled back. See [execution](execution.md).

One `apr_run` starts one saved recipe. A declared jobs input and foreach can
produce a combined keyed result; there is no generic dynamic-recipe batch endpoint.
Short batches can complete in the initial call. For composed work the bounded
wait uses a constant-size aggregate over the entire authorized run family, up to
4,096 children, while no member needs attention. Handoffs, held/uncertain work or
wait exhaustion return the durable run. An older service falls back conservatively
to a complete tree page of at most 25 runs. The
[batch guide](../runbooks/batch-jobs.md) and
[installed check](../evaluations/2026-09-22-batch-execution.md) distinguish those
paths. Recipe setup and authoring cost must be reported separately from recurring
model-turn reuse; parallel runtime work alone proves neither speed nor token savings.

Pausing a parent holds descendants across restart. Continuing rechecks authority and preserves prior results. Cancelling a subtree invalidates pending handoffs and stops future dispatch while preserving completed results and uncertain-effect records. In-flight trusted Python is interrupted and reaped; a dispatched remote request cannot be unsent. Reconciliation of one uncertain outcome never silently resolves another. Explicit retry is limited to eligible pure work; uncertain effects require evidence first.

Retention treats linked runs as a family. Active or pinned children preserve their parent tree and referenced artifacts. Backups include the graph and results, while restore disables execution authority and revokes caller tokens until explicit local reattachment/rebind/resume.

## Examples and verification

Executable examples use the same v0.2 primitives:

- [Research triage](../../examples/research-triage/process.json): supplied structured evidence can complete independently or request a caller synthesis.
- [Operations intake](../../examples/operations-intake/process.json): complete versus review-needed requests.
- [Software verification](../../examples/software-verification/process.json): supplied check results versus caller investigation.

The synthetic service harness verifies both paths in each domain. It records real calls, bytes and elapsed time; model tokens, decision quality and cost remain unknown. Runtime tests cover committed decisions, skipped-output rejection, pinned child identity/configuration, parent controls, shared budgets and process-death boundaries. Exact commands and outcomes belong in the [validation log](../logs/runtime-validation.md).

## Future scope and critique

Dynamic catalog retrieval, hierarchical candidate search, automatic process selection, adaptive graph rewriting, arbitrary loops, distributed worker pools and live graph migration are separate extensions. Current call targets must be explicit immutable versions. A caller can author a replacement process through normal admission, but an active graph does not mutate itself.

Routing improves reuse of a known repertoire. It cannot repair missing evidence, incomplete candidates, incompatible outputs or unclear success criteria. An accepted model choice is not proof of a good business decision. Evaluate held-out examples and complete assignments, including human correction, first-authoring cost, retries and later reuse, before claiming quality or token savings. Jev remains optional under its [provider contract](decision-providers.md).

## Lazy inspection and narrow Jev publication

`apr_inspect` supports `view:"graph"` with exactly one of `process_ref` or `run_id`, optional `limit` (1–100) and `after` step ID. It returns one process's nodes/edges, pinned child references, at most 20 instance previews per node and optional recorded selected routes. Saved runs inspect frozen definitions; normal catalog visibility governs definition inspection. The graph response has a 44 KiB projection budget and standard transport limits still apply. Request `detail:true` for a complete graph page.

`view:"tree", run_id` resolves the authorized root family and returns compact paginated run identities, process names/versions, parent/step identities, depth, state and `actionable_waiting`. Parent pause suppresses actionable waits. No continuation tokens, inputs or raw outputs are included; request the particular child run when its full handoff is needed. The root is depth zero; a depth limit of 32 permits 32 call edges below it. Legacy omitted limits still permit eight; owner-authorized manifests can explicitly request up to 128. Foreach children count against the same root budgets.

Author-only `view:"jev_tuning", process_ref` exposes the declared literal questions and directly associated decision policies. `apr_manage` action `process.tune_jev` uses `process_ref`, `mode:"preview"|"publish"` and narrow `changes`. A question change contains `kind:"question",step_id,question_id,question`; a policy change contains `kind:"policy",step_id,policy`. Publish additionally requires the preview's `expected_process_ref`. Content-addressed publication is replayable and preserves graph/source/authority boundaries. It never rewrites existing child callers or accepted run graphs. See the [inspector guide](../runbooks/local-inspector.md) for supported tuning fields and limitations.

`view:"capacity"` returns the current owner policy through the authorized read-only interface. Attachments cannot change it. New roots and future scheduled occurrences must fit the current policy. Lowering policy does not retroactively cancel accepted work or invalidate same-key replay; active runs keep frozen budgets. Oversized future scheduled occurrences pause with a recorded error. SQLite migration 10 adds this policy without changing stored manifests.

`view:"tree_status", run_id` returns only whole-family state counts, a completeness
flag and whether work can proceed without the caller. It includes no inputs,
outputs or continuation tokens and applies the same root-family authorization as
`tree`. `view:"workers"` exposes owner policy plus running and approximate queued
step counts within the attachment assignment. Other assignments' connection names
and counts remain private. Agents cannot change worker policy through MCP.
