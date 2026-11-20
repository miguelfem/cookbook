# API and process protocol

## Prepared MCP results

`apr_run.result_view` controls the reply. Full remains the default. The
preference is excluded from service execution arguments and request-key identity.

| View | Successful, complete reply |
| --- | --- |
| `full` | Original run envelope, including progress and output. |
| `compact` | Run identity, receipts and full output. Omits `steps`, `completed_steps`, and `summary` when that makes the reply smaller. |
| `output` | `run_id`, `state`, `result_view`, `output_complete: true`, and the exact `output`. |
| `receipt` | `run_id`, `state`, `result_view`, and `output_available: true`. The full saved output is available in the inspector. |

Projection requires a succeeded, untruncated response with output and without
an error or pending caller request. Preflight, unfinished, failed, uncertain and
truncated replies retain their original form. The output view uses the existing 64 KiB tool reply ceiling. The other
views use the normal 8 KiB ceiling. `apr_inspect {"view":"run","run_id":...,"detail":true}` returns the full
authorized audit. Use only views advertised by the connected server.

## User-level project connection

`apr --state-dir ABS mcp --projects` adds `apr_project` with tagged actions
`prepare {root, name?}` and `connect {challenge_id}`. Prepare returns a five-minute
session proof for the agent's existing file tools; connect returns an opaque
`workspace`. All five ordinary tool argument objects accept `workspace`, required
in this mode and forbidden in fixed-attachment mode. There is no implicit selection.
Public guide-only discovery is allowed before connection with unknown capabilities;
project operations require a verified handle. Tokens and attachment paths stay
private. See [client authority](client-integrations.md) for the proof, lease,
revocation and concurrency rules. Fixed connections retain their four/five tools.


## Desktop 0.2 / macOS–Linux extension — 2026-09-18

[Current execution and billing contract](macos-linux-completion.md) adds owner OAuth/refresh, MCP resources/prompts/bounded subscriptions and durable user/model continuations, optional approved text sampling workers, read-only Apps, durable whole-period billing, invoice matching, dispatch reservations, reviewed price refresh, and signed-update/release tooling. Earlier tools-only/no-OAuth/no-ledger/no-updater statements below describe the 0.1 baseline and are superseded for this extension. Native Windows remains excluded. Actual publisher signing, live accounts and complete native credential acceptance require separate evidence; implementation is not a signed stable release. Recipe authoring and cost analysis remain agent-driven.

## Recipe/desktop extension — 2026-09-17

On the implementation branch, recipe-author payload/cost analysis, scoped connection proposals, trusted owner approvals, outbound tools-only MCP, and the Tauri desktop now extend the earlier inspector. [Current architecture](../architecture/recipe-connections.md) and [desktop runbook](../runbooks/desktop.md) supersede earlier statements that all of those surfaces are deferred. Workflows are branded **Recipes**; wire identifiers stay compatible. Source presence is not release evidence: consult the validation log for actual checks. OAuth login/refresh, Windows, signing, general visual building, invoice reconciliation and hard monetary caps are not implied.

Draft v0.1, 2026-09-14; implementation status updated 2026-09-16. The [local CLI walkthrough](../runbooks/local-cli.md) and [scoped MCP setup](../runbooks/client-integration.md) document implemented entry points. The four everyday MCP tools, restricted authoring, attachment authorization, and service are implemented. Managed setup, layered configuration, run-tree controls and reconciliation are also implemented; explicitly marked future concepts are outside the executable surface.

## Implemented transport

For fixed connections, the attachment selects exactly one assignment; tool arguments cannot choose or expand it. `apr_discover` supports a query/limit, a pinned version with `detail: "schema"`, or an exclusive `guide` topic for installed authoring knowledge. `apr_run` accepts that version, input, a stable request key, and preflight/start mode; start saves a queued run. `apr_inspect` supports run, pending, events, and artifact views; `detail: true` requests the larger budget. `apr_resume` supports submit, renew, continue and cancel. Author mode adds `apr_manage` for process.validate, process.register and artifact.import using project-relative files. Fixed connections cannot create or close assignments or revoke attachments. The explicitly installed user-level bootstrap above is the separate project-creation path.

