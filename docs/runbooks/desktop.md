# CookBook desktop and owner setup

## No GitHub Actions required

Use the [local verification and packaging guide](local-verification.md) to run the full checks and build macOS/Linux installers on your own machine. Hosted CI is optional; publisher signing and live-account acceptance still require actual credentials and platform evidence.

## Version 0.2.0 — macOS/Linux

The [current architecture](../architecture/macos-linux-completion.md) defines OAuth, outbound MCP resources/prompts/continuations and update support. This is a development preview unless the separate publisher release procedure has actually run. Local certificate signing below does not make it a notarized publisher release.

Follow [install and first project](first-project.md) for the normal user flow. Install the package in its permanent location, open **Settings & access → Connect your agent**, and preview/apply the one-time Codex or Claude connection. The agent creates projects itself. This does not bypass that client's own trust/approval policy. The service remains independent of the desktop window.

For OAuth, use metadata discovery with a provider-supported public client ID. Review the exact issuer, resource audience, scopes and requested operation, then choose browser authorization. Sign in on the provider's page, not in a recipe/chat/API-key field. Codes/tokens return directly to native storage. A failed refresh or locked vault blocks execution and needs reconnection.

Use **Billing and budgets** to inspect a complete time interval, set an explicit dispatch policy and import a reviewed normalized invoice. Amounts ending in `_nanos` are decimal strings in billionths of the stated currency; the display does not convert currencies or sum estimates with provider/invoice amounts. A maximum reservation is an owner assumption that must actually bound the configured request. Price-source refresh creates reviewable proposals, not automatic changes to existing runs.

**MCP requests** separates actual user elicitation from attached-agent sampling. An explicitly approved API sampling profile can execute independently; it must refer to another named connection requested by the recipe. Uncertain model attempts show their exact intent and evidence-based reconciliation controls. Do not submit another generated answer to disguise an unknown charge.

**MCP App preview** accepts the run, completed UI-resource step and related tool step. Only a matching approved `ui://` resource is shown, in the self-contained read-only profile. External network and host tool calls are disabled. Approve sharing the selected tool result with that App before opening it.

**Updates** requires a publisher key/feed compiled into a release. Previews report that no feed is configured. Optional daily checks never install silently. Resolve pending runs before installing; a failed update leaves execution paused, with explicit maintenance recovery in setup. AppImage supports in-app replacement; Debian packages should be updated through the package manager. Never disable system security globally to install a development build.

### Build / release

Use the local [verification and packaging guide](local-verification.md). From the repository root:

```sh
.venv/bin/python tools/build_desktop_local.py --offline
# Explicit optional build; ordinary builds exclude Jev:
.venv/bin/python tools/build_desktop_local.py --offline --with-jev
```

The builder compiles matching native owner and sidecar runtimes. `--with-jev` selects the Cargo feature and sidecar preparation together; `packages.json` records the selection. It does not configure any account. Advanced direct Tauri builds must pair `COOKBOOK_INCLUDE_JEV=1` with `--features jev`; use the local builder to avoid mismatching the two binaries. Rust, Node/npm and platform prerequisites are development dependencies; packaged app users do not need them. Recipe-specific workers/servers may require their own dependencies.

No GitHub Actions, upload, installation or client setup is part of this procedure. Only source assets and executable dependencies enter packages; mutable state, credentials and grants stay outside them. Source and package checksums are saved with the logs.

On macOS, repeated ad hoc builds can acquire a different code identity and ask for Documents access again. To give local builds a certificate-backed identity, explicitly select an existing development certificate:

```sh
.venv/bin/python tools/build_desktop_local.py --offline --with-jev \
  --development-sign-identity "Apple Development: YOUR EXISTING IDENTITY"
```

This signs the app and its runtime sidecar, verifies the bundle, and records `development_signed` in `packages.json`. The private key remains in the system's signing store; macOS may require consent to use it. Publisher secrets and notarization credentials are stripped from every step. This option is macOS-only and cannot be combined with `--signed`. Use the same certificate and app identifiers for subsequent local updates. An initial Documents consent may still be necessary, and the app must respect it; do not change the privacy database or grant broad disk access as a workaround. Keep disposable test projects outside Documents to avoid unrelated folder prompts.

If the checkout is in cloud-synced Documents and signing reports Finder information or resource forks in the generated bundle, use `--target-dir "$HOME/Library/Caches/CookBook/desktop-local"` to build in a nonsynced location. This changes only generated build output, not source, runtime state or OS privacy settings. Keep the normal code-signature verification gate.

Publisher releases use the separate explicit `--signed` option. They require the owner-selected license, `TAURI_SIGNING_PRIVATE_KEY`, `COOKBOOK_UPDATE_PUBLIC_KEY` and credential-free HTTPS `COOKBOOK_UPDATE_ENDPOINT`; macOS also needs an existing `APPLE_SIGNING_IDENTITY` and notarization variables `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`. Keep secrets in a local secret mechanism, never chat or Git. A successful local development build is not publisher signing/notarization evidence.

Generate a release feed only after signing, using `tools/release_manifest.py`; hosting it and publishing packages remain separately authorized owner operations. No feed is published by the local builder.

## First launch

Choose a project directory and assignment in **Settings & access**. The app manages a private local runtime store under its application-data directory; it does not silently adopt or modify another CLI store. Select Codex or Claude Code and preview the managed client settings patch. Apply only the current reviewed preview; unrelated settings remain protected by the existing installer. Client trust/approval remains separate. Diagnostics distinguish configured, connected and unverified client behavior.

