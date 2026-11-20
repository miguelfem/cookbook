# Reusable tools and stored accounts

CookBook separates the protected account credential from each permitted operation. Previously, an approval stored a key in the operating-system vault and created an assignment-scoped connection: any recipe in that workspace could use it, but another project could not discover or reuse it. Migration 15 adds an owner-managed library of exact approved tools. Migration 17 adds independent saved accounts and backfills active credential references without sharing tools or reviving disabled grants.

The trusted approval window defaults to **Only this workspace** and offers **All projects** separately. Both choices save a newly connected account for future reviewed operations. Browser sign-in and remote MCP API-key setup can also save an account before any operation is approved. All projects atomically saves the approved connection and its protected credential reference in the library while granting the requesting workspace access. Existing and future projects can discover that exact tool. Workspace-only keeps the earlier scope. Credentials are never returned through MCP, recipe outputs, guides or public contracts. Library publication is an owner operation, not an author permission.

An earlier approved Jev or other service can be published from **Settings & access → Tools & accounts → Share an existing workspace connection**. Review its exact contract and current revision, then choose **Make available to all projects**. This uses the existing protected reference without retrieving or entering the key again. Re-publishing a library-derived grant preserves the original tool identity. OAuth browser approval may complete its workspace grant before this explicit publication; a failed publication must be retried through the existing-connection flow, not treated as a global grant.

An author discovers `apr_inspect {view:"connection_library"}`, pages using `after:next_cursor`, and reads a selected `library_contract` by `library_id` and `revision`. It binds the exact revision locally with `apr_manage {action:"connection.use",library_id,revision,name}`. The caller supplies a local name, never a replacement endpoint, command, credential, schema or permission. This operation is idempotent and refuses conflicting names or stale/disabled tools. Run-only attachments cannot bind tools. Recipes still declare capabilities and use ordinary preflight, budgets, frozen bindings, dispatch intents and reconciliation.

For a newly proposed operation, the owner can select an existing stored credential instead of entering the same key again. Reuse requires the exact same destination, transport and authentication contract, including header name or OAuth configuration. Another destination/command/audience needs its own credential setup. Reusing an account never silently approves a different operation. Pending access remains a readiness dependency and is not replaced by a routine caller handoff.

Library disable invalidates every unchanged derived grant, including the originating grant. In-flight requests may finish; frozen runs fail the ordinary current-revision check before later dispatch. Owner-replaced unrelated bindings remain intact. A workspace-specific revocation blocks that library identity in the workspace, including new aliases. Disconnecting an account disables all current tool grants and library entries that use its protected reference, increments revisions, and attempts vault removal. Tool disable leaves the account available for a distinct reviewed operation. Restoring a backup disables saved accounts, the library, grants and attachments; copied account metadata never authenticates a restored installation. Relocation retains explicit reauthorization requirements. Re-enabling withdrawn authority requires a new owner review; neither replay nor a different recipe name grants it.

Owner-only `execution account-list` pages account metadata; `execution account-revoke --id ACCOUNT --revision REVISION` disconnects a reviewed account. `execution approve-connection` accepts `--saved-account ACCOUNT --saved-account-revision REVISION`; legacy library credential selection remains compatible. Account inventory and revocation are unavailable through scoped MCP. Compatibility uses exact endpoint, transport/command, auth scheme/header and OAuth configuration, not an endpoint-only UI guess.

The library is local to this CookBook installation. It does not sync secrets between devices, infer live credential health, or promise savings. Tests use disposable databases and synthetic credentials; real account acceptance is separate.

Browser sign-in and remote MCP API-key setup save the account before metadata discovery. If listing tools,
resources or prompts fails, the account remains saved and the owner sees the
discovery failure separately. The owner can select a compatible saved account
after a restart; multiple matching accounts require an explicit choice. Native
discovery receives only its account ID and revision; the runtime rechecks its current
revision, enabled state and exact contract before vault access. Discovery grants
no operation. HTTP failures expose only the status code and fixed guidance, never
provider response bodies; uncertain tool effects retain their recovery boundary.

The normal **Connect a service** page combines public OAuth discovery and browser
sign-in; the owner need not copy metadata between pages. API-key setup supports
bearer, `x-api-key` and `api-key` contracts and uses the owner-only native
`connect_mcp_account` boundary. It rejects mixed new-key/saved-account selection,
invalid discovery kinds and nonremote/non-key contracts before saving. Successful
storage and metadata failure are separate outcomes; neither creates a grant.

## Public client identity and protocol selection — 2026-09-22

OAuth discovery uses an exact-issuer public client profile first, then the
maintainer-configured HTTPS Client ID Metadata Document when the authorization
server advertises support, then supported dynamic registration. No fabricated
identity or automatic account authorization is supplied. The shipped public
profile is in `integrations/oauth/client-identity.json`; an owner-managed
`oauth-client-identity.json` in runtime state can override it. This bounded,
validated file contains public identifiers only, rejects secret/unknown fields
and symlinks, and is unavailable to scoped author management. Existing saved
OAuth bundles omit the new optional metadata field and keep their old digests.

The bundled profile now names `https://itscookbook.dev/client.json`, a public
static native-app identity. The installed app can use the same owner-managed
override without replacing its executable. Publication does not grant account
access, and a working identity URL does not qualify a provider's sign-in flow.

A hosted identity is fetched and validated during discovery and sign-in. It must
list exact loopback callback addresses; the native flow binds an available listed
port and never silently chooses an unlisted redirect. Provider-specific public
registration may still be required. Preparation/configuration grants no operation
and does not read or migrate account credentials. See the
[maintainer hosting steps](../runbooks/connecting-services.md#maintainer-client-identity-setup).

Automatic MCP negotiation happens only during owner discovery. It reuses the
exact account/destination/authentication contract, returns the detected version
and pins it in the proposal. Normal dispatch requires the approved version.
Negotiation never invokes a tool or widens saved-account compatibility.

## Reviewed reuse after a native adapter update

Automatic account matching still compares the complete transport, including
arguments and source pins. An updated local adapter is never silently considered
the same credential destination. The owner can request an explicit preview for
a pending proposal and a current saved account revision. Eligible candidates are
limited to native stdio bearer/header credentials with the same endpoint,
executable path, authentication scheme/header and credential environment name.
Changed launch arguments, hashes and operation permissions remain review content.

The preview contains old/new public account contracts and operation schemas. It
warns that opaque native code can hide destinations; pins are not a source diff
or a sandbox. Its digest binds the proposal digest, account ID/revision and all
reviewed contracts. Approval recomputes this under the store lock, records the
decision with the ordinary approval receipt, and reuses the protected reference
without reading or copying the key. No author MCP action accepts this exception.
Stale/revoked accounts, changed previews and conflicting replays are rejected.

The original account contract remains exact and unchanged; the decision grants
only the reviewed proposal. Existing grants and pending requests are retained.
Disconnecting the account still disables every revision using its reference.
Native UI separates **Review updated adapter** from an exact saved-account match
and requires a fresh preview plus explicit consent before approval.
