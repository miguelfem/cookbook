# Connect an agent to CookBook

Updated 2026-09-21. The normal path is the desktop's one-time **Connect your agent**
flow in [install and first project](first-project.md). Projects are then created by
the agent. Building or testing APR does not edit real client settings.

## User-level connection (CLI alternative)

Use the installed executable at its permanent path and the app's private runtime
state directory, not a new store inside a project. Replace the example paths:

```sh
APR_BIN="/absolute/path/to/apr"
APR_STATE="/absolute/path/to/private-apr-state"
"$APR_BIN" --state-dir "$APR_STATE" integrate agent-preview \
  --client codex --output-file "$APR_STATE/plans/agent-codex.json"
"$APR_BIN" --state-dir "$APR_STATE" integrate agent-apply \
  --plan-file "$APR_STATE/plans/agent-codex.json"
```

For the explicit Codex recipe-trust option, add `--codex-trust-apr-run` to
`agent-preview`, review the generated plan, then apply that exact plan. It sets
only `mcp_servers.apr.tools.apr_run.approval_mode = "approve"`. This affects all
recipe runs through the connection, including future projects; APR's workspace
permissions, grants, pins and revocation still apply. It does not relax the
sandbox or other tool approvals. Omit the flag to keep normal client review.
This option is Codex-only. Agents must not install it as a workaround for a denial.

Use `--client claude` for Claude Code. Preview defaults to your home; `--home` is
available for disposable-home tests. It installs `apr --state-dir ABS mcp
--projects` in user config plus the user skill. No project root or attachment is
needed. Review paths and normal client trust. To remove only owned material:

```sh
"$APR_BIN" --state-dir "$APR_STATE" integrate agent-uninstall --client codex
```

Ask the agent to use CookBook in its current project. `apr_project prepare`
returns a temporary proof file for the agent's authorized filesystem tools to
write; `connect` returns the workspace handle required on every subsequent APR
call. The agent removes the proof file. Reconnect after bridge restart; the same
verified folder reuses its saved workspace. Read `apr_discover guide=overview`
for installed authoring/JeV guidance. Before connection, public guides contain
unknown runtime capability metadata; after connection they report the actual
service. No secrets or source checkout are needed.

Existing project-level config may override user config. Remove its owned entry
with the legacy uninstall command below, preserving the saved project data.
Do not overwrite an unmanaged entry or bypass policy. Revoked/expired/closed
workspaces need owner recovery and cannot be silently recreated by an agent.

Real subprocess tests cover the bootstrap proof, two project scopes, authoring,
execution and reconnecting. These do not establish a complete model-driven
Codex or Claude journey; see [runtime validation](../logs/runtime-validation.md).
The rest of this page documents the **advanced fixed-attachment alternative**.

## Create an explicit attachment

Build with the pinned Rust toolchain, then choose absolute paths for the executable, private state directory and project. The state directory can be outside the project. Replace the example values before running these commands:

```sh
cargo build --locked --workspace
APR_BIN="/absolute/path/to/cookbook-apr/target/debug/apr"
APR_STATE="/absolute/path/to/private-apr-state"
APR_PROJECT="/absolute/path/to/project"
"$APR_BIN" --state-dir "$APR_STATE" project attach "$APR_PROJECT"
```

Copy the returned `project_id` into the next command, then use the returned `assignment_id`:

```sh
"$APR_BIN" --state-dir "$APR_STATE" assignment create \
  --project PROJECT_ID --name "Local review"
"$APR_BIN" --state-dir "$APR_STATE" attachment create \
  --assignment ASSIGNMENT_ID --mode author \
  --output-file "$APR_STATE/attachments/local-review.json"
```

The attachment output must remain **inside its runtime state directory**. The CLI creates missing parents with mode `0700`, writes a new file with mode `0600`, rejects symlink parents and never overwrites an existing file. It prints metadata and the file path, never the bearer token. APR excludes the entire state directory from managed project imports so an author attachment cannot import another attachment's credentials. Do not put attachment contents in prompts, source control or shared client configuration.

