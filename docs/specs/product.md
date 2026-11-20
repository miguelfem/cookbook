# CookBook APR: product and implementation specification

## Preserve solved workflows — 2026-09-22

The default journey is to solve and verify a task with the user's agent, then
preserve the repeatable operation as a recipe. The agent retains observed
inputs/outputs, actual steps and checks, source references, variable inputs and
remaining decisions. It derives the recipe from that reference and validates
equivalent results before reuse. Prior accepted evidence can supply the reference;
missing evidence is labeled unknown. CookBook does not automatically record or
compile conversations, and reference capture does not authorize new effects.

Known steps become code or reviewed tool calls. Jev or a chosen model handles
bounded residual judgment; exceptions return to the agent. Setup and validation
are front-loaded, while recurring calls retain complete outputs and checks.
The installed skill and on-demand MCP guides expose this process. The overview
maps user needs to supported capabilities; `workflow_design` explains operation
ownership and migration of existing project procedures. Agents preserve quality
requirements while replacing covered manual operations with recipes and verified
receipts. Optimization evidence counts actual model activity inside handoffs,
separately from the number of runtime steps.

Optional website recipes use an isolated headless browser with a fresh session
per run. Reviewed recorded clicks can execute without model calls, with fresh
observations and per-action postconditions; caller/Jev decisions remain optional.
This helper does not inherit browser logins or control the foreground desktop.
Native macOS automation still shares that desktop. See the supported operations
and current limits in [browser setup](../runbooks/background-browser.md).

## Prepared recipe reuse — 2026-09-22

Once a recipe and its capabilities are prepared, the agent can call its saved
reference and use completed output directly. Small recipes finish within the
bounded initial call; longer work retains its durable run and explicit inspection
or caller-handoff path. Exact input validation, complete required outputs and
permissions are preserved.

An optional compact result returns the complete recipe output and receipt
identity while leaving progress details available through full inspection. The
default response stays compatible. The installed skill supplies a short entry
point; full authoring, API/LLM, Jev, connection and recovery guidance loads on
demand. Neither smaller replies nor local execution alone establish token savings.
Setup and repeated use are measured separately against equivalent accepted outputs.

An agent can send one jobs JSON to a prepared recipe that declares a bounded
keyed batch. The owner selects 1–64 simultaneous native/API operations in Settings (default
four), with separate local-script and named-connection limits. Slots refill as
jobs finish, respecting recipe limits and shared budgets. Results retain input
order and full required data. Explicit child recipes remain version-pinned; a
request cannot introduce arbitrary executable targets. Short composed batches
can complete within the same run call; longer work and caller decisions retain
their durable inspection/handoff path. See [batch jobs](../runbooks/batch-jobs.md).

Codex connection setup offers an optional, explicit **Trust recipe runs in Codex**
choice. The review explains that it covers recipe execution in projects using
that managed connection while CookBook still enforces workspace access and execution grants.
Other Codex actions and its sandbox retain their approval settings. The default
keeps the client's existing policy. This is distinct from approving a new account,
operation or native environment in CookBook.

## Local credentials and Python access

CookBook saves an account once in a private local TOML file, then reuses it for approved operations without operating-system vault prompts. The file is plaintext, accessible to the same OS user, outside projects and excluded from CookBook backups. Existing installations can explicitly import old saved keys once; OS consent may be required for that migration. Saving an account never approves a new operation.

Agents prepare Python environments when a recipe needs them. Owners review the interpreter once in **Settings → Python recipes** for the chosen workspace. Repeated use keeps that grant; changed interpreters or revoked access still require review. Python has the user's native filesystem/network privileges.


## Connecting services — 2026-09-21

The agent inventories existing approved tools, evaluates the service’s official
MCP capabilities and proposes only the operations a recipe needs. The owner can
connect an account once and reuse it for compatible, separately reviewed operations.
Saving an account, approving an operation and sharing a tool across projects are
separate choices. Browser OAuth uses the native protected flow; API adapters identify
their required credential type. See [the account contract](../architecture/connection-library.md)
and [the connection journey](../runbooks/connecting-services.md).