The default 8 KiB and explicit 64 KiB response budgets are enforced in bytes, including tool-result overhead. Request lines are capped at 1 MiB before MCP parsing and action arguments at 64 KiB. Events have a scoped cursor; catalog and pending lists are bounded and flag truncation. Artifact text previews are at most 32 KiB before JSON encoding; larger or non-text artifacts return metadata. The bridge uses SDK-generated strict schemas and structured results without a second textual copy. These tests do not establish native client approval behavior or model token savings. Actual model behavior and token economics need separate client trials.

MCP `apr_run` start now briefly waits after durable acceptance. `wait_ms` defaults
to 1000, permits 0–5000, and is a bridge-only polling budget; preflight never
waits. A small finished recipe returns its ordinary envelope and output in that
same call. Only queued/running states are polled through authenticated service
reads. Waiting handoffs, child work, failures and other actionable states return
promptly. A deadline returns the last authorized envelope and durable run ID;
an authorization or transport failure is surfaced, never replaced by cached
output. The bridge never starts the work again or owns its execution. Start,
cold-service startup and each IPC read retain their own timeouts, so `wait_ms`
is not a total tool wall-time cap. Oversized output remains explicitly truncated
and requires `apr_inspect {"view":"run","run_id":"…","detail":true}`.
Known prepared execution needs no authoring-skill or guide reload; authoring,
configuration and troubleshooting use the installed skill. In project-aware
mode, run/inspect/resume/manage advertise a required nonempty workspace string;
pre-connect installed-guide discovery and fixed-attachment schemas are unchanged.

Clients should consume complete returned output directly, reuse known recipe
contracts, and inspect only unfinished or truncated results. Every inspection
requires `view`; strict parsing and client approval policies are unchanged.

Manifest 0.2 adds [routing/composition](workflow-routing.md); the existing service/MCP wire remains 0.1 with additive inspection fields and actions. Baseline 0.1 process files retain their meanings. Current additional inspect views are runs, config, bindings and effects; resume includes pause, retry and reconcile; authoring includes config.read/set, process.test and authorized process.promote. Owner-only credential, grant, quota, restore and installation operations are not exposed through the generic MCP dispatcher. The SDK-generated schemas and Rust admission are authoritative for the executable surface; broader target API sketches below remain design material.

## Installed authoring knowledge

Guides reuse `apr_discover` without adding tools or authority. The bridge embeds version-matched markdown, JSON Schemas and complete examples. Topics: `overview`, `workflow_design`, `authoring`, `execution_review`, `schema`, `schema_v0_1`, `starter`, `routing`, `jev`, `classifier_learning`, `recipe_improvement`, `connections`, `adapters`, `handoffs`, `schedules`. The overview maps use cases to these guides; `workflow_design` covers capability selection and migration of solved procedures. A guide request excludes query/limit/process_ref/detail and has the 64 KiB complete-response budget, including wrapper; it is never a partial manifest. It makes no inference or external provider call.

Connected guide calls authenticate an ordinary `service.info` request. A user connection may read public guides before bootstrap with unknown runtime metadata. The service response adds `runtime_version` and `features.jev`; these describe the actual service process, which can outlive an app upgrade. Older services omit these fields; guide responses expose `null`, not an invented enabled/disabled result. The guide reports its own revision and bridge version separately. Feature inclusion never grants connection access or demonstrates live account health. The installed skill and MCP initialization both point to this discovery path.

## Shared conventions

Use JSON Schema draft 2020-12 for process I/O and handoff results. JSON messages include `protocol_version: "0.1"` where they cross the service boundary. Reject unknown major versions; minor additions must be negotiated and preserve existing semantics.

Identifiers are opaque strings. Do not parse authorization from a name or prefix. Handoff timestamps are UTC RFC 3339 strings; schedule fields ending in `_ms` use UTC Unix milliseconds, and intervals explicitly use seconds. Version digests use `sha256:<64 lowercase hex>`.

The transport authenticates an attachment before dispatch. Project authority does not come from a `project_id` supplied by the model. A call cannot override that attachment's assignment.

MCP annotations are honest: discover and ordinary inspect are read-only; run, resume, and manage can change local/remote state. Clients retain their own approval policy. A broad dispatch tool must not be advertised as harmless merely because some actions are reads.

## Everyday tools