The main Home/Recipes window holds assignment authority only. It can inspect runs, ingredients, graphs, retained analytics, scoped connection contracts and pending requests. The separate bundled setup window owns credential entry, discovery and approval. Neither window renders remote markup or loads CDN scripts. Native command capabilities are explicitly allowlisted by window; the inspector has no owner command, shell, arbitrary file or window-creation permission. Attachment tokens remain native and are never delivered to the WebView.

### Disposable native validation

Launch the newly built application directly from its build/evidence directory;
copying it into Applications or replacing an existing installation is unnecessary.
The desktop accepts `--state-dir /absolute/private/runtime`. An explicit directory
must already be owned by the current user with no group/other permissions, or be
a new directory under such a private parent. Invalid, relative and symlink roots
fail without falling back to the normal store or changing existing permissions.
The selection, attachments, grants, runs and update preferences stay in this
store. Explicit-state windows use nonpersistent WebView storage. This is store
isolation, not a sandbox or an alternative credential backend: client setup,
native workers, external connections and explicit legacy-key imports retain their normal
approval requirements.

On macOS, use the exact fresh `.app` path returned by the local build report:

```sh
smoke_root=$(mktemp -d /tmp/cookbook-desktop-smoke.XXXXXX)
mkdir "$smoke_root/project"
"/absolute/fresh/CookBook APR.app/Contents/MacOS/cookbook-desktop" \
  --state-dir "$smoke_root/runtime"
```

Use the disposable `project` path in **Settings & access**, create an assignment,
and leave **Apply reviewed settings** unused when testing only the native app.
Use synthetic fixtures and unauthenticated local fixtures for recipe/approval
checks. The bundled sidecar is `Contents/MacOS/apr`; any fixture CLI command must
use the same explicit `--state-dir`. Do not copy attachment contents into logs or
screenshots. Linux launches use the newly built executable or AppImage with the
same option. A default launch without this option uses the normal application
store.

Record both the app path and the disposable root. Reopen by running the same
command with the same `--state-dir` value; a new `mktemp` invocation creates a
different first-launch test. Confirm the selected assignment, saved recipe and
run identity, update preferences and completed artifacts after reopening. To
verify independent work, start a bounded synthetic run, observe it executing,
quit the desktop before completion, and verify the original service process and
run complete while the UI is closed before reopening. A replacement service
started by an inspection command is not proof that the original survived. The
trusted **Stop service** control can stop only this selected disposable service
when the test ends. Retain private evidence until verification is recorded.

## Connect a service

An agent normally prepares the public connection and cost contract while authoring a recipe. It submits `connection.propose`; the owner reviews the pending request. The generic JSON proposal form exists for advanced setup, not as a replacement for agent authoring. API keys or bearer tokens are entered only in the trusted password control and saved automatically in the runtime’s private `credentials.toml` (`0600`, parent `0700`). The file is plaintext and excluded from CookBook backups. Normal calls do not access Keychain. Existing users can choose **Tools & accounts → Import existing saved keys** once; OS consent may be needed to read legacy entries. An import preserves accounts, grants, and old entries for rollback.

For MCP, choose the exact HTTPS endpoint or local executable/argv and the protocol version. Before native discovery, explicitly acknowledge its full OS access and token access. Discovery does not install packages or invoke a tool. Select a discovered tool, review its schemas and the proposed cost assumptions, submit the proposal, then approve the exact request. A connection authorizes only its selected tool/arguments contract within the assignment. Changing a binding blocks older runs until explicit owner rebind. Revocation stops new effects, not already completed remote actions.

OAuth browser login and refresh are implemented for supported metadata/public-client flows. Provider conformance and native-vault behavior require separate live-account acceptance. Do not enter a provider password into an API-key field. Durable server elicitation is answered by the owner; sampling uses the attached agent or a separately approved worker profile.

## Headless equivalents

```sh
apr --state-dir /private/state execution requests --assignment ASSIGNMENT
apr --state-dir /private/state execution request-connection --assignment ASSIGNMENT --proposal-file reviewed-proposal.json
apr --state-dir /private/state execution approve-connection --request REQUEST_ID --digest REVIEWED_DIGEST --secret-stdin
apr --state-dir /private/state execution reject-connection --request REQUEST_ID --digest REVIEWED_DIGEST
apr --state-dir /private/state execution mcp-pin /absolute/real/executable
apr --state-dir /private/state execution mcp-discover --connection-file public-connection.json
apr --state-dir /private/state execution cost-preview --connection-file public-connection.json --request-file approved-request.json --response-file approved-response.json
```

Omit `--secret-stdin` for no-auth operations. For trusted stdio discovery add `--trust-native`; supply `--secret-stdin` only when the chosen credential environment slot needs it. Never put a secret into a command argument, shell history, proposal file or recipe. CLI commands are trusted local owner operations, not agent self-approval instructions.

## Lifecycle and recovery

Closing a window or quitting the desktop does not cancel accepted runs. The separate service continues while the device is awake. Setup's explicit Stop service control requests a stop for the current service instance; it is not a recipe/MCP permission and does not clear saved state. It does not promise rollback or instant completion of already dispatched HTTP actions. Reopening a recipe can start the on-demand service again. Sleeping or powered-off devices cannot run schedules; login startup/always-on remote execution are separate deployment choices.

An expired attachment can be renewed by reselecting the assignment in setup. A pending caller task needs an actual observed result from its authorized agent. An uncertain effect needs reconciliation evidence, not a generic retry. The inspector cannot generate those external results merely because it is inside a desktop app.