## Optional recipe improvement — 2026-09-21

A user can ask their agent to review a recipe's retained runs, propose a specific
change, test it and present an adoption decision. The cycle is manual and opt-in;
it does not optimize recipes in the background. Successful execution alone is
not evidence of answer quality. Evidence remains bounded, scoped and attributable,
with missing records, unknown usage and unobserved human effort made explicit.

An immutable candidate preserves public contracts and required outputs. Its
frozen evaluation policy and disjoint labeled cases determine pass, fail or
inconclusive status. The first implementation supports controlled account-free
evaluation of deterministic recipes in disposable runtime state. Unsupported
provider, native-script and effectful paths require additional evaluation; they
cannot dispatch production operations during this check.

Only a complete report with a demonstrated declared benefit makes a candidate
eligible for explicit review. Adoption names the exact candidate/report and checks
that the selected baseline has not changed. It affects selected future work;
existing runs and schedules retain their pinned versions. Ordinary authorized
recipe authoring remains available outside this optional improvement workflow.

## Optional learning from Other — 2026-09-21

A user may enable category learning for a classifier. CookBook collects unmatched
cases, asks the current agent for supported new categories, evaluates a frozen
candidate on independent labeled examples and lets the caller register a reviewed
new version. This is off by default and never mutates running work, grants or
schedules. The [user flow](../runbooks/classifier-learning.md) and
[architecture](../architecture/classifier-learning.md) define bounds and evidence.

## Desktop 0.2 / macOS–Linux extension — 2026-09-18

[Current execution and billing contract](../architecture/macos-linux-completion.md) adds owner OAuth/refresh, MCP resources/prompts/bounded subscriptions and durable user/model continuations, optional approved text sampling workers, read-only Apps, durable whole-period billing, invoice matching, dispatch reservations, reviewed price refresh, and signed-update/release tooling. Earlier tools-only/no-OAuth/no-ledger/no-updater statements below describe the 0.1 baseline and are superseded for this extension. Native Windows remains excluded. Actual publisher signing, live accounts and complete native credential acceptance require separate evidence; implementation is not a signed stable release. Recipe authoring and cost analysis remain agent-driven.

## Recipe/desktop extension — 2026-09-17

On the implementation branch, recipe-author payload/cost analysis, scoped connection proposals, trusted owner approvals, outbound tools-only MCP, and the Tauri desktop now extend the earlier inspector. [Current architecture](../architecture/recipe-connections.md) and [desktop runbook](../runbooks/desktop.md) supersede earlier statements that all of those surfaces are deferred. Workflows are branded **Recipes**; wire identifiers stay compatible. Source presence is not release evidence: consult the validation log for actual checks. OAuth login/refresh, Windows, signing, general visual building, invoice reconciliation and hard monetary caps are not implied.

## Visual scope correction — 2026-09-17

Workflows are generally authored by the agent and displayed as read-only diagrams with nested process/run navigation. The user can fine-tune existing Jev questions and directly associated decision policies through explicit preview and new-version publication. There is no general node builder, code editor, graph rewiring, or automatic modification of active runs in this scope. Desktop packaging is deferred to a later run; the service remains usable headlessly.

Implementation status, 2026-09-16: the [tracker](../plans/task.md) and [validation log](../logs/runtime-validation.md) describe the running Rust/SQLite service, managed MCP setup, trusted workers, protected connectors, optional Jev, manifest0.2 composition, live browser inspector and maintenance/distribution tools. The fuller product journeys below remain release requirements; model-client acceptance, provider quality and economics are not implied by local implementation.

Version 0.2 — 2026-09-16. **Design, not implementation.** This is the canonical product specification. Technical mechanisms are defined in linked architecture documents. The machine-readable manifest remains draft v0.1; broader routing/composition requires a contract extension.

## 1. The product

