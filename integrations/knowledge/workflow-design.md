# Design a recipe from solved work

Use this guide when preserving a successful workflow, migrating an existing
procedure, or reducing repeated agent work. Reuse an unchanged prepared recipe
directly. CookBook executes the authored graph; it does not compile a conversation
or optimize a written procedure automatically.

## Choose the execution owner

Start from observed successful inputs, operations, outputs and acceptance checks.
Keep the user's model choice, quality requirements and authorization. Separate
variable inputs from reusable code and instructions.

| Work to preserve | Recipe design | Guide |
| --- | --- | --- |
| Calculate, normalize, validate, transform files | Builtins where supported; otherwise a frozen, approved Python helper. Reuse existing scripts. Native code is trusted host code, not sandboxed. | `authoring`, `schema` |
| Combine database, API, MCP or CLI operations into one callable workflow | Named approved connections; a pinned adapter for a missing supported interface. Keep pagination, projection and routine persistence inside execution. | `connections`, `adapters` |
| Generate or reason with a particular external model | An approved model API node with explicit provider/model and supported parameters. Project only needed inputs; validate output and retain usage. | `connections` |
| Answer a bounded semantic question | Optional Jev Choice, Score or Noul node. Its answer can be recipe data or feed a separate declared routing policy. Evaluate the exact question/model before relying on it. | `jev` |
| Apply exact choices or combine reusable workflows | Rules, branches, joins and pinned child recipes. Rules need no model. | `routing` |
| Process many independent jobs | One bounded input batch, stable item keys and declared dependencies; return keyed results together. Parallelism remains subject to service, recipe and provider limits. | `routing` |
| Use a tool available only in the calling agent, or resolve open judgment | A scoped handoff containing the required evidence, precise request and result schema. Execute surrounding routine work in the recipe. | `handoffs` |
| Repeat observed browser actions or operate a native application | Prepared computer-use scaffold with fresh observations and effect checks. Isolated browser and foreground desktop capabilities differ; inspect their limits. | `schedules` |
| Run later, or improve a saved workflow | Explicitly requested schedules; separately requested, evidence-reviewed improvement or learning from Other. No automatic promotion. | `schedules`, `recipe_improvement`, `classifier_learning` |

Availability, connection approval and proven quality are separate. Inspect the
running service and exact selected contracts. Saved accounts are reusable only
within owner-approved tools and scope. A missing connection stays a named
dependency; prepare its proposal rather than disguising service work as a caller
judgment. A guide never grants access or authorizes a paid call.

## Map the whole procedure before writing its graph

For each observed operation, record: source evidence, required check, execution
owner, exact helper/connection or caller capability, inputs, returned evidence,
and unresolved access. Keep this mapping with the reference; it is authoring
evidence, not extra manifest fields. Every original requirement needs an owner.

Group consecutive routine operations into one automatic segment. Reuse a helper
that already performs a batch; count its existing combined call once when
comparing designs. One user objective normally becomes one composed recipe, with
child recipes where reuse, versioning or authority requires them. Large raw
outputs remain artifacts; the caller receives the next decision, receipt or
exception. Reuse unchanged guidance by revision.

Inspect work *inside* each proposed handoff. A requirement for one native image
tool or one visual decision does not make its file preparation, hashes, metadata,
selection and export agent work. Code supplies known identifiers and fixed
structure; the model supplies actual observations and decisions. Fuse persistence
and preparation after an accepted result before returning the next real request.
Preserve every substantive check; compact evidence without inventing passes.

Generation with a caller-only tool and review of its result can share one bounded
request when both belong to that caller. Split requests where a runtime-owned
stage or independent dependency must intervene. During preparation, confirm how
the caller can deliver the exact result schema, including real artifact import
when required. Missing import authority is a dependency to resolve before work
whose result cannot be delivered; a file path alone is not an imported artifact.

For example, a reusable asset workflow can prepare validated requests, generate
through an approved API, retain outputs, ask for visual judgment, then assemble
the reviewed artifacts. With a caller-only native generation tool, keep that tool
in an explicit host handoff. The rest of the ownership map still applies. This is
a design pattern, not evidence of provider quality or a ready-made integration.

## Migrate repository instructions with the execution

Read the existing project procedure as the acceptance contract. Separate quality,
authorization and recovery requirements from the commands the agent used to run.
Within the authorized migration scope, replace covered manual commands with the
recipe invocation, expected receipt and exception path. Keep manual recovery
instructions reachable when that exception occurs. If project guidance cannot be
changed, report the conflicting instruction and proposed revision explicitly.

End with one normal execution path: preparation runs once; unchanged reuse sends
the variable inputs; only a genuine judgment or exception returns to the caller.
Retain the root run and fulfill its saved requests rather than rebuilding the
workflow or starting another run to check progress. Schedules do not wake a
disconnected caller, and CookBook does not inherit another application's tools.

## Qualify the conversion

Fetch `execution_review` for the actual audit sidecar. Record observed baseline
calls and expected caller calls, including calls *inside* handoffs, separately
from raw service requests. Label estimates and unknown tokens. A single agent
step may contain many model responses; a clean static audit is not proof of
automation or savings.

Check equivalent outputs, selected artifacts, required reviews, scope, uncertain
effects and recovery against retained cases. Replay only where recorded inputs
and responses support it, in disposable state without repeating real effects.
Replay proves mechanics; provider quality and token savings need an authorized
matched run. Count setup separately, retain cached/uncached input and output,
corrections and all caller activity. A returned `needs_revision` is unfinished
business work even when the runtime successfully saved that result.

Finish preparation with a pinned recipe, input example, output contract,
requirement-to-step mapping, remaining caller duties and unresolved dependencies.
Then follow `authoring` for validation, registration, readiness and execution.
