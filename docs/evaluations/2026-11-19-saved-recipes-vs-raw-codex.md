# Saved CookBook recipes vs. Codex from raw inputs

2026-11-19 · macOS ARM64 · Codex CLI 0.169.1 · GPT-6 Sol, Low

The previous comparison gave Codex alone a ready-made Python command. This run gave that side raw synthetic data and the task rules, with no workflow script in the project. CookBook used the same three saved recipes. Authoring and registering the recipes, granting execution, importing inputs and connecting the workspace took place before timing. Codex-alone code written during the turn counted toward its result. Each arm of each matched input ran in a separate fresh ephemeral thread, for 24 measured turns. A new raw-only folder was used for each input. No raw case invoked an MCP tool or referenced a script from the prepared project in its recorded shell commands.

| Workflow | Codex alone, median | CookBook in Codex, median | Raw input tokens | CookBook input tokens |
| --- | ---: | ---: | ---: | ---: |
| Profile lookup, 4–16 addresses | 9.85 s | 12.93 s | 186,715 | 166,776 |
| Inventory reconciliation, 72–300 rows | 7.67 s | 9.75 s | 165,135 | 210,846 |
| Ticket SLA checks, 72–300 rows | 10.50 s | 8.39 s | 212,292 | 189,421 |
| Total for 12 cases | 115.12 s | 123.94 s | 564,142 | 567,043 |

Four inputs per workflow make this a pilot. The profile cases used the same fictional records and requested order. The saved profile recipe injects a per-record I/O delay while the raw side reads a local CSV directory. Profile latency therefore includes this implementation difference. Inventory and SLA produced matching shapes and rows under the same task rules, with one strict grading exception noted below.

## Why the token gap got smaller

CookBook used **0.51% more input tokens** and **0.58% more total input and output tokens**. Both arms made **27 model requests** across 12 turns. Each arm had nine two-request turns and three three-request turns. The first request averaged about **20,500 input tokens** on each side, before any workflow result was available. Later requests also carried Codex context. This is client-reported input usage, including cached tokens.

The earlier [prepared-script comparison](2026-11-19-fresh-threads-and-standalone.md) saw a larger difference because CookBook made two additional model requests across the 12 pairs. This run shows why removing script authoring did not translate into lower input tokens for these tasks. The raw side could read the file or write and run code in one shell command most of the time. It used 15 shell calls across 12 turns. CookBook used one `apr_run` per turn. Saving three execution tool calls did not reduce the number of model requests in this sample.

Codex alone used 3,890 output tokens, compared with 4,292 for the CookBook turns. The observed token mix corresponds to $0.36124 for Codex alone and $0.28513 for CookBook at published GPT-6 Sol Standard API rates. CookBook's estimate was 21.1% lower in this run because more of its input was cached. These are API-equivalent estimates. The runs used a ChatGPT sign-in, and cache reuse varies across runs. [GPT-6 Sol pricing](https://developers.openai.com/api/docs/models/gpt-6-sol)

Four supplemental turns tested whether this App Server session exposed raw model response events. The session emitted usage and workflow tool timing without raw response events, so the cause of an individual third request remains unresolved. These probes are outside the matched totals.

## Correctness

All 24 chat replies were exact. CookBook met the full output and chat contract on 12/12 cases. Codex alone met it on 11/12. Its 96-row inventory output had all expected rows and the correct chat counts, and included an extra top-level summary key in the saved report. The strict grader expected only the rows key.

## Where reuse helps

Codex can run a normal script stored in a repo. A project `AGENTS.md` can point it to the command, or a skill can bundle the script and its instructions. A matching skill can activate implicitly from its description, and the user can invoke it explicitly as well. Either approach lets a model-driven Codex turn reuse existing tooling. A registered CookBook recipe also has a typed execution entry point, so a caller or schedule can run it without creating a model turn. The [previous typed comparison](2026-11-19-fresh-threads-and-standalone.md) verified 12 exact outcomes with zero model tokens. [Codex skills](https://developers.openai.com/codex/skills/) · [Project instructions](https://developers.openai.com/codex/guides/agents-md/)

## Evidence and reproduction

[Per-case CSV](fixtures/2026-11-19-raw-inputs/raw-comparison-cases.csv) · [Sanitized traces](fixtures/2026-11-19-raw-inputs/raw-comparison-traces.json) · [Harness](../../tools/evals/benchmark_codex_raw_inputs.py)

Run the harness with the installed managed CookBook connection and a ChatGPT Codex sign-in. Give the run a new output path.

```sh
mkdir -p work
python3 tools/evals/benchmark_codex_raw_inputs.py --output work/raw-codex-vs-cookbook
```
