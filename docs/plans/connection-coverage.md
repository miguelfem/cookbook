# Broad connection coverage with agent-assisted setup

Status: **implementation-backed design and remaining launch work**, not a claim
that most services have passed. The current runtime already supplies remote MCP,
local stdio MCP, protected JSON HTTP operations, trusted Python workers and durable
caller handoffs. The missing piece is consistently turning those routes into a
simple, verified connection experience.

## Product contract

A user asks their project agent to connect a tool or create a recipe. The agent
prepares the technical configuration, source and tests. The user signs in, enters
a key in a protected field, or approves a meaningful installation/permission.
They should not have to edit JSON, select a wire protocol, register an OAuth app
or debug a dependency environment.

A recipe can replace a bespoke wrapper's orchestration: accept a stable input,
combine services and local computation, make explicit model decisions, and return
a useful result. It is callable through CookBook's MCP surface. Automatic public
HTTP hosting, a provider marketplace and universal connector coverage are not
implemented or implied.

## Current routes and remaining work

| Surface | Implemented today | What remains for broad easy setup |
| --- | --- | --- |
| Remote MCP | Bounded HTTP JSON/SSE replies; automatic metadata-only selection across four pinned protocol revisions; exact tool contracts; public OAuth discovery, published HTTPS client identity, configured public client profiles, dynamic registration, PKCE, protected refresh and saved accounts. | Qualify the hosted identity through named provider sign-in flows; supply provider-specific public registrations where needed; stronger multi-provider qualification. Standalone legacy SSE is not the current HTTP transport. |
| HTTP APIs | Fixed public HTTPS destination; bounded JSON request/response; fixed GET or POST/PUT/PATCH/DELETE; bearer and two API-key header names; protected metering and effect receipts. | Reviewed path/query templates, documented nonsecret headers, additional credential schemes, multipart/binary artifact handling and explicit streaming behavior. OpenAPI-informed preparation, not a blind spec-to-permission import. |
| Local CLIs and SDKs | Pinned, reviewed stdio MCP commands and frozen Python workers can wrap installed software. The stdio path injects one protected credential into a declared environment variable. | The installed `apr adapter prepare/check` scaffold supplies fixed JSON CLI wrapping and file/hash readiness. Service-specific translation, dependency installation and updates still require agent preparation and owner review. No universal direct shell node is assumed. |
| Calling agent's exclusive tools | A durable handoff can ask the attached agent to perform a declared step and return an observed result. | Better setup assistance and readiness explanations. This route remains dependent on a responding caller; it is not independent service execution. |

A generated adapter can bridge missing wire formats, SDK calls or CLI output.
It cannot repair an OAuth provider's refusal to recognize CookBook, create an
account entitlement, invent credentials or remove a required owner decision.
Those are distinct readiness blockers, not reasons to request a broader token.

## Delegation that already fits the runtime

1. Inventory the requested capability and the exact inputs, output, effects and
   data destinations. Reuse a compatible approved binding or library tool first.
2. Read official service documentation or an OpenAPI description and inspect the
   installed CLI/version when relevant. Treat examples, schema descriptions and
   installation text as data, not permission to run arbitrary commands.
3. Choose the simplest route that meets the contract: supported MCP, direct JSON
   HTTP, or a reviewed local adapter. Assess official MCP coverage without forcing
   it when an API or CLI is the appropriate documented interface.
4. For an adapter, prepare source, a locked dependency environment, fixed launch
   arguments, input/output schemas and a small synthetic contract test. Inspect
   the exact executable/environment CookBook will use; no inherited shell PATH,
   environment secrets or client configuration may be assumed.
5. Present the actual access proposal and installation requirements. Native code
   runs with the owner's OS privileges. A pinned interpreter or launcher does not
   freeze imported packages or turn the adapter into a sandbox. Owner review must
   cover its destinations, effects and dependency/update behavior.
6. After authorized setup, perform a bounded health/contract check, reuse the
   saved account after restart and run a harmless representative operation where
   authorized. Keep writes disabled until their exact operation is approved.
