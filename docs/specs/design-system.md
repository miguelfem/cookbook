# Interface and design system

## Desktop 0.2 / macOS–Linux extension — 2026-09-18

[Current execution and billing contract](../architecture/macos-linux-completion.md) adds owner OAuth/refresh, MCP resources/prompts/bounded subscriptions and durable user/model continuations, optional approved text sampling workers, read-only Apps, durable whole-period billing, invoice matching, dispatch reservations, reviewed price refresh, and signed-update/release tooling. Earlier tools-only/no-OAuth/no-ledger/no-updater statements below describe the 0.1 baseline and are superseded for this extension. Native Windows remains excluded. Actual publisher signing, live accounts and complete native credential acceptance require separate evidence; implementation is not a signed stable release. Recipe authoring and cost analysis remain agent-driven.

## Recipe/desktop extension — 2026-09-17

On the implementation branch, recipe-author payload/cost analysis, scoped connection proposals, trusted owner approvals, outbound tools-only MCP, and the Tauri desktop now extend the earlier inspector. [Current architecture](../architecture/recipe-connections.md) and [desktop runbook](../runbooks/desktop.md) supersede earlier statements that all of those surfaces are deferred. Workflows are branded **Recipes**; wire identifiers stay compatible. Source presence is not release evidence: consult the validation log for actual checks. OAuth login/refresh, Windows, signing, general visual building, invoice reconciliation and hard monetary caps are not implied.

The live interface supports read-only visual workflow inspection with nested processes and narrow Jev decision tuning. The [visual-workflow plan](../plans/visual-workflows.md) records implementation and acceptance requirements. The older [interactive mockup](../../design/inspector.html) is historical and simulates every action. The live inspector renders actual process/run structure and supports Jev preview/publication.

## Product character

APR should feel like a quiet local utility: clear state, inspectable work, and a small number of relevant actions. The agent generally authors workflows. Users visually inspect their structure and nested execution. Only Jev question and associated policy nodes provide manual tuning controls; ordinary nodes cannot be added, removed or reconnected here. Jev changes must be previewed and published as a new immutable version. The runtime enforces the same assignment authority and source/binding boundaries.

Product display name: **CookBook**. Agent Process Runtime describes the underlying system; `apr` remains the CLI and integration identifier. Use CookBook in the application title and wordmark, with the technical expansion in documentation where useful.

## Information architecture

The live 0.2 sidebar has Home, Recipes, Runs, Connections, Costs, and Schedules. Each destination displays one page; detailed API activity, recipe author controls and desktop App previews sit under Advanced. Selecting a recipe or run opens its own detail page with a return link. Hash navigation retains the browser inspector's in-memory session, strips the initial session credential, marks the current navigation item, and moves keyboard focus to the page heading.

Home is a concise entryway: explain what a recipe does, lead to the recipe library, and show current work needing attention. Keep getting-started guidance in a short disclosure instead of repeated feature cards. Explain repeatable work and handoff to the user's agent in ordinary language. Keep machine references and raw JSON in expandable details. Local measurements, unknown costs and permission boundaries must remain precise.

The top bar shows the page purpose, local service status, and a native Settings & access action. The trusted settings window separates project setup, access requests, server requests, billing and updates, with technical setup under Advanced. Changing projects or approving connections continues to use the existing owner commands; navigation creates no authority.

The 2026-09-16 [routing extension](../architecture/workflow-routing.md) adds expandable child runs beneath their parent step and clearly distinguishes selected, skipped, waiting, and failed paths. A route detail shows the considered options, the recorded rule/model result, any review, and the selected process version. Show observed evidence; do not generate an explanation and present it as the model's recorded reasoning.

Keep shared assignment limits visible when inspecting a child. Label whether pause/cancel targets that child or the parent and its descendants. A pending agent request opens with its exact run and step identity. These routing views are specified here but are not yet present in the static HTML mockup.

## Primary screens

| Screen | Information | Primary action |
|---|---|---|
| Home | Short product explanation, recipe-library path, waiting agent steps and access requests | Browse recipes / connect an agent |
| Recipes | Searchable saved workflows and their purpose | Open a recipe / ask the agent to create one |
| Run detail | Step timeline, instance progress, artifacts, provenance, current blockers | Act on the precise current state |
| Connections | Provider/account label, scope, permitted methods, health, expiry | Connect or rebind through trusted local flow |
| Setup | Client detection, proposed config patch, trust/connection state | Attach selected project |
| Permission request | Concrete action/resource, duration, existing grant, requested addition | Approve scoped request or decline |

The initial HTML mockup is historical. The live inspector and trusted native setup implement the current navigation; consult the validation log for exercised behavior.

## Agent conversation

Default progress is compact: “Checked 8 records. Waiting for your review of record 3. Earlier results are saved.” Link/open details only when useful.

