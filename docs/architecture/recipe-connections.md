# Recipe authoring, connections and trusted approval

## Desktop 0.2 / macOS–Linux extension — 2026-09-18

[Current execution and billing contract](macos-linux-completion.md) adds owner OAuth/refresh, MCP resources/prompts/bounded subscriptions and durable user/model continuations, optional approved text sampling workers, read-only Apps, durable whole-period billing, invoice matching, dispatch reservations, reviewed price refresh, and signed-update/release tooling. Earlier tools-only/no-OAuth/no-ledger/no-updater statements below describe the 0.1 baseline and are superseded for this extension. Native Windows remains excluded. Actual publisher signing, live accounts and complete native credential acceptance require separate evidence; implementation is not a signed stable release. Recipe authoring and cost analysis remain agent-driven.

Implementation extension, 2026-09-17. See the [validation log](../logs/runtime-validation.md) for execution evidence and the [desktop runbook](../runbooks/desktop.md) for the new Tauri surface. A recipe is the user-facing name for a versioned process; existing identifiers, manifest versions, databases, MCP tools and CLI commands retain compatibility.

## Author-time analysis, deterministic execution

During recipe creation the agent inventories each requested connection and analyzes user-approved request/response schemas, examples and pricing documentation. This is not a new background model or a model invocation for every tool result. The agent compiles the analysis into an explicit metering contract; the runtime validates and calculates it deterministically. Never include credentials in that analysis, recipe source, examples or tool arguments. Production payload capture and remote telemetry remain off; ordinary artifacts retain their existing privacy rules.

`cost.preview` accepts a public `connection`, representative `request`, and `response`. It makes no external call or grant and returns only observations, schema-match flags and schema digests. Use an initial mapping to obtain the digests, then attach `analysis` evidence to the final contract: recipe name, request and response schema digests, public pricing references, check date, and limitations. Altering a schema invalidates that analysis. Price and payload rules are frozen in the approved connection revision and subsequently in each accepted run. A new rate is not permission to silently change old run accounting.

Meter arithmetic supports `request:/json/pointer`, `response:/json/pointer`, a legacy `/json/pointer` response alias, and `constant:N`. Sources are reported on each observation. Request-based quantities describe planned/requested work, not proof of fulfilled provider work; response-based quantities are observed provider reports. Meter units remain explicit nonnegative integers: use milliseconds instead of fractional seconds. Unknown fields stay unknown. Do not infer tokens from character counts, include an output limit as actual generated tokens, or double-count cache/reasoning subtotals inside inclusive totals. Fixed rates are estimates for the reviewed dimensions, not universal billing or hard spending caps.

The request in this interface is the recipe's business payload. For `mcp.call`, it becomes the MCP call's `arguments`; reserved protocol metadata and handshake messages are not part of that schema. The response is the complete MCP tool result, so tool-specific usage can be under `response:/_meta/...` or `response:/structuredContent/...`. This layer does not infer charges for server handshakes, subscriptions, opaque credit conversions or unrelated external calls. Mark uncovered fees explicitly. See [metering](metering.md) for fixed-point arithmetic and retained ledger semantics.

## Requesting is not approving

An author-mode attachment may submit `apr_manage` action `connection.propose` with `name`, stable `request_key`, rationale, and a public Connection. Credentials must use `credential_ref: "pending"` or a genuinely unauthenticated operation. The agent cannot supply a usable credential reference, approval flag, grant, executable installation, or arbitrary owner dispatch. The proposal is stored in SQLite migration 11 with assignment identity, content digest, expected current binding revision and pending state. Schema validation is not security approval.

`apr_inspect` view `approvals` shows only the attached assignment's requests. View `connection_contract` with a binding `name` exposes its schemas, approved metering and selected tool name; it omits credentials and local command arguments. View `readiness` with `process_ref` conservatively scans reachable recipe versions, counting caller dependencies, missing grants and unpriced connections. It is a bounded static check, not a live credential/endpoint health test, uptime guarantee or promise of completion. Oversized views report truncation rather than readiness.

The owner reviews the exact destination or local command, tool name, schemas, auth type, pricing rules and requested assignment in a separately privileged local window or explicit owner CLI command. Native MCP commands and their installed dependencies have the owner's OS access; a token injected into one explicitly named environment slot is readable by that trusted server. This is not APR's protected-script mode or an OS sandbox. No package is downloaded or installed by discovery or execution.

Approval compares the reviewed digest and current binding revision, expires pending requests after seven days, requires an active assignment, and puts a provided key into the OS credential backend. Grant revision and approval receipt commit in one SQLite transaction. Replayed approval cannot rotate a key or restore a revoked grant. A stale request requires a new proposal. Existing runs require an explicit rebind; unresolved effects must first be reconciled. Declining creates no grant. Restored databases keep revoked grants and invalidated attachments under the existing backup rules.

