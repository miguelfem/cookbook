# Visual workflows and compact agent access

## Recipe/desktop extension — 2026-09-17

On the implementation branch, recipe-author payload/cost analysis, scoped connection proposals, trusted owner approvals, outbound tools-only MCP, and the Tauri desktop now extend the earlier inspector. [Current architecture](../architecture/recipe-connections.md) and [desktop runbook](../runbooks/desktop.md) supersede earlier statements that all of those surfaces are deferred. Workflows are branded **Recipes**; wire identifiers stay compatible. Source presence is not release evidence: consult the validation log for actual checks. OAuth login/refresh, Windows, signing, general visual building, invoice reconciliation and hard monetary caps are not implied.

Recorded 2026-09-17. Status: corrected user direction. General workflows are agent-authored and visually inspected; only Jev decision questions and associated policies are visually tunable. Desktop packaging is explicitly deferred. Consult the runtime validation log for executed evidence.

## Product direction

CookBook APR should provide a Windmill-like workflow workspace: a flow canvas, step settings/code panel, reusable process catalog, and run history. Users inspect workflows generally authored by their agent. This is not a general-purpose visual builder: there are no add/remove/reconnect controls for arbitrary nodes or code. Jev question and decision-policy tuning is the deliberate exception, because those judgments need manual fine-tuning. Jev remains optional.

Use the admitted process definition for visualization and the same Rust validation boundary for narrow Jev changes. Preview the precise changes and candidate version before Publish. Publishing registers a new immutable version; it never mutates an active run, parent reference, schedule or source file. Graph layout is presentation only. A changed preview must be reviewed again before publication.

The inspector now exposes lazy graph/tree views and narrow author-only Jev tuning. The general workflow graph remains read-only. Do not describe it as a visual builder.

## Editor and execution views

- Left: project/assignment, workflows, runs, schedules, connections. Show the current execution device; selecting a view never changes authority.
- Center: readable flowchart with labeled edges, collapsible child processes, zoom/fit, and breadcrumbs into nested processes.
- Right: selected step's purpose, inputs/output bindings, code or decision questions, schema, connection requirements, retry/limits, and validation errors.
- Bottom or adjacent tab: fixture input, test output, events, and selected run state.

Displayed nodes represent supported builtins, trusted Python, protected JSON operations, typed decisions, caller tasks, bounded foreach, joins and pinned child processes. Ordinary changes remain an agent/source authoring task. JavaScript/TypeScript execution, arbitrary loops and parallel native dispatch are not implied by the diagram.

Jev tuning exposes declared literal Noul/Choice/Score questions, their descriptions and criteria, and directly associated policies. Choice IDs, question types, score cardinality, source bindings and graph routes remain fixed; structural changes remain with the agent. Graph inspection does not dump source inputs, credentials or arbitrary step output. Explicit rule nodes apply thresholds and choose declared routes, including uncertainty/review paths. Batch independent questions where the provider contract allows it. Show the observed score/distribution/confidence and actual selected edge, not an invented reasoning trace. Schema validity does not establish that a decision is correct.

Run mode overlays the frozen graph with completed, selected, skipped, waiting, failed, and uncertain states. Expand a child using its actual run identity. Controls make clear whether they target a child or its entire subtree. A waiting caller node presents a schema-driven result form and evidence; it cannot fabricate a tool result. Exported artifacts stay references with bounded previews.

Arbitrary script internals remain opaque code nodes. There is no general canvas-to-code round trip in this scope. The Jev publisher patches only admitted, explicitly allowed fields and retains pinned source snapshots.

## Measured current agent surface

Inspected the source and real author-mode MCP `tools/list` response on 2026-09-17:

| Component | Measured size |
|---|---:|
| Complete `integrations/skills/apr/SKILL.md` | 785 whitespace-separated words; 5,554 bytes |
| Five author-mode MCP tool definitions, compact JSON | 11,945 bytes |
| Server instructions | 258 bytes |

These are file/wire sizes, not tokenizer counts, per-turn billing, or savings measurements. Clients decide when skill bodies/tool schemas enter context and whether caching or deferred discovery applies. The complete skill is approximately 1,400–1,800 tokens by a rough size estimate, not a tokenizer measurement.

Keep the initial skill focused on discovery, start/preflight, complete handoffs, idempotency, authority, and reconnection. Move detailed authoring, schedules, provider, and computer-use guidance into discoverable references. A proposed initial skill budget is 400–700 tokens, subject to behavioral regression tests. The managed installer must package and track the references too; simply adding links to files it does not install would break the skill.