APR is a general local workflow runtime that Codex and Claude Code can use to turn work into reusable, versioned processes. The agent chooses, builds, and revises processes. APR executes operations, routes work through declared paths, composes processes, retains intermediate data, and records progress. Exact rules handle settled choices; an optional decision model handles bounded semantic choices. Reasoning and generation may call an approved external LLM API directly, with an explicit provider/model choice for cost, speed or quality, or use a durable caller handoff. Tools owned by the current agent require that handoff. Direct model APIs use their approved JSON HTTP or MCP contracts and account billing; they do not require Jev or inherit client subscriptions.

The primary interface is the existing agent conversation. A small desktop inspector makes processes, runs, permissions, connections, and pending work visible. A visual workflow editor is not required to use the product.

Example request: “Compare these proposals, identify the missing information, and prepare a decision memo.” A parent process can classify the documents, run extraction/comparison processes in parallel, route incomplete cases to clarification, and hand interpretation and drafting to the agent. The same mechanism should serve research, business operations, software delivery, and creative production.

## 2. Problem and product hypothesis

An unconstrained agent often rereads large documents, restates plans, makes many small tool calls, and handles deterministic glue repeatedly. Replacing an n8n-style workflow with that pattern can raise orchestration token use even when the actual production steps have not changed.

APR should reduce **repeated orchestration** through reusable code, compact discovery, batching, explicit state, and artifact references. It does not make image generation free, eliminate model calls needed for judgment, or guarantee that generated code is correct.

The economic hypothesis is: setup cost amortizes over sufficiently similar runs, while accepted-result quality is maintained. The [measurement plan](../architecture/measurement.md) separates authoring from execution and tests this hypothesis. Rust is an implementation choice; fewer model interactions are the mechanism.

## 3. Confirmed requirements

| ID | Requirement |
|---|---|
| R01 | Run locally with a lightweight Rust core. |
| R02 | Let an agent create and maintain processes, rather than requiring a human to build a visual graph. |
| R03 | Discover and expose processes per project and assignment without flooding the context with every tool definition. |
| R04 | Let an agent manage ordinary configuration and request named credential bindings. |
| R05 | Support independent execution through configured APIs or local operations. |
| R06 | Support explicit durable pause/resume for work performed by the calling agent using its available tools. |
| R07 | Integrate with both Codex and Claude Code through their supported extension mechanisms. |
| R08 | Preserve the work locally and on GitHub with enough context to continue on another device; keep the repository private during v1 development. |
| R09 | Support general workflows across domains, including routing complex work between processes and within their steps. |
| R10 | Keep Jev optional: the default core, rules-based routing, and explicit caller handoffs require no Jev adapter, credentials, or independent inference account. |
| R11 | Provide optional computer-use workflows with credited upstream reuse, bounded actions, explicit observable completion or durable caller review, and conservative recovery after uncertain effects. |
| R12 | Let authorized agents set up durable one-time, interval and calendar jobs per assignment, with visible timing, no overlapping unfinished runs and recovery after downtime. |

## 4. Proposed first release

A macOS-first developer preview, followed by Linux. One Rust executable, a local SQLite database, and filesystem artifact storage. Built-in steps cover common structured-data and artifact operations. One optional trusted Python executor covers existing helpers. Independent connector calls and caller-assisted steps share the same execution record.

The first release has a CLI, stdio MCP server, two client adapters, a minimal process manifest, and durable step boundaries. A Tauri inspector comes after the engine proves useful. The HTML mockup specifies that inspector's experience; it is not a commitment to build the whole UI before the engine.

Authoring uses ordinary files plus machine-readable validation. The first graph supports acyclic steps, explicit dependencies, typed bindings, and bounded item expansion. It does not evaluate arbitrary expressions embedded in YAML. Complex pure transformations belong in a script. Repeated creative steps use explicit agent requests; retries have finite budgets.

General routing and composition are a core product direction delivered in a later vertical slice of this preview, after the durable engine and connector foundation. The [routing design](../architecture/workflow-routing.md) proposes explicit branches, child-process calls, joins, and shared limits. These are not supported by the current v0.1 manifest. [Decision providers](../architecture/decision-providers.md) are optional compiled/configured adapters. Jev is excluded from default builds and setup; its live evaluation never gates the core release.

## 5. Terminology

