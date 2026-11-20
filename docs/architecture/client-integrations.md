# Codex and Claude Code integration

Updated 2026-09-21. The shared MCP bridge, private assignment attachments and
managed user-level setup and agent-created projects are implemented. See [install and first project](../runbooks/first-project.md)
for users and the [scoped client runbook](../runbooks/client-integration.md) for
CLI commands. Protocol tests, successful setup and model-driven client acceptance
are distinct evidence; consult the [validation log](../logs/runtime-validation.md).

## MCP plus a short user skill

Setup installs one stdio MCP entry and the [canonical skill](../../integrations/skills/apr/SKILL.md).
The default is user-scoped: Codex uses `~/.codex/config.toml` and
`~/.agents/skills/apr/SKILL.md`; Claude Code uses `~/.claude.json` and
`~/.claude/skills/apr/SKILL.md`. Advanced project-only setup remains supported. The skill runs in the
current conversation so it can use that conversation's actual tools. Client
trust and tool approvals remain in force. Official references:
[Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli),
[Codex skills](https://learn.chatgpt.com/docs/build-skills),
[Claude MCP](https://code.claude.com/docs/en/mcp),
[Claude skills](https://code.claude.com/docs/en/skills).

The skill is triggered for setup, authoring, configuration and troubleshooting;
unchanged prepared execution uses the self-contained MCP contract directly. It
explains when to discover a recipe, author a file, validate,
preflight, execute and fulfill an observed handoff. The binary supplies exact
schemas and complete examples through `apr_discover` guide topics. Load only the
topic needed. MCP initialize instructions and tool descriptions point to the
same path even when a host does not load a skill. The installed skill is a concise
entry point; detailed credential, provider, paging, adapter and authoring contracts
remain in the on-demand guides. Known prepared runs need no repeated guide load.
No source repository checkout is assumed. See the [protocol](protocol.md) for topic names and response bounds.

The overview includes a use-case capability map rather than requiring authors to
infer the product from operation names. The on-demand `workflow_design` guide
maps observed work to execution owners, including direct model APIs, Jev data
nodes, parallel batches and caller-exclusive tools. It also covers migrating
existing project procedures: preserve quality/authority requirements while
replacing recipe-covered manual commands with invocation and receipt handling.
The audit remains advisory; authors account for model work inside each handoff,
and qualify the conversion separately from counting graph steps. Prepared reuse
does not reload this design material.

Initialization guidance reflects the attachment's actual run or author authority;
fixed run connections do not load project-bootstrap or authoring instructions.
Advertised schemas group only identical selector shapes and factor exact repeated
property definitions. The strict parser, accepted input set and tool annotations
stay unchanged. This reduces metadata; measured model savings remain separate.

Prepared reuse calls a known recipe directly. The start call briefly waits for a
completed result; the client consumes its output without a mandatory inspection.
Long or truncated results retain the durable run ID and explicit inspection path.
Setup/guide loading stays separate from repeated execution measurements; client
approvals and model inference remain client costs even for fast local recipes.
Clients advertising `result_view` may request `"compact"`: only a completed,
untruncated success omits step-progress metadata, preserving the complete business
output and receipt identity. Default/full output and scoped detailed inspection
remain available; errors, handoffs and unfinished or truncated results are unchanged.
The [protocol contract](protocol.md) defines the exact guards. Smaller metadata
and skill text are byte reductions, not evidence of a particular token or dollar
saving; model-driven evaluations must separate cached input from new input.

Ordinary tool calls are the portable knowledge path. MCP resources are
[application-controlled](https://modelcontextprotocol.io/specification/2026-07-28/server/resources);
we do not assume a client automatically injects them into its agent context.
Fixed attachments retain four ordinary tools and the author-only fifth tool. The
user connection adds one project bootstrap tool, `apr_project`.
Documentation is guidance; the runtime's semantic validation and scoped grants
remain authoritative. A guide's Jev feature report describes the actual running
service, not merely a newer MCP executable on disk. Older unknown metadata is
explicitly null. Optional feature inclusion is not an account or execution grant.

## Managed installation and authority

The default `apr integrate agent-preview/agent-apply/agent-uninstall` manages one
user-level connection with `apr --state-dir ABS mcp --projects`. The desktop offers
this without selecting a project. Plans show the exact paths and server entry;
owned hashes, anchored directory access, private snapshots and journals preserve
unrelated settings and reject symlinks, stale plans, conflicts and later edits.
By default they leave client trust and approval policy unchanged. Codex setup
also offers an explicit owner-reviewed `trust_apr_run` choice. It adds only
`[mcp_servers.apr.tools.apr_run] approval_mode = "approve"`; it does not change the
sandbox, server-wide defaults or any other tool. This policy covers **every
`apr_run` call through the managed connection**, including future connected
projects, rather than one recipe version. APR still enforces its workspace,
attachment, execution-grant, code-pin and revocation checks. Native code retains
its existing OS access. Stricter host policies can still require review.

The choice is off by default, visible in the native preview and bound to the
plan digest. Changing the client or choice invalidates that preview. The CLI
exposes it through `agent-preview --codex-trust-apr-run`; Claude setup rejects
that Codex-only option. Ownership checks hash nested configuration tables too,
so later user edits cannot be silently overwritten. Normal users connect once.

The agent then calls `apr_project prepare` with its current absolute project root.
The broker pins the directory identity and returns a random session challenge,
verification JSON and a five-minute deadline. The agent writes that file using
its own authorized filesystem tools and calls `connect`. Formatting and key order
may differ; a strict typed JSON decoder requires the exact challenge, nonce and
root strings and rejects missing, extra or duplicate fields. The broker
reads the proof through the pinned directory descriptor, rejecting symlinks,
hardlinks, nonregular files, changed roots, unsafe modes and mismatched/expired
proofs. The 4,096-byte cap also applies after reading. Parser failures retain the
same generic refusal. The filesystem root, home ancestors and runtime-state
overlaps are refused.
The agent removes the temporary proof afterwards. No project setup is required
from the user, and an MCP path argument alone never grants access.

The private per-root record serializes concurrent creation and reuses a persistent
assignment, separate from manually created assignments. It creates no execution
grants, credentials or schedules. `connect` returns an opaque session workspace
handle, never an attachment token or file path. Each ordinary global-mode call
**requires that handle**. The run/inspect/resume/manage schemas advertise a
required nonempty string; pre-connect guide discovery remains available. There is
no implicit current assignment: shared MCP
processes can serve different conversations without switching their authority.
Clients must not claim isolation between two chats in the same project; those
intentionally share its workspace. Session handles cannot be imported into a
second bridge. At most 32 pending proofs and 32 connected projects are retained;
bounded lost-response replay does not consume project slots.

The service still authenticates each call. Revoked/expired attachments and closed
assignments cannot be revived by reconnecting or replaying a challenge. Fresh
proof extends only an active, unexpired attachment's 30-day lease. After expiry,
owner recovery is necessary; agents must report this boundary, not create another
identity to circumvent it. Closing the assignment is the project-level revocation.
A client with arbitrary same-user native code is outside this interface boundary;
proof is evidence of existing client file authority, not a new OS sandbox.

Advanced fixed attachments use `apr mcp --attachment-file ABS`. They cannot call
`apr_project` or supply workspace overrides. Their project-only installer remains
`apr integrate preview/apply/update/doctor/uninstall`. Remove an old owned
project-level entry deliberately before switching a project to user setup;
client configuration precedence can otherwise keep the older attachment active.

Protocol setup and readiness do not establish a model-driven client journey.
Installing a package alone does not register a client or authorize a server.

## Lifecycle and compatibility

The independent service owns accepted work after the app or client closes.
Caller requests remain saved; they do not automatically wake a conversation.
Reconnect through the active attachment, inspect known runs/pending requests,
and submit observed output with stable identity. An interrupted external effect
requires evidence before retry. Native client tools do not transfer to another
client or to the runtime; separately approved API workers are explicit options.

Keep executable installation paths stable. After an app upgrade, existing service
processes can still run the old build. Finish pending work before explicitly
stopping the selected service and reopening. Cross-device source transfer is not
runtime migration; use private backup/restore, fresh attachments and reauthorization.

Qualification still includes trusted/untrusted projects, existing settings,
spaces/nested worktrees, missing executables, shared assignments, reconnect and
revocation, caller handoffs, and full interactive sessions in both clients.
Never bypass client policy to make a test pass. Future plugin packaging must keep
state/credentials outside replaceable caches and must not silently download and
execute an unpinned runtime.