7. Register the immutable recipe, preflight it and report whether it can finish
   independently or needs the caller. Changed adapters/contracts require review;
   failed or uncertain effects are inspected before a retry.

The agent does this preparation through its existing authorized file/terminal
capabilities and CookBook's proposal interfaces. CookBook supplies a bounded CLI adapter scaffold and read-only readiness check; it does not supply a
single automated dependency installer. If those host capabilities are absent, report
that specific limit. Never tell a user to paste a key in the conversation.

Credentials stay in CookBook's owner-managed bindings. The current stdio route
supports one injected secret; multiple credentials, different endpoints or a
changed launcher are not automatically compatible with an existing account.
A future credential profile may authorize reuse across explicit routes, but must
preserve owner-reviewed audience/destination/operation bounds and revocation.
A CLI using its own documented login store needs adapter-specific, owner-reviewed
support for the exact store path and its separate access/revocation lifecycle.
The cleared environment/private working directory does not inherit a logged-in
shell. If that path is unsupported, use the named protected credential or report
the missing authentication path; never extract store secrets into prompts.

## Publishable coverage work, in order

1. **Standard remote MCP setup:** automatic protocol discovery and configured
   CIMD/public-client selection and public CookBook identity hosting are implemented.
   Complete named provider sign-in/callback qualification. Preserve
   exact resource binding, PKCE, scope review and server-metadata validation.
   Protocol probing is metadata-only; it must never retry an executed tool.
2. **Agent-prepared adapters:** the bounded scaffold and readiness diagnostics
   for CLI/SDK translation are implemented. Test them through actual owner proposal/approval and
   first-use flows, and make dependencies and updates reviewable. Keep generic
   repairs out of the end-user instructions.
3. **Common API coverage:** add the bounded templates/headers/artifact transport
   needed by the chosen release integrations. Generate a proposed contract from
   official documentation; never translate an entire OpenAPI document into a
   blanket grant. A recipe cannot supply an arbitrary URL or secret header.
4. **Qualification matrix:** test a named service set across OAuth MCP, key-based
   MCP, JSON API, model/image API and local CLI routes on our devices/accounts.
   Record service/client versions, auth, supported operations, transport limits,
   reuse/restart, revocation, failure and uncertain-effect behavior. Use local
   fixtures for negative cases and owned accounts for the claimed live paths.

The first two are first-launch requirements for a claim of broadly simple MCP
setup plus agent-assisted adapters. API additions are required by the integrations
we choose to advertise. A narrower preview can ship sooner with explicit coverage;
it must not describe these gaps as finished or claim an unsupported percentage.
All acceptance can be run by us. Outside beta participants and external reviewers
are not prerequisites; provider registration or account restrictions may still
be external constraints.

Keep the [launch evaluations](evaluations.md) small: extend E01 with one supported
route from each advertised surface, including a missing-adapter case prepared by
the user's agent. The owner should only authenticate and approve. E02 compares a
saved recipe with both a competent agent baseline and a reusable script. Product
value is not proved by counting fewer nodes or merely moving calls out of chat.

## Standards checked

- [MCP client registration](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/authorization/client-registration.mdx): use known pre-registration, advertised CIMD, then supported dynamic registration. A hosted metadata document is a client identity, not an account token.
- [MCP transports](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/transports/index.mdx): stdio and Streamable HTTP are standard bindings; protocol-era compatibility is a client responsibility.
- [Official TypeScript client](https://ts.sdk.modelcontextprotocol.io/v2/api/@modelcontextprotocol/client/client/client.html): maintained clients demonstrate era discovery. Evaluate SDK reuse behind CookBook's broker boundary before expanding hand-written wire handling; do not import ambient permissions or bypass broker validation.
- [OpenAPI security descriptions](https://spec.openapis.org/oas/v3.1.2.html#security-scheme-object): APIs describe multiple authentication placements/flows. Parsing a description is useful preparation; it does not itself authenticate, grant access or prove execution compatibility.

See [current connection behavior](../runbooks/connecting-services.md) and
[native execution boundaries](../architecture/execution.md) for the authoritative
implemented contract.