| Tool | Input / behavior |
|---|---|
| `apr_discover` | Query/limit, pinned `process_ref` with `detail: "schema"`, or exclusive `guide:"overview"` (and its listed topics). |
| `apr_run` | Immutable `process_ref`, `input`, allowed `config_overrides`, stable `request_key`, and `mode: "preflight"` or `"start"`. Start durably accepts, then waits up to `wait_ms` (default 1000, range 0–5000) for a usable envelope; work stays independent. |
| `apr_inspect` | `view`: run, runs, pending, events, artifact, config, bindings, effects, schedules or schedule. Scope is fixed by the attachment. Explicit detail requests the larger response budget. |
| `apr_resume` | `action`: submit, renew, retry, pause, continue, cancel or reconcile, with the action-specific strict arguments. |

Default responses are at most 8 KiB including the response wrapper; explicit detail is at most 64 KiB. Byte budgets are not token-count promises. Truncation is explicit. Pending handoffs are complete or omitted, with a bounded discovery path; never execute a partial handoff. Root, parent and child identities make nested work navigable without embedding every child payload in the parent.

Pending requests remain discoverable after restarting a client. Use inspection when returning to a task; do not ask a reasoning model solely to poll unchanged state. Pause hides actionable handoffs and prevents new dispatch. A valid result for an already-observed request can commit while paused, but execution waits for explicit continue; renewal is unavailable while paused.

## Authoring, configuration and owner control

Author mode adds `apr_manage`: process.validate, process.register, process.test, process.promote, config.read, config.set and artifact.import. Files are relative to the registered project and checked against its directory identity. `process.test` runs bounded pure and caller fixtures; it refuses native/network effects. Promotion requires an existing owner grant. Ordinary configuration overlays replace top-level keys in defaults → project → assignment → allowed run override order, then validate the complete result and freeze its digest/revisions.

Scheduled jobs add author actions `schedule.create`, `schedule.pause`, `schedule.resume` and `schedule.delete`; read views `schedules` and `schedule` remain available to either attachment mode. Creation accepts name, process_ref, input, config_overrides, stable request_key and a strict once/interval/cron trigger. The schedule is independently accepted work, including any already-authorized external operations. Read the [schedule contract](scheduled-jobs.md) for frozen authority, UTC timing, bounded history and overlap semantics. Computer-use scaffolding creates ordinary manifest 0.2 processes and needs no additional MCP tool.

Run-only mode exposes the four everyday tools. Changing mode may need a client refresh. It never changes the runtime's authority checks. Assignment/attachment creation, closure and revocation, project configuration, credential entry, interpreter and connection grants, rebind, quota, backup/restore and installation remain owner CLI operations. The model cannot turn a config value into a plaintext secret read.

See the runnable [CLI walkthrough](../runbooks/local-cli.md), [owner controls](../runbooks/owner-controls.md), and [client integration](../runbooks/client-integration.md). The CLI's `--help` and SDK-generated strict MCP schemas describe exact flags/arguments. Secret entry uses protected stdin and a private local TOML credential store, never a `--secret VALUE` argument.

## Run states

| State | Meaning / valid next action |
|---|---|
| `queued` | Accepted; eligible for scheduling |
| `running` | One or more eligible steps executing |
| `waiting_for_agent` | Saved native-tool/judgment request; renew/submit |
| `waiting_for_child` | Saved child identity; inspect that child or its pending handoff |
| `paused` | Deliberate pause at a safe boundary; continue after recheck |
| `blocked` | Missing executor, binding, grant, or artifact condition |
| `needs_reconciliation` | An effect may have occurred; inspect/reconcile before retry |
| `succeeded` | All required outputs committed and validated |
| `failed` | Exhausted or non-recoverable failure |
| `cancelled` | New work stopped; any in-flight effects remain in the record |

Inspect physical step state and child identities when branches differ. A parent can remain waiting_for_child while that child needs reconciliation; parent inspection does not aggregate all descendant blockers. Paused/cancelled headlines deliberately persist. Success requires selected work and the final output contract to complete; skipped branches supply no fabricated results.

Completion is terminal. Editing or rerunning a successful process creates a new run. A retry of a failed step is an explicit transition with a new attempt and unchanged pinned inputs; it does not erase the failed evidence. Budget increases, config changes, or replacement artifacts require a recorded amendment and revalidation, or a new run when they change prior dependencies.

