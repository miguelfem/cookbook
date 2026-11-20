# Start here

For the product overview, read the [repository README](../README.md). The [evaluation plan](plans/evaluations.md) describes the next small pilots; the [results index](evaluations/README.md) separates planned studies from existing engineering evidence. [Credits](../CREDITS.md) records source reuse and design influences.

For a normal installation and a first recipe in another project, start with [Install and first project](runbooks/first-project.md). Agent knowledge ships through a short user skill plus on-demand MCP guides; see [client architecture](architecture/client-integrations.md).

For new reusable work, solve and verify the task with the agent first, retain a
[redacted reference](../examples/reference-first/README.md), then derive and test
the recipe. Optional [background browser recipes](runbooks/background-browser.md)
preserve observed website steps in an isolated browser. The
[qualification report](evaluations/2026-09-22-reference-first-browser.md) separates
this tested path from native foreground automation and unproven model savings.

Optional [learning from Other](runbooks/classifier-learning.md) adds agent-assisted category proposals and held-out evaluation to a Jev classifier. It is off by default.

For a manual review of an existing recipe, ask your agent to review retained runs,
propose one change, test it and explain whether to adopt it. The installed
`recipe_improvement` MCP guide describes the [bounded improvement contract](architecture/recipe-improvement.md).
Its first evaluator covers pure deterministic recipes; live providers and effects
need separate qualification.

## Desktop 0.2 / macOS–Linux extension — 2026-09-18

[Current execution and billing contract](architecture/macos-linux-completion.md) adds owner OAuth/refresh, MCP resources/prompts/bounded subscriptions and durable user/model continuations, optional approved text sampling workers, read-only Apps, durable whole-period billing, invoice matching, dispatch reservations, reviewed price refresh, and signed-update/release tooling. Earlier tools-only/no-OAuth/no-ledger/no-updater statements below describe the 0.1 baseline and are superseded for this extension. Native Windows remains excluded. Actual publisher signing, live accounts and complete native credential acceptance require separate evidence; implementation is not a signed stable release. Recipe authoring and cost analysis remain agent-driven.

## Recipe/desktop extension — 2026-09-17

On the implementation branch, recipe-author payload/cost analysis, scoped connection proposals, trusted owner approvals, outbound tools-only MCP, and the Tauri desktop now extend the earlier inspector. [Current architecture](architecture/recipe-connections.md) and [desktop runbook](runbooks/desktop.md) supersede earlier statements that all of those surfaces are deferred. Workflows are branded **Recipes**; wire identifiers stay compatible. Source presence is not release evidence: consult the validation log for actual checks. OAuth login/refresh, Windows, signing, general visual building, invoice reconciliation and hard monetary caps are not implied.

Updated 2026-09-16. Product name: **CookBook APR**. APR means Agent Process Runtime; the CLI command remains `apr`.

## Current state

APR has a seven-crate Rust implementation: pure admission/routing, a SQLite runtime, CLI, official-SDK MCP bridge, managed client integrations, protected connections, and an optional Jev adapter. The local inspector reads real service state. Configuration snapshots, bounded item execution, Python workers, explicit external-effect reconciliation, backups/restore and retention are implemented. Routing/composition and storage/distribution additions are covered by the current completion pass; consult the [tracker](plans/task.md) and [validation log](logs/runtime-validation.md) for their verified status.

The full implementation design remains in the [product specification](specs/product.md), [delivery plan](plans/implementation.md), and [concrete remaining-components plan](plans/remaining-implementation.md). Broader future designs must not be mistaken for supported behavior. The [first-launch evaluation slice](plans/evaluations.md) is maintainer-run, with explicit client/provider scope. Broader token-economics studies and external usability sessions are separate from first-launch acceptance; publisher signing and APR licensing remain release decisions.

Jev remains optional. Default setup and core operation need no inference account. General research, operations, software and creative work share the same engine. Public fixtures use synthetic data and general-purpose tasks.

The latest extension adds [scheduled jobs](runbooks/scheduled-jobs.md) and [optional computer-use process scaffolding](runbooks/computer-use.md). Schedules are durable assignment-owned work; the computer helper is an explicitly prepared macOS native adapter with credited MIT source reuse. [The extension plan](plans/computer-use-and-schedules.md) and validation log distinguish local tests from live GUI/provider qualification.

The current workflow view represents agent-authored processes and their nested execution. Only Jev questions and directly associated policies have visual tuning controls; publication creates a new immutable version. The [corrected UI scope](plans/visual-workflows.md) excludes a general visual builder; its earlier deferred-packaging status is superseded by the desktop 0.2 implementation.

## Start using or reviewing it

1. Install the [native app and connect your first project](runbooks/first-project.md), or open the [local demo](runbooks/local-demo.md) to explore the real interface, or run the [account-free CLI walkthrough](runbooks/local-cli.md).
2. Use [managed client setup](runbooks/client-integration.md) to connect your agent once; it creates projects itself.
3. Open the [live local inspector](runbooks/local-inspector.md) against a private attachment.
4. Read [execution](architecture/execution.md) before granting Python or a named HTTP connection. Native code is trusted, not sandboxed.
5. Read [routing](architecture/workflow-routing.md) for versioned exact decisions, guarded branches, joins and child runs.
6. Check [validation](logs/runtime-validation.md), [verification requirements](plans/verification.md), and the [implementation tracker](plans/task.md) before making release claims.

Use [the index](INDEX.md) for other architecture/research documents. The original `design/inspector.html` is still simulated; it is not the running inspector. Earlier research is dated and should be rechecked before changing provider/client contracts.

## Continue on another device

Follow [the handoff runbook](runbooks/continue-on-another-device.md). Git transfers source and tests. A state backup is a separate private artifact, with explicit restore, project relocation, fresh attachments and owner reauthorization. Do not copy credentials into Git or copy a live database as a substitute for backup.

Keep the baseline recovery and permission tests. Add evidence through real public CLI/MCP/runtime boundaries; an HTTP protocol test does not prove an actual model followed the client skill. Unknown token usage stays unknown. The public source preview is available from GitHub. CookBook’s project license remains undecided.

New workflows can use [configurable capacity](runbooks/capacity.md): 32 call levels by default in generated definitions, owner-enabled depths up to 128, and separately bounded graph size and executed work.

## Reusable tools and execution review — 2026-09-21

The current continuation adds [reusable tools and stored accounts](architecture/connection-library.md) plus [execution-boundary review](architecture/recipe-authoring.md). Existing workspace approvals remain scoped until the owner shares them; new tools may be approved for all projects. Installed MCP guidance includes executable examples and a recipe audit.