| Term | Meaning |
|---|---|
| Process | A reusable definition of inputs, outputs, steps, dependencies, and requested capabilities. |
| Process version | Immutable snapshot of a definition and executable inputs, identified by content digest. |
| Project | A registered workspace and its process catalog, defaults, and connection bindings. |
| Assignment | A named task within a project, with a specific tool selection, limits, and lifetime. |
| Attachment | Device-local association between a client connection and authorized project/assignment context. |
| Run | One execution of a frozen process version and resolved configuration. |
| Step instance | One persisted unit of work; expanded items receive separate stable identities. |
| Artifact | A stored input or output, referenced by identifier and integrity metadata. |
| Connection | A named route to an external service, with credentials managed separately from process source. |
| Handoff | A saved request for the current agent or another explicitly assigned worker to complete a step. |
| Grant | Runtime permission issued through trusted local control, scoped to specific resources/actions. |
| Route decision | A recorded selection from declared eligible paths, accepted by a versioned policy. |
| Child run | A pinned process execution linked to a parent step, with explicit input/output contracts and shared assignment limits. |

## 6. Core user journeys

### 6.1 First setup

The user installs APR and connects Codex, Claude Code, or both once. The installer
previews and merges user-level MCP and skill entries, preserving existing settings.
The agent creates or reconnects a CookBook project in its current authorized
folder; the user does not select a project, copy configuration or run setup commands.
The client may require its normal project trust/MCP approval. APR does not approve itself.

The agent proves existing file access and receives an explicit workspace handle
for every subsequent call. Different projects remain scoped; chats in one project
share its saved recipe workspace. Connection and credential grants still require
owner approval. Advanced fixed-attachment setup remains available.

Setup reports settings installation separately from a real client connection.
A model-driven journey is accepted only when that actual client exercises the flow.

First-run defaults need no external account. An API connection is added only when a selected process needs it. Installation, setup, and binding operations must never require the model to see a secret.

### 6.2 Reuse a process

1. The user gives the agent an assignment.
2. The adapter establishes an assignment within the project already bound to the client.
3. The agent discovers a short list of relevant processes, then requests the selected process's full input contract.
4. APR preflights inputs, required capabilities, executor availability, bindings, and estimated operation counts.
5. The agent starts the run within existing grants. The initial tool call returns after a short bounded wait.
6. APR executes multiple steps and stores intermediate data. The agent sees completion, an exception, or a structured handoff.

Discovery cannot select another project's authority. Matching by name is only search; executing requires the same policy checks as an explicit process reference.

### 6.3 Build a process

When no suitable process exists, the agent first solves and verifies the operation,
or inspects an accepted reference within its authority. It derives a draft with
bounded inputs/outputs and side effects declared per step, writes files in the
project's allowed draft area, and validates equivalent output using fixtures or
an authorized dry run where supported. Unobserved behavior remains unqualified.

Validation checks syntax, schema compatibility, dependency graph, pinned file references, bounded fan-out, effect classification, and required permissions. It reports errors at exact paths. It does not claim semantic correctness or security approval.

The agent can register an immutable version within its current authoring grants. A successful run can be suggested for reuse; promotion never silently broadens scope or creates new credential access. Changes create a new version, while existing runs keep their pinned version.

Agents should avoid building a process for a trivial one-time action. Prefer authoring when an existing deterministic helper is available, repetition is expected, or persistence is necessary. Expose setup cost so the user can judge whether reuse pays off.

### 6.4 Pause for the current agent

An agent step produces a request with the purpose, relevant artifact handles, expected result schema, and continuation identity. APR commits the waiting state before returning it.

The current agent reads only the necessary context, uses its own authorized tools, verifies the observed result, imports the resulting files, and submits structured output. The response is validated and the next step becomes eligible.

Closing the conversation leaves the request pending. Reopening or switching agents requires an explicit attachment/claim to that assignment; APR does not guess ownership from a project folder or silently send a message into an unrelated task.

An unavailable client tool produces a capability blocker. The agent can explain it or the user can explicitly select a configured independent provider. The system must not invent a result or silently switch to a billable provider.