An attachment binds one assignment and one mode. `author` exposes process authoring, ordinary assignment configuration and project-file artifact import. `run` exposes the four ordinary tools. Both modes can start and control authorized runs. Create a second attachment for the same assignment when switching clients; create a separate assignment for independent work.

The local owner CLI controls attachments. A fixed-attachment MCP connection cannot create assignments, grant itself author mode or revoke another attachment. Only the explicitly installed user-level bootstrap above can create a project after fresh filesystem proof. The service rechecks saved authority on every call; editing a descriptor's `mode` does not grant author rights. Same-user native file access and arbitrary host programs remain outside APR's interface boundary.

## Configure Codex or Claude Code

Managed setup adds one `apr` MCP server and the APR skill to an existing local project. It preserves unrelated configuration values and TOML comments. Create a private attachment above, then preview and save the plan:

```sh
"$APR_BIN" --state-dir "$APR_STATE" integrate preview \
  --client codex --project "$APR_PROJECT" \
  --attachment-file "$APR_STATE/attachments/local-review.json" \
  --output-file "$APR_STATE/plans/codex.json"
"$APR_BIN" --state-dir "$APR_STATE" integrate apply \
  --plan-file "$APR_STATE/plans/codex.json"
"$APR_BIN" --state-dir "$APR_STATE" integrate doctor \
  --client codex --project "$APR_PROJECT"
```

Use `--client claude` for Claude Code and choose a separate preview filename. The running APR binary supplies the absolute executable path; `--executable PATH` can select an installed copy explicitly. Preview reads attachment metadata only and prints no attachment or existing settings contents. An optional plan file must be new and beneath the private state directory, with mode `0600`. Apply rejects a stale preview. Replaying a completed plan is a no-op while its owned entry and skill remain unchanged.

APR refuses existing unmanaged `apr` entries/skills, symlinked config paths, unsafe permissions and tracked client configuration files. Keep the generated device-specific settings local; APR does not change Git ignore rules, global client settings or approval policy. If a project already tracks `.mcp.json` or `.codex/config.toml`, resolve that repository policy before managed setup; the manual template remains available for deliberate alternative arrangements.

`doctor` reports saved configuration, a bounded client `--version` probe, and a real authenticated APR service check. It may start the APR supervisor. `service:"ready"` means APR answered through the attachment. Client project trust remains `unknown` and the actual client connection remains `unverified` until the normal client flow is exercised. Missing or broken client executables are reported separately. `--client-executable PATH` selects the version probe explicitly.

To switch an installed integration to another attachment, use `integrate update` with the same `--client`, `--project` and new `--attachment-file`. It preserves unrelated settings and refuses edits to the APR-owned entry or skill. Private ownership records, before/after snapshots and an operation journal live under `STATE/integrations/`. Retry the same `apply` plan or the same `update` request after an interruption. Recovery accepts exact saved before/after states; a later user edit produces a conflict instead of an overwrite. APR operations serialize through a local lock; arbitrary same-user programs are outside this boundary.

### Manual templates

The inactive [Codex template](../../integrations/codex/config.toml.template) belongs in a trusted project's `.codex/config.toml`, merged with existing settings. The [Claude template](../../integrations/claude/mcp.json.template) belongs in `.mcp.json`, merged under its existing `mcpServers` object. Replace both absolute-path placeholders. Keep this device-specific configuration local and preserve unrelated entries; adding the attachment **path** does not require exposing its contents.

Both configurations launch the same implemented command:

```sh
/absolute/path/to/apr mcp --attachment-file /absolute/path/to/private-apr-state/attachments/local-review.json
```

There is no `--stdio` flag: this command already reserves stdin/stdout for MCP. The first service call starts the independent supervisor on demand. The MCP process can exit while saved work continues. `apr --state-dir PATH service` runs that supervisor in the foreground for local diagnostics.