The largest definitions are `apr_manage` (4,379 bytes), `apr_inspect` (3,081), and `apr_resume` (2,764). Optimize these alongside the skill. Preserve the five-tool compatibility surface while evaluating opt-in operation discovery and a compact execution interface. Hiding all detail behind untyped strings is not an acceptable substitute for validation. Measure actual initial and repeated client context, result bytes, calls, and error rates before claiming an efficiency improvement.

## Composition and Code Mode

The root is depth zero. New authored processes explicitly request 32 edges, 1,024 descendants and 10,000 step instances under device-owner policy. Hard ceilings are 128 / 4,096 / 100,000, with a separate 4,096-version graph ceiling. Legacy omitted budgets retain 8 / 256 / 10,000. See [capacity controls](../runbooks/capacity.md). Cycles are rejected. Child targets, configuration and grants are frozen before acceptance. One externally selected process can therefore represent substantial internal work without loading every child schema into the agent context.

This is durable process composition, not yet arbitrary model-written orchestration code. Code Mode adds a useful second pattern: discover selected typed methods, compose calls/transforms inside one execution, and return only a useful final result. More nesting alone does not reduce model traffic. Every caller-assisted step may still require the caller's participation; provider steps still incur their own provider work.

A future code composition layer must call the same scoped service, preserve child identity and whole-tree budgets, and treat waits as explicit durable continuations. It must not bypass permissions, expose credentials to generated code, or blindly replay effects after interruption. Evaluate a bounded local JS/Wasm execution environment separately from existing trusted Python. Do not assume Cloudflare's Worker executor is an embedded Rust runtime or that a client tool connection is inherited by the service.

## Browser versus desktop

The Rust service, SQLite state, CLI, MCP and schedules already run without a browser. The present inspector is browser-hosted. Explicitly deferred to a later run: a Tauri desktop shell sharing the same frontend and service, with optional browser inspection and headless operation retained. Tauri uses an operating-system WebView; that removes the need for an external browser window, not all web technology.

The desktop app should support normal launch, project setup, server lifecycle/status, and resuming inspection after relaunch. Closing its window should not destroy accepted work. Distinguish closing a window, quitting the UI, stopping the service, and disabling login startup. Retain scoped service checks; desktop integration must not grant arbitrary filesystem/native capabilities to the frontend. Signing, installers, updates, OS permissions and per-platform lifecycle tests are real delivery work, not solved merely by wrapping the page.

Remote-host control is separate from desktop packaging. A machine label and handoff guidance prevent confusing remote localhost with the user's current device. Do not expose the unauthenticated local transport publicly to make a remote browser work.

## Delivery and acceptance

1. **Graph inspection and nesting:** lazy assignment-scoped `apr_inspect` graph/tree views, real frozen run states, child navigation and actionable descendant handoffs. Graph pages exclude full child schemas, source and input payloads; they remain bounded. Qualify default 32 and owner-enabled 128 levels, preserve legacy eight-level behavior, and verify root completion after a deep handoff and process restart.
2. **Jev tuning only:** author-only `jev_tuning` details and `process.tune_jev` preview/publish. Allow existing questions/criteria and directly associated thresholds/fallbacks/rule values; reject graph, source, binding, route-set and permission changes. Publishing requires the preview's candidate hash and retains registered bundled source. Default builds perform structural validation without inference; provider-enabled builds additionally validate through the adapter.
3. **Reproducible nested demo:** register leaf, child and root bottom-up, expose the root as one reusable process, leave an actual leaf review waiting, then verify exact submission replay and root completion without duplicate children.
4. **Agent instructions:** explain compact root calls, lazy child graph/schema inspection and direct submission to the actual waiting child. Existing receipt, scope and uncertain-effect semantics remain unchanged.
5. **Separate future work:** additional skill/tool context optimization and optional isolated code composition remain proposals. A general visual builder is out of scope. Desktop packaging is deferred to a later run.

## References checked 2026-09-17

- [Windmill flow editor](https://www.windmill.dev/platform/flow-editor): script steps, control flow and editing model.
- [TypeSafe workflow evaluations](https://evals.typesafe.ai/) and [invoice workflow](https://evals.typesafe.ai/invoice_processing): typed questions, code rules and visual execution paths. These are demonstrations/evaluations, not evidence that TypeSafe supplies our editor.
- [Cloudflare Code Mode](https://developers.cloudflare.com/agents/tools/codemode/how-it-works/): progressive discovery, code composition and runtime boundaries.
- [Tauri architecture](https://v2.tauri.app/concept/architecture/): Rust and operating-system WebView desktop packaging.