### 6.5 Run independently

The run may continue while the client is disconnected if all remaining operations have independent implementations and current grants. Provider/model calls are explicit connector steps, with separately configured credentials and operation limits.

“Prefer current agent” and “independent provider” are explicit process inputs or binding choices resolved before the affected step. A lost caller is not automatic permission to spend through a fallback provider. A sleeping or powered-off machine cannot execute jobs; the UI explains what will recover when the service returns.

### 6.6 Diagnose and resume

The inspector presents the failed or waiting step, completed outputs, and the exact missing condition. Actions distinguish supplying a result, reconnecting a credential, reconciling an uncertain effect, retrying a safe failure, and starting a new run.

A duplicate result submission cannot trigger another image request or write. A timed-out external action is not automatically failed: when its outcome is unknown, the run stops for reconciliation.

### 6.7 Move to another device

Git carries process definitions, tests, dependency locks, adapter templates, and documentation. Device-local grants, secrets, client identifiers, and mutable runs are not Git content.

For this repository, cloning transfers the design work. For the future product, “attach project” recreates local registration and connections. Transferring an in-progress run requires a separate verified export/import operation; sharing SQLite through a synced folder is unsupported.

### 6.8 Route a complex assignment

An agent can select or author a parent process that delegates parts of an assignment to reusable child processes. APR filters candidate routes by declared contracts, current authority, available capabilities, and limits. Exact rules or an optional decision model select among eligible paths. The runtime records the accepted route and executes it.

The inspector shows the chosen path, child runs, pending work, and recorded decision evidence. Parallel children share the assignment budget. A child handoff can be fulfilled by the current agent using the same durable request/result mechanism.

If no process fits, the assignment returns to the agent with an explicit no-match result. The agent may propose and validate a new version. The first routing extension starts a linked replacement run for a changed graph; it never silently rewrites an active run. Whole-assignment completion, correction effort, and cost determine whether this delegation is useful.

### 6.9 Computer use and scheduled work

The agent can scaffold and register a bounded computer-use process. An owner prepares its native dependencies, allowed applications and execution grants. Routine choices can use optional Jev or explicit caller requests. Each observation/action retains a separate execution record; unsupported or stale UI state stops for review. A declared, observed success condition can finish an independent run. Without it, caller verification remains explicit. See [computer-use architecture](../architecture/computer-use.md).

The agent can schedule a registered process for one instant, a fixed interval, or UTC cron times. The schedule freezes its process/configuration and grants, survives conversation/service restarts, and creates ordinary runs when due. Missed times coalesce; a job with unfinished work does not overlap itself. The inspector shows jobs and their histories. Pausing a schedule prevents new occurrences without pretending to cancel its already accepted run. Jobs need an awake device and running supervisor. See [scheduled-job architecture](../architecture/scheduled-jobs.md).

## 7. Scope and exposure

| Scope | Contains | Behavior |
|---|---|---|
| Personal library | Reusable templates and pinned versions | Projects explicitly import them; imports grant no connections. |
| Project | Processes, safe defaults, declared connection names | Bound to a registered local root and local grants. |
| Assignment | Selected catalog, overrides, capabilities, budgets | Cannot widen the project's grants; can be explicitly closed. |
| Run | Frozen version/config references, attempts, artifacts | Survives a client disconnect, subject to retention and grants. |

Assignments are explicit records even when a client exposes no reliable session identifier. Multiple assignments can coexist in one folder. Attachment identity and run identity are different.

Closing an assignment prevents new effects and stops eligible work at a safe boundary. An in-flight external request may already have completed; reconciliation remains available. Closure never claims rollback.

## 8. Agent-facing surface

The everyday MCP surface is four stable tools:

| Tool | Job |
|---|---|
| `apr_discover` | Search the authorized catalog or describe one selected process. |
| `apr_run` | Preflight or start a pinned process version with a deduplication key. |
| `apr_inspect` | Read run state, pending work, bounded logs, or an artifact excerpt. |
| `apr_resume` | Submit a result or request an explicit legal transition. |

