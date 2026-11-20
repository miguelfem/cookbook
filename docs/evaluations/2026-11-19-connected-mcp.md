# Repeat a prepared recipe on a connected MCP session

The [first profile benchmark](2026-11-19-profile-batch.md) used fresh Codex
sessions and automatic MCP approval review. This follow-up held a client
thread open for three batches per arm. The recipe version, input contract,
local interpreter grant and client tool configuration were ready before
timing. Each measured call returned the complete ordered synthetic profile
list. One result required no additional discovery or inspection calls.

## Where the time went

In the original-bridge warm probe with the scoped approval policy, MCP tool
calls took 0.88–0.90 s within Codex turns of 9.54–13.96 s. The execution path
made no provider or handoff calls. A Codex turn still invokes the model to
choose and construct the tool call and to produce the requested final JSON.
Automatic approval review is a further cost when configured. The warm probe
does not isolate its exact share in the first benchmark, which used different
session setup too. Use the explicit owner-approved per-tool choice when it
fits the project. [Managed Codex configuration](../architecture/client-integrations.md)

## Prepared result and typed input checks

The candidate adds an `output` view that returns exact values and the run ID.
A `receipt` view leaves completed values in the saved run for the inspector.
The compact response observed in this fixture was 1,434 bytes in the agent's
tool result. Output view reduced it to 1,125 bytes, about 22%. Output view can
also use the existing detailed-response budget, so an eligible result above
8 KiB can finish in one reply. Unfinished work keeps its durable identity and
handoff or error fields.

For a managed project, the test connected through `apr_project`, used the real
filesystem proof, then registered the fixture and approved only its disposable
Python grant. Codex App Server's `mcpServer/tool/call` submitted
typed inputs to the existing `apr_run` tool with that workspace handle. The
host received the exact JSON. It started no model turn and received no
thread-usage event. In a product integration, the caller can display those
values directly. Start and result reads retain the runtime's existing authority.

| Approach | Exact batches | Median end to end | Total input tokens | Standard API equivalent |
| --- | ---: | ---: | ---: | ---: |
| MCP compact, fixed attachment | 3/3 | 9.31 s | 185,811 | $0.08698 |
| MCP output, fixed attachment | 3/3 | 10.56 s | 166,411 | $0.07325 |
| MCP output, managed project | 3/3 | 10.81 s | 167,198 | $0.07429 |
| Prepared script in Codex | 3/3 | 8.75 s | 182,804 | $0.08402 |
| Typed host call, managed project | 3/3 | 0.83 s | 0 | $0 in model tokens |

All agent measurements used Codex CLI 0.169.1, GPT-6 Sol Low and three
consecutive turns per arm. Each turn used one tool call. Two orders alternated
across the eight fictional addresses. All MCP arms explicitly enabled only
the same scoped `apr_run` approval option used by the managed setup feature.
Token totals use differences in cumulative per-thread counters and include
cached input. Costs use Standard GPT-6 Sol pricing. ChatGPT subscription
charges were unobserved. [Model pricing](https://developers.openai.com/api/docs/models/gpt-6-sol)

These small probes used separate threads per arm and were run in sequence.
Cache warmth and client timing varied. The fixed-attachment output response
was smaller and the measured input token and estimated cost totals were
lower than the script's. Chat latency remained higher. This trial establishes
the model-free path for typed inputs and the response-size reduction. A
direct, model-free script remains a strong local baseline.

[Sanitized per-case metrics](fixtures/connected-mcp-2026-11-19/metrics.json)