## Handoff shape

The [example waiting response](../../examples/contracts/waiting-for-agent.json) is a representative envelope. A request includes:

- Request ID, physical step-instance ID and generation. The envelope supplies run/version identity; the attachment supplies assignment authority.
- Required capability such as `image.generate` or `structured.reasoning`; these are semantic labels, not an assertion that a matching native tool exists.
- Bounded instructions and artifact inputs; no wholesale transcript.
- Result schema and artifact constraints. Submission carries provenance; there is no separate handoff schema-digest field.
- Claim/continuation token, expiry, and exactly which operation it permits.

The client adapter uses the current agent's existing tool-selection mechanism. It does not send native tool names into the Rust daemon and expect them to execute.

Submission requires a unique `submission_key`, request identity/generation, continuation token, observed output, and provenance. Plaintext token exposure is limited to the authorized caller; events/logs show redacted identifiers.

The durable receipt records output digest and transition. After authenticating the attachment and scope, look up an existing receipt before validating a new claim. The same submission key and semantic payload yields the same receipt even after the token has been consumed. The payload digest excludes transport credentials. A conflicting payload returns `SUBMISSION_CONFLICT`. A new submission under an expired or transferred claim returns `STALE_HANDOFF`; it never instructs blind regeneration.

## Process manifest

The [0.1 schema](../../schemas/process.schema.json), [0.2 schema](../../schemas/process-v0.2.schema.json) and [local review example](../../examples/local-review/process.json) define:

- `schema_version`, stable process name, human description, I/O schemas.
- Config defaults, override allowlist, capabilities, and finite run limits.
- Steps of kind `builtin`, `script`, `connector`, or `agent`; 0.2 adds `decision`, `join` and `call`.
- Named dependencies and typed input bindings.
- `effect`: pure, read, write, agent, or 0.2 control; finite retry policy. Native/network effects have no automatic retry.
- Optional bounded `foreach` with unique stable item keys and explicit concurrency.
- Final output bindings.

Bindings are tagged objects. `{"input": "/record"}` selects from run input; `{"step": "select", "pointer": "/value"}` selects prior committed output; `{"item": "/document_id"}` selects the current expanded item; `{"config": "/label"}` selects frozen ordinary configuration; `{"literal": ...}` supplies a value. JSON Pointer selects data; it is not an executable expression.

Expansion produces `{"items":[{"key":"document_1","output":{...}}]}` ordered by the frozen input list. Each child has its own receipt/checkpoint. A duplicate key or item count beyond the declared limit prevents expansion.

The JSON Schema covers shape. Semantic validation separately checks references, paths, authorization, bounds, DAG acyclicity, output mappings, compatible value types, and effect/retry constraints. The documentation checker validates all shipped manifest examples against their versioned schema, input fixtures and native source hashes; runtime tests separately exercise semantic admission and execution.

## Errors and recovery

The service returns bounded errors with code and message fields inside the request-identified response envelope. Retryability is not a separate public error field. Internal stacks, credentials and unrelated project paths never enter scoped responses. The current mapping in `apr-runtime/src/service.rs` is authoritative; clients should act on codes and explicit state, not parse prose.

An uncertain native/network effect persists an immutable intent before dispatch. Reconciliation requires the intent and step identity plus bounded evidence. `observed_result` supplies the observed executor output; for Jev it supplies the original raw provider response, which is checked against the frozen request and normalized again. `confirmed_no_effect` records evidence before allowing a bounded new attempt. `cancel_uncertain` cancels pending work in the subtree while retaining evidence that an effect may already have occurred. Rebinding never resolves uncertainty, and reconciling a cancelled run never resumes it.

Future asynchronous-provider polling, separate approval executors, claim transfer and live graph migration need new contracts; they are not accepted actions today.

## Local analytics inspection

The additive `apr_inspect` view `analytics` accepts optional run_id, limit (1–20) and after. Request detail for its bounded ledger payload. No run selects the assignment; an authorized run selects its whole root tree. Authorization remains assignment-scoped. The response separates retained summary metrics from current-page currency subtotals, exposes unknown coverage, and uses decimal-string nanocurrency values. See the [metering contract](metering.md). This does not add a top-level MCP tool or any owner permissions.