Authoring uses a separately enabled `apr_manage` dispatch tool with action-specific schemas, plus normal file editing. The CLI exposes the same operations. The runtime does not publish every process as a separate MCP tool in the first release.

Tool results default to concise status and artifact references. Large outputs are read on demand with byte/line bounds. Pending work is checked at relevant interactions or via supported notifications, not by repeatedly invoking the model for unchanged status.

All changing operations return structured errors with whether retry is safe and what input or authorization is required. [Protocol details](../architecture/protocol.md) are normative for implementation.

## 9. Configuration and connections

Ordinary config follows this order: process defaults, project defaults, assignment overrides, allowed run overrides. Types and which keys may be overridden are part of the process contract. Resolve once when creating a run.

The agent can see connection names, provider type, binding status, allowed operations, and health. It can bind an already authorized connection. Adding credentials uses a trusted local UI, secure prompt, or supported provider flow.

Protected connections execute authenticated calls in a host-side broker. Arbitrary generated code receives no plaintext secret. A trusted script mode can deliberately inject an environment value, but the UI must identify that the script can read it and access the host. This mode cannot claim protected execution.

Details and enforcement limits are in [security and configuration](../architecture/security.md).

## 10. Desktop experience

The inspector is optional for normal execution. The live 0.2 interface opens to **Home**, which explains the flow: connect a project, ask the agent for a recipe, then run and review it. **Recipes**, **Runs**, **Connections**, **Costs**, and **Schedules** are separate pages. Recipe and run details open contextually; API activity, author controls, and isolated App previews are under Advanced. Page navigation preserves the in-memory inspector session and supports Back without reloading. Project selection and account/access changes remain in the native **Settings & access** window, whose sections also have separate pages.

The main run detail shows a compact timeline, the current step, resulting artifacts, and a relevant action. The routing extension adds selected branches and expandable child runs, with the rule/model result and any review that determined the route. “Waiting for your agent” explains the requested capability. It never offers a fake “Generate” button that the desktop app cannot fulfill itself. The existing mockup predates these routing views.

Process detail shows purpose, inputs, version, execution mode, requested access, fixtures, and recent outcomes. Configuration separates editable values from connection bindings. A secret never appears as a revealable text field in the agent-facing surface.

The interface shows estimated savings only after a comparable baseline exists. Before then it can report measured steps, calls, runtime, and artifact count; it must label sample data in demos.

Home explains CookBook as a shared tool layer for connected agents: recipes can reuse approved services or project-specific tools, follow repeatable steps and rules, call an external model API chosen for cost, speed or quality, optionally use Jev for routing, and hand work back to the caller agent. Credentials stay in the protected store; this copy must not imply automatic access for every project. Each assignment still needs approved connections. No low-cost or savings claim appears without measurements.

See [design system](design-system.md) and the historical [interactive mockup](../../design/inspector.html).

## 11. Product acceptance

| ID | Observable acceptance |
|---|---|
| A01 | A fresh supported client can discover and run a fixture after install/attach and its normal trust flow. |
| A02 | An agent authors, validates, registers, and reuses a process without a visual editor. |
| A03 | At least five deterministic operations run without five separate model decisions. |
| A04 | A waiting agent step survives daemon restart; a valid submitted result resumes the correct version exactly once in local state. |
| A05 | Duplicate start and resume requests return the recorded outcome; conflicting payloads are rejected. |
| A06 | A second assignment cannot discover, inspect, claim, or execute unauthorized resources through APR's API. |
| A07 | A configured independent run completes after the originating MCP connection closes. |
| A08 | A protected connector's credential is absent from generated-code input, output, logs, and manifests; its grants constrain operations. |
| A09 | A provider-success / response-loss simulation enters reconciliation instead of blind retry. |
| A10 | Codex and Claude Code each complete the same caller-assisted fixture using a supported available tool; unavailability has a tested recovery path. |
| A11 | Frozen process code and inputs are unchanged by edits made after the run starts. |
| A12 | Comparative benchmarks report accepted-result cost, authoring cost, quality, and uncertainty without extrapolated savings claims. |
| A13 | The routing extension completes fixtures across at least three domains using the same engine primitives, including a rules-only process without a model provider. |
| A14 | Committed route choices and parent/child identities survive restart without reclassification, duplicate child dispatch, or fabricated skipped outputs. |
| A15 | No-match, missing context, review, cancellation, and uncertain effects have explicit outcomes; nested or parallel work cannot bypass the assignment's grants and shared limits. |
| A16 | A default installation without Jev completes local rules-based routing, nested processes, durable caller-result submission, and recovery with zero APR-created inference requests; an absent optional provider blocks only steps explicitly requiring it. |