Owner native control is not available through the generic inspector, MCP dispatcher, or a recipe. These are application boundaries in a trusted single-user environment; unrestricted same-user shell/native code can bypass application-level assumptions. See [security](security.md).

## Outbound MCP

The existing `apr-mcp` crate remains APR's inbound agent interface. The separate outbound adapter lives behind `apr-connectors::mcp` and the existing `connector` step kind, using `operation: "mcp.call"`. A connection pins one tool contract; the recipe supplies arguments, not a replacement endpoint/tool/credential. A direct HTTP connection cannot masquerade as MCP or vice versa.

Supported tool transports are public HTTPS Streamable HTTP and explicitly trusted local stdio. Owner metadata discovery can automatically select 2025-03-26, 2025-06-18, 2025-11-25 or 2026-07-28; the resulting proposed and approved tool still pins one concrete version. Advanced setup supports explicit selection. Legacy versions initialize and acknowledge initialization, with a session header when supplied. The 2026 version supplies per-request metadata and routing headers and accepts only complete results. There is no implicit protocol downgrade during execution. Discovery probes modern metadata first and can fall back to a fresh legacy session within one shared deadline; authentication, throttling, malformed responses and unavailable servers do not trigger a tool retry. HTTP accepts bounded JSON or SSE responses; arbitrary redirects, inherited proxies and private network destinations remain rejected. Long-lived server push subscriptions and legacy HTTP+SSE transport are not supported.

Discovery lists bounded tool pages without issuing `tools/call`. Selected input/output contracts are pinned. Execution rechecks the advertised contract before calling; schema/name drift blocks the call and requires reviewed reconfiguration. Tool annotations do not grant read/write authority. Only tools are advertised as usable: no sampling, elicitation, arbitrary roots, resource links, nested tool delegation or embedded Apps execution. Unsupported interaction requests stop conservatively; they never authorize a model call or fabricate a result. Authentication accepts no-auth, owner-supplied bearer tokens or approved API-key headers. OAuth browser sign-in, discovery and token refresh are not implemented by this adapter; a token requiring an interactive flow must be provisioned separately and rotated explicitly.

Stdio uses a reviewed absolute canonical executable and digest, exact argv, a fresh private working directory, a minimal environment, bounded I/O/deadlines and process-group cleanup. Executable hashing does not freeze interpreter libraries or argument-referenced files; those dependencies are explicitly trusted, not sandboxed. A local server can perform effects during startup, so ambiguous failures after startup remain uncertain even when a tool was not called.

Each actual recipe connector attempt uses the existing persisted intent, frozen binding revision, operation budgets, usage observation and reconciliation path. Replayed starts do not execute again. Tool errors, timeouts and lost acknowledgements may follow a partial external write and must not be blindly retried. Metadata exchanges are not individually counted as recipe tool attempts. HTTP cancellation cannot unsend a request; it remains bounded by its timeout. Credentials and protocol session IDs are rejected if echoed in decoded results. No response is trusted HTML.

## Cookbook language

Use Recipes for reusable workflows, Recipe library for the catalog, Ingredients for inputs, Kitchen for the overview, and Pantry · Connections for configured services. Keep Steps, Runs, Schedules, Results, Approvals, Permissions, Costs, Failures and Reconciliation literal. Avoid disguising destructive actions as culinary metaphors. Technical `process_*` fields and immutable IDs are unchanged.

## Reusable tools and execution review — 2026-09-21

[Connection library](connection-library.md) extends workspace approvals with explicit All projects publication and stored-key reuse for the exact destination/auth. Agents can discover and bind published contracts; this is not inherited client credential access. Legacy grants are not automatically shared. [Authoring review](recipe-authoring.md) makes the runtime/caller boundary an explicit authoring requirement.

## Installed adapter preparation — 2026-09-22

`apr adapter prepare` creates a new immutable local bundle from a reviewed JSON
spec; `apr adapter check` checks readiness without launching its target. The
installed `adapters` MCP guide carries the full synthetic example and the actual
CLI path. Templates are embedded in the binary. No source checkout, package
installation, account or grant is created by preparation.

A stdlib Python wrapper exposes one fixed CLI as a bounded JSON-in/JSON-out tool.
Its launcher pins source/spec bytes, interpreter and declared executable/dependency
files. It clears ambient environment and uses private working state. One explicitly
named protected credential may be injected through the existing stdio boundary.
The connection proposal still needs owner review before recipe execution. Native
code retains OS privileges; declared hashes do not discover transitive dependencies
or constitute a sandbox. Updates require a new bundle and reviewed binding.
