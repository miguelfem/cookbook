# Install CookBook and try your first project

CookBook holds reusable recipes for your agent. A recipe saves the repeatable parts of a task, can use approved tools or external model APIs, and can hand a step back to your agent. You describe the task; the agent writes the recipe. Choose a model API for cost, speed or quality through an approved connection; see the [installed connection guide](../../integrations/knowledge/connections.md).

## 1. Install the app

Use a package built for your computer. On an Apple Silicon Mac, open the `aarch64` DMG and drag **CookBook APR** to **Applications**. Eject the disk image and open the copy in Applications. Install it in this permanent location **before** connecting a project: the connection points to the executable inside the app. Rust, Node and the CookBook source repository are not required to use this package. Individual recipe tools may have their own prerequisites.

Local 0.2 packages are development previews, not notarized public releases. If macOS blocks the app after an open attempt, use its per-app **System Settings → Privacy & Security → Open Anyway** approval only for a package you trust. Follow [Apple's instructions](https://support.apple.com/en-us/102445); do not disable Gatekeeper. Production signing/notarization remains a release gate.

There are two explicit build choices. The standard package needs no inference account and excludes Jev. A package marked **Jev enabled** includes its optional adapter and can also run ordinary recipes. It does not contain credentials, enable paid calls or connect an account. Both use the same app identity and store: install one choice, not two copies simultaneously. Existing accepted work should finish before you switch builds; quitting the app alone does not stop the older service.

## 2. Connect your agent once

1. In CookBook, open **Settings & access → Connect your agent**.
2. Choose **Codex** or **Claude Code**, click **Preview connection**, then **Connect agent**.
3. Open a new task in any project in that agent. Accept the client's normal trust/MCP prompts; restart the client if it caches its configuration.

That is the whole user setup. You do not create a CookBook project, choose an assignment, copy files or run terminal commands. The agent creates or reconnects the CookBook workspace in the project it is working on. CookBook adds one user-level MCP connection and a short skill, preserving unrelated settings and leaving approval policy unchanged by default. Existing unmanaged `apr` entries or edited owned skills are reported as conflicts instead of overwritten.

Codex also offers **Trust recipe runs in Codex** before preview. This optional
choice removes Codex's ordinary per-call approval for recipe execution in projects
using that user-level connection. CookBook still checks workspace access and execution grants;
other agent tools and the Codex sandbox keep their existing rules. Review that
scope before connecting. Changed or revoked CookBook permissions still block
execution. Host policies may impose additional review.

Older project-only `apr` settings can take precedence over the new user connection. If you previously configured this exact test project that way, remove its managed integration through the [client runbook](client-integration.md) before using the new flow. A fresh project needs no such migration.

Installing settings is not proof that the current conversation loaded its tools. The first prompt below checks that from the agent.

## 3. Ask for a first recipe

Paste this into the new project task:

> Use CookBook for this project. First check that its MCP tools are available, discover existing recipes, and read the installed authoring guidance. Create and run the account-free save-note starter with the note “My first CookBook recipe”. Show me the saved result and where the recipe source lives. Do not connect a paid service or change permissions.

The agent should find `apr_project`, prove its existing write access using a temporary verification file, connect the project and remove the file. It then includes the returned workspace on every call to `apr_discover`/the other tools, fetches `overview`, `authoring`, `schema` and `starter` as needed, writes `recipes/save-note/process.json`, validates/registers it, preflights, and runs it. To see the recipe in CookBook, use **Refresh projects → Open workspace** on the connection page. This viewing step is optional; agent execution does not depend on it. The result is a saved JSON artifact. You can then describe a useful repeated task from your actual project.

For example:

> Make a CookBook recipe that prepares a short report from this project's input files. Reuse approved tools where useful. Keep repeatable steps in the recipe and return any judgment call to you for review. Explain any access or paid service needed before requesting it.

Recipes live in a persistent workspace for the verified project folder. Two chats in the same project share its recipes. Different project folders receive separate workspaces, and every tool call names its workspace explicitly. Credentials can be reused through protected bindings, but each assignment still needs approval for the operations it will use.

## 4. Try Jev when you need semantic routing

Start after the ordinary recipe succeeds:

> Read CookBook's installed Jev guide and report whether the running service includes Jev. Draft a recipe with a bounded Choice question, declared routes and a caller-review route for uncertain answers. Validate it without executing a paid call. Reuse an approved connection if available; otherwise prepare the access request for my review.

`runtime.features.jev` reports the **running service**: `false` means omitted, `true` means included, and `null` means an older service did not report the feature. Inclusion, an approved connection and live account health are separate checks. After an intentional build upgrade, finish pending work and use **Settings & access → Advanced → Runtime controls → Stop service**, then reopen the workspace to start the new bundled service.

The owner approves the exact proposed operation and supplies credentials only in trusted CookBook settings, never in chat or recipe files. Jev provides structured observations; declared recipe rules select the next step. Confidence is not a calibrated probability or a guarantee. Uncertain cases can wait for the current agent. Live Jev quality, charges and provider compatibility require a separately authorized account test.

## What the agent receives

| Layer | What it does |
| --- | --- |
| User skill | Explains when to use CookBook, the safe workflow, and which guide to request. Codex: `~/.agents/skills/apr/SKILL.md`; Claude Code: `~/.claude/skills/apr/SKILL.md`. |
| MCP initialization and tool descriptions | Explain the product and point to authoring guides even if the host does not load skills. |
| `apr_discover` guides | Return the installed manifest schemas, complete ordinary/routing/Jev examples and focused instructions on demand. No source checkout is needed. |
| Runtime validation and grants | Enforce accepted manifests, assignment scope, execution limits and owner-approved access. Guidance alone never grants authority. |

The user connection adds `apr_project` for project creation; the five recipe tools handle discovery, authoring and execution. Legacy fixed attachments keep their existing four/five tools. Guides use ordinary MCP tool calls so hosts do not need a resource browser or automatic resource injection. CookBook does not inherit Codex/Claude native tools. A caller handoff uses the tools actually available in that conversation and submits the observed result explicitly.

For manual setup, diagnostics, updates and removal, use the [client runbook](client-integration.md). For development builds and disposable validation, use the [desktop runbook](desktop.md).

## Reusable tools and execution review — 2026-09-21

Existing accounts remain available for separately reviewed workspace operations. To share a particular earlier Jev or service tool from **Settings & access → Tools & accounts**. Open its existing workspace, load workspace connections, review the contract, and choose **Make available to all projects**. No key re-entry is needed. New access requests offer **All projects** or **Only this workspace**, plus compatible stored accounts. Ask your agent to inspect `connection_library` and reuse approved tools before requesting new credentials. Stop the previous runtime through Runtime controls before replacing a development app; this source update does not replace an installed copy.

For browser OAuth, independent saved accounts, and explicit API exceptions, follow [Connecting services](connecting-services.md).