These criteria describe future verification. This repository's documentation checks satisfy none of the runtime criteria by themselves.

## 12. Non-goals and release boundaries

The first release excludes arbitrary remote hosting, a cloud control plane, a public process marketplace, automatic cross-device live-run sync, a new general-purpose language, a full n8n-style visual builder, and guaranteed exactly-once external effects.

It also excludes tool access hidden behind unsupported client internals, autonomous credential acquisition, and claims that a Rust process safely confines arbitrary native code. Cross-client portability applies to the process contract; available creative tools and provider entitlements can differ.

Windows, richer sandbox executors, scheduling, multi-user/team distribution, signed process packs, and remote workers are later decisions. Linux support should follow the macOS vertical slice before broad desktop packaging.

## 13. Critical tradeoffs and open decisions

1. **Build versus extend.** Lobster already handles local reusable pipelines; Code Mode handles compact code-driven composition. APR must demonstrate value in the combined client integration, scoped lifecycle, and durable caller handoff. Revisit the build decision after the first comparable prototype.
2. **Reliability versus authoring freedom.** Explicit step boundaries constrain generated code but make recovery inspectable. An arbitrary script cannot be resumed at an arbitrary instruction.
3. **Isolation versus compatibility.** Trusted Python supports existing helpers quickly. Strong isolation requires a separately tested backend and narrower capabilities.
4. **Smooth setup versus client control.** One-time trust and normal tool permission prompts remain owned by the client.
5. **Runtime simplicity versus UI scope.** Ship the engine and adapters before the full inspector. Keep both on one internal API.
6. **Adaptability versus accumulated error.** A complex process can consist of many small decisions, but plausible local choices do not guarantee a correct final result. Evaluate candidate discovery, routing, execution, and escalation together. Use models only where meaning changes the next action.
7. **Remaining decisions.** Product name, license, eventual isolated executor, validated platform matrix, and distribution channel remain open. Working choices are recorded in [decisions](../logs/decisions.md).

The [implementation plan](../plans/implementation.md) defines work packages and dependencies; [the tracker](../plans/task.md) records status and [the verification plan](../plans/verification.md) maps acceptance to required evidence.

## Configurable capacity — 2026-09-17

Deep composition uses explicit per-process budgets bounded by a persisted owner policy. New authoring defaults to 32 call levels; owner approval can admit up to 128. Graph size and route-aware execution work are budgeted separately. Preserve existing manifests and accepted runs. The [capacity runbook](../runbooks/capacity.md) owns current values and controls.

## Analytics increment — 2026-09-17

The inspector now includes retained local operational analytics and optional per-connection usage/cost observations. It exposes missing coverage instead of showing unknown charges as zero. Estimates and provider-reported monetary amounts remain separate, currency-separated views, and displayed subtotals cover only the returned call page. Connections is a read-only binding-status surface; owner key setup and outbound MCP are not implemented by it. The [metering contract](../architecture/metering.md) owns exact semantics. Desktop packaging remains separate.

## Reusable tools and execution review — 2026-09-21

CookBook stores a provider credential once and offers the exact owner-approved tool in a discoverable library for existing and future projects when All projects is selected. Workspace-only approval remains available. Agents reuse an immutable approved contract, propose new operations for review, and never receive secrets. Recipe authoring inventories incremental model round trips and retained context, compiles routine operations into the runtime, records each judgment boundary, and exposes an advisory audit and bounded output contract. See [connection library](../architecture/connection-library.md) and [authoring review](../architecture/recipe-authoring.md).