Codex documents trusted project configuration and local stdio servers; its `mcp add` command is not a project-scoped installer. Claude documents project MCP configuration and client-controlled trust/approval. Follow each client's normal policy rather than changing global permissions. Official references: [Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli), [Claude MCP](https://code.claude.com/docs/en/mcp).

Managed setup installs the [APR skill](../../integrations/skills/apr/SKILL.md) in `.agents/skills/apr/SKILL.md` for Codex or `.claude/skills/apr/SKILL.md` for Claude. Manual setup can copy the same inactive body with the user's chosen repository policy. The skill runs in the current conversation; it does not fork a new context or acquire access to another client's tools. See [Codex skills](https://learn.chatgpt.com/docs/build-skills) and [Claude skills](https://code.claude.com/docs/en/skills).

The skill is intentionally short. `apr_discover {"guide":"overview"}` lists focused guides; `authoring`, `schema`, `starter`, `routing` and `jev` expose the installed contract and full examples on demand. Additional topics cover connections, handoffs, schedules and legacy schema 0.1. The MCP initialize instructions and discovery description expose this path even without skill loading. Guide calls authenticate through `service.info`, so revoked/expired attachments remain blocked. Their `runtime.features.jev` comes from the running service, not the bridge binary; `null` means an older service cannot report it. Inclusion does not prove account setup or authorization. Do not combine `guide` with search/schema selectors; each guide fits a complete 64 KiB detail response or fails closed.

Two chats using the same client configuration share the same attachment and assignment. A chat title, working directory or skill invocation does not select new authority. Separate tasks require separately selected client configurations/attachments; automatic per-chat isolation is not implemented.

## Use the tools

| Tool | Current behavior |
| --- | --- |
| `apr_discover` | Find up to 20 permitted recipe summaries; request one recipe schema with `process_ref` and `detail:"schema"`, or installed authoring knowledge with `guide:"overview"`. |
| `apr_run` | Preflight or start a registered immutable process using an input, allowed configuration overrides and a stable request key. |
| `apr_inspect` | Inspect assignment run history, a run, frozen configuration, pending requests, bounded events or an authorized artifact preview. |
| `apr_resume` | Submit a complete caller result, pause/continue saved work, renew a pending handoff or cancel a run. |
| `apr_manage` | Author mode only: validate/test/register/promote a process, read/set ordinary configuration or import a project-relative artifact. |

Registered processes belong to this assignment's catalog. The runtime supports local builtins and explicit caller handoffs, plus owner-approved pinned Python and named JSON connections. Conditional routing and child process calls use the admitted v0.2 contract. Native execution remains trusted host code; protected credentials and execution grants require the [local owner controls](owner-controls.md). A model or process cannot create its own authority. The optional Jev adapter is disabled by default and requires an explicit build feature and configured binding; installing a client integration never enables or calls it. See [execution architecture](../architecture/execution.md) and the current [validation log](../logs/runtime-validation.md) for the implementation and verification boundaries.

Assignment configuration is ordinary nonsecret JSON. `config.get`/`config.set` operate through author mode, while `config_overrides` on a start remains subject to its process schema. The run freezes its resolved configuration at acceptance; subsequent edits affect future runs. Promotion requires an explicit same-project grant from the owner and does not transfer artifact access or execution grants.

For the included local-review process, attach this repository as the project, then:

1. Call `apr_manage` with `{"action":"process.validate","path":"examples/local-review/process.json"}`, then `process.register` for that path. Save its returned `process_ref`.
2. Import `examples/local-review/source.txt` with `{"action":"artifact.import","path":"examples/local-review/source.txt","media_type":"text/plain"}`. Save the returned artifact reference.
3. Discover the registered schema. Preflight, then start with `input:{"record":{"title":"Local review"},"sources":[ARTIFACT_REFERENCE]}` and a stable `request_key`. Reuse the same key, input and configuration overrides after a lost response.
4. Inspect the saved run. At `waiting_for_agent`, read the request, review its actual inputs using the caller's authorized tools and submit an observed result conforming to its result schema.

This fixture packages either boolean answer; it does not implement approval routing. The [standalone walkthrough](local-cli.md) explains its output artifacts.

## Complete a saved caller handoff

Ordinary tool results are limited to 8 KiB and use structured content without a second copy of the JSON as text. If a response is truncated, request `apr_inspect` with `{"view":"run","run_id":"RUN_ID","detail":true}` for up to 64 KiB. A handoff is returned completely or omitted; never execute a partial request. Large source/output data should remain artifact references. Artifact previews are bounded and do not export binary files to client tool storage.

For `apr_resume` submission, copy `protocol_version`, `run_id`, `request_id`, `step_instance_id`, `generation` and `continuation_token` from the saved envelope/request, set `action:"submit"`, choose a stable `submission_key`, and include the actual `output` and its `provenance`. Keep the same semantic submission when retrying: the saved receipt survives a lost acknowledgement. Never fabricate a native tool result or silently replace a missing tool with a paid provider API.

MCP action JSON is limited to 64 KiB; an input wire line over 1 MiB closes the bridge before parsing. Catalog descriptors and variable handoff fields are limited to 48 KiB so their complete identities fit in a detail response. Oversized static contracts fail validation/registration; oversized resolved handoffs fail before publishing a claim. A maximum-detail response that still cannot fit returns `LIMIT_EXCEEDED`. Refactor the process to use artifact references rather than repeatedly requesting the same oversized view.

APR does not inherit Codex or Claude's image, browser or other native tools. The caller can use its own available tools, then explicitly return the result. Importing a new artifact requires an author attachment for the same assignment; a run-only attachment cannot bypass that restriction. If the needed capability is absent, leave the request pending and report the blocker.

## Reconnect, revoke and remove

After reconnecting through the same active attachment, inspect the saved `run_id` or use the pending view. Closing the MCP connection does not cancel a run. `apr_resume` with `action:"renew"` rotates a pending handoff's generation/token and expiry; the previous unaccepted token becomes stale. Accepted receipts remain replayable.

The trusted local owner can revoke one attachment or close the assignment:

```sh
"$APR_BIN" --state-dir "$APR_STATE" attachment revoke ATTACHMENT_ID
"$APR_BIN" --state-dir "$APR_STATE" assignment close ASSIGNMENT_ID
```

Revocation blocks that attachment's subsequent requests. Assignment closure revokes its attachments and cancels its unfinished runs while preserving saved history. Removing a client configuration or skill only disconnects that client; it does not revoke authority or delete runtime state. Revoke before removing an attachment file.

Remove a managed integration with `integrate uninstall --client codex --project "$APR_PROJECT"` (or `--client claude`). Uninstall removes only the unchanged owned entry and skill. Later unrelated settings remain, and edited owned material is reported as a conflict. APR does not restore a whole old configuration over newer user changes. Private backups and runtime state are retained. Repeat an interrupted uninstall to finish its journal. Cross-device source transfer does not migrate an active run or installed device paths.

### Client approval and JSON compatibility

The bundled Codex CLI 0.154.0-alpha.6.2 was exercised in an ephemeral model session with unsaved MCP overrides. It registered and discovered the synthetic process; its noninteractive never-approval policy refused `apr_run`, so that attempt created no run. Use the client's normal interactive approval flow for the complete journey. APR grants do not override client approval policy, and APR does not relax it during installation.

The adapter advertises explicit JSON-value types rather than a boolean `true` schema, which Codex otherwise normalizes to text. Process inputs and caller outputs remain raw JSON values: send an object as an object and a string as a string. Do not double-encode JSON. The real protocol tests cover nested objects/arrays and literal JSON-looking strings.
