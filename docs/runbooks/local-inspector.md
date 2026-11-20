# Inspect an attached assignment

## Recipe/desktop extension — 2026-09-17

On the implementation branch, recipe-author payload/cost analysis, scoped connection proposals, trusted owner approvals, outbound tools-only MCP, and the Tauri desktop now extend the earlier inspector. [Current architecture](../architecture/recipe-connections.md) and [desktop runbook](../runbooks/desktop.md) supersede earlier statements that all of those surfaces are deferred. Workflows are branded **Recipes**; wire identifiers stay compatible. Source presence is not release evidence: consult the validation log for actual checks. OAuth login/refresh, Windows, signing, general visual building, invoice reconciliation and hard monetary caps are not implied.

The inspector is a bundled browser interface served by the local Rust CLI. Its workflow view represents processes generally authored by the agent; it is not a general visual builder. It uses the same private APR service and scoped attachment authority as MCP. No desktop shell or remote website is required.

```sh
apr inspector --attachment /private/apr-state/attachments/client.json --port 0
```

Open the printed `http://127.0.0.1:PORT/#session=…` URL. Port `0` selects an available loopback port. Keep the command running; stopping it closes the inspector while the independent APR service keeps saved work. The printed URL grants the inspector session's assignment authority until this inspector process exits, so do not share it.

The browser removes the URL fragment immediately and keeps the inspector session token only in memory. It never receives the attachment file or its bearer credential. A refresh after the fragment is removed has no session; reopen the original printed URL. Starting a new inspector creates a new session token.

Use the page to:

- Browse paginated run history, then inspect a run, its frozen configuration, events, or pending caller handoff.
- Discover registered processes and their schemas, preflight inputs, and start work with a stable request key.
- Pause, continue, renew a handoff, cancel, or submit a complete observed caller result.
- Inspect a bounded artifact preview by pasting its authorized reference.
- With an author attachment, validate/register project-relative manifests, import artifacts, and read/set ordinary assignment configuration.

Run-only attachments hide author controls, and the service remains the authority even for manually crafted requests. The browser cannot grant permissions, set protected credentials, relocate projects, or operate another assignment. A paused caller request still needs the caller's own authorized tools and an explicit submission; the inspector does not acquire client-native tools.

The web server binds only `127.0.0.1`, verifies exact Host and Origin values, requires a separate custom session header for JSON POST requests, and serves only bundled assets with a strict content-security policy. It rejects CORS preflights, duplicate headers, chunked bodies, requests over 64 KiB, and headers over 16 KiB. At most eight requests are active; HTTP reads have a three-second absolute deadline. Service dispatch retains its own bounded timeout. Responses are normally 8 KiB, with explicit detail requests up to 64 KiB. Runtime data is rendered as text, including artifact previews.

This is a loopback interface, not a sandbox against arbitrary same-user host programs. Client setup/trust remains separate; see the [client integration runbook](client-integration.md). The earlier [design mockup](../../design/inspector.html) is a separate simulated artifact. Browser delivery replaces that mockup for live inspection; a future packaged desktop shell can reuse the same service boundary.

## Follow a nested workflow

Select a catalog process or saved run to open its diagram. Nodes and arrows come from the admitted process definition; a saved run uses its frozen version and recorded step states. Select a call node to open its pinned child definition, or a created child run to follow the actual execution. Breadcrumbs return to earlier views. The **Nested runs and pending reviews** panel lists the run family and identifies actionable descendant requests without loading every child schema.

A pending review belongs to its exact child run. Open that child and inspect its run to populate the submission form. Parent runs in `waiting_for_child` cannot receive a child's result under the parent's identity. A successful leaf submission allows its parent chain to advance; refresh the root to see the final state. Parent pause/cancel semantics remain those of the durable runtime.

Graphs and families are paginated. Each graph request shows up to 100 step definitions with at most 20 instance previews per step, and tree inspection pages through all created children. A partial graph page does not imply absent dependencies. No graph view evaluates code or reveals raw step inputs, credentials or handoff tokens.

## Fine-tune Jev decisions

With an author attachment, select an editable Jev question or directly associated routing node, then **Tune Jev decisions**. Edit the existing question instructions, choice descriptions, score rubric, or supported routing threshold/fallback/rule values. Preview the precise edits and candidate version, then publish that reviewed version. Changing a field invalidates the preview.

Question types, choice identifiers, score cardinality, graph edges, route sets, bindings, code and permissions remain fixed. Broader changes belong to the agent/source authoring workflow. The narrow operation rejects edits to non-Jev policies. Structured instructions remain JSON fields when the existing value is structured; ordinary question and criterion text uses text controls.

Publication registers a new immutable catalog version and retains the original registered source snapshots. It does not rewrite the project source file, active runs, parent call references, saved assignment configuration for an older version, or schedules. Ask the agent to update/re-register callers and recreate schedules if they should use the new version; configure that version separately when needed. Preview is structural validation, not a provider quality evaluation or execution grant. No inference call is made; live Jev execution still requires the optional adapter, configured protected connection and owner authorization.

Desktop packaging is deferred. The runtime and scheduled work remain independent of this browser interface.

## Analytics and connection status

Open **Analytics & costs** for retained assignment metrics or the selected run's entire root tree. Refresh explicitly after new work. External attempts show usage, configured-meter estimates, provider-reported amounts and provenance. Currency subtotals cover **only the current call page**; reported and estimated values are alternative observations, not additive charges. Missing usage is unknown. Fixtures are included, and client subscriptions/calls outside APR are excluded. See [metering](../architecture/metering.md) for setup, units, privacy, rounding and limitations.

**Connections** lists the assignment's existing execution bindings and revision/enabled state. It does not test service health, add credentials, install external MCP servers, or grant access. Owner setup remains separate. No desktop shell is included in this slice.