When a run finishes, report the useful output, important validation result, and artifact reference. Do not dump intermediate JSON, paths, or a complete timeline into the conversation.

A blocker names the missing condition and available next action. “Image provider is not connected” is useful; “Invalid runtime capability binding exception” is not.

## State and action design

| State | Label | Action |
|---|---|---|
| Running | Working locally | View progress / pause after current step |
| Waiting for agent | Waiting for your agent | Show/copy handoff; resume through the attached agent |
| Waiting for approval | Needs your approval | Review exact action |
| Blocked | Needs a connection / file / capability | Resolve named condition |
| Needs reconciliation | Check whether this action completed | Inspect provider evidence |
| Paused | Paused | Continue after current access check |
| Succeeded | Complete | Open output |
| Failed | Stopped at [step] | Inspect safe recovery options |

A production “Resume” button never fabricates a native-tool result. If the inspector cannot send a request into the active client through a supported adapter, offer a copyable resume instruction with run ID. The mockup's “Simulate agent result” button is explicitly a demo action.

Before the user cancels an operation, show whether a remote action is already in flight. Cancellation copy says future work stops; it does not promise rollback.

## Visual tokens

| Token | Value / use |
|---|---|
| Canvas | `#FAF7F1`, warm paper |
| Surface | `#FFFDF9`, inputs, selected details, and technical responses |
| Sidebar | `#F0EBE2`, light linen navigation |
| Main text | `#29251F`, dark espresso |
| Muted text | `#62594E`, secondary copy |
| Border | `#CFC5B7`; row divider `#DED5C9` |
| Accent | `#9B3D28`, rust for primary actions and navigation marks |
| Accent hover | `#762C1D` |
| Waiting | Dark brown text on pale ochre; retain the state label |
| Error | Dark rust text on pale rose; retain the error label |
| Focus | `#8F442F`, visible 3px outline with 3px offset |

Use Georgia for the CookBook wordmark and large editorial headings. Use system sans-serif for functional text and system monospace for IDs, versions, and code details. Body text is 15–16px, labels at least 14px, and controls at least 42px high. Use 4/8/12/16/24/32px spacing. Rows and dividers carry structure; reserve contained surfaces for inputs, responses, and the workflow diagram. Control corners are restrained at 4px. The small CSS book-and-bookmark mark echoes the native app icon without replacing labels.

Use text labels/icons in addition to color. No external fonts, trackers, or CDN assets in the mockup. The production app should work offline for local inspection.

The native app icon uses two abstract ivory pages and a rust bookmark on an
espresso tile, with a matching flat geometric mark in the sidebar. Avoid physical
book textures, leather, bevels and shadows. Source PNGs at 16–1024px live in `apps/desktop/src-tauri/icons`;
`prepare.py` validates their dimensions and generates the seven-size macOS ICNS.
The same icon ships inside the app and installer. Updating the local development
package does not replace an existing installation or clear the user's icon cache.

## Accessibility and responsive behavior

Use semantic navigation, buttons, headings, tables/lists, and form labels. Every interactive control is keyboard reachable with a visible focus indicator. Meet at least 4.5:1 contrast for normal text and 3:1 for meaningful controls and focus indicators, following [W3C text contrast](https://www.w3.org/WAI/WCAG21/Understanding/contrast-minimum) and [non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html). Keep control targets at least 40px. Do not rely on hover to disclose a required action.

This refresh takes cues from Linear's [neutral chrome, alignment, and stronger text hierarchy](https://linear.app/now/how-we-redesigned-the-linear-ui) and [calmer layouts](https://linear.app/now/behind-the-latest-design-refresh). It applies those principles to a printed cookbook identity; it does not copy Linear's product styling or imply new runtime behavior.

Modal dialogs move focus inside, support Escape, constrain tab navigation, and restore focus to their trigger. Live status changes use a restrained polite announcement region.

At narrow widths the sidebar becomes a compact top navigation, two-column detail becomes one column, and rows wrap without losing actions. Target desktop widths from 1100px; remain usable at 390px. Respect reduced-motion preferences.

## Empty, failure, and offline states

An empty project explains that the agent can create its first process; it does not display fake success metrics. A stopped service offers diagnostics/reconnect. A disconnected client leaves saved requests visible. A missing credential displays a named connection, not a secret-entry request in chat.

Never show token savings before a benchmark exists. Actual measured step/call/byte counts can be displayed with their source; unknown usage remains unknown.

## Prototype scope and QA

The mockup must support navigation, process filtering, opening process detail, switching run focus, simulated pause/resume, viewing connection detail, and copying a handoff. All simulated state resets on page reload. No backend, credentials, file writes, provider calls, or real client connection exist.

Visually inspect desktop and narrow layouts, keyboard focus, dialog dismissal, and both waiting/completed states. Record mockup QA separately from runtime acceptance.
