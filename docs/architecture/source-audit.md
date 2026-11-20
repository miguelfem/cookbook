# Source audit and prior art

Checked 2026-09-14. Documentation/source inspection is separate from independently executing software. No precedent was installed or benchmarked for this design.

| Source | Relevant documented capability | Consequence for APR |
|---|---|---|
| [OpenClaw Lobster](https://docs.openclaw.ai/tools/lobster) / [repository](https://github.com/openclaw/lobster) | Local reusable pipelines, structured workflow definitions, resumable checkpoints. TypeScript/Node implementation. | Closest local precedent; compare before maintaining another engine. |
| [Lobster embedded limitation](https://docs.openclaw.ai/tools/lobster#important-limitation-embedded-lobster-vs-openclawinvoke) | Embedded dispatch context has specific limits. | Do not infer access to a calling Codex/Claude session. |
| [Cloudflare Code Mode](https://developers.cloudflare.com/agents/tools/codemode/) | Experimental code-driven tool composition and progressive discovery. | Compact tool interfaces and keeping intermediates out of context are established ideas. |
| [Code Mode mechanics](https://developers.cloudflare.com/agents/tools/codemode/how-it-works/) | Host connectors, saved snippets, and approval replay with recorded call results. | Specify stable identity, replay limits, and promotion explicitly. |
| [Code Mode package](https://github.com/cloudflare/agents/blob/main/packages/codemode/README.md) | Custom executors are possible; supplied Dynamic Worker executor uses Workers. | A local isolated implementation remains engineering work. |
| [Browser Code Mode](https://developers.cloudflare.com/agents/tools/codemode/browser/) | Browser-owned tool registration with a different execution/approval model. | Browser execution is not inherited access to arbitrary client tools. |
| [Dynamic Workflows](https://developers.cloudflare.com/dynamic-workers/usage/dynamic-workflows/) | Separate durable execution primitives. | Do not conflate ordinary code execution, approval replay, and durable jobs. |
| [Windmill MCP](https://www.windmill.dev/docs/core_concepts/mcp) | Workflow/script exposure through MCP. | Study distribution and authoring cost; no new comparative runtime test here. |
| [Codex MCP](https://learn.chatgpt.com/docs/extend/mcp) | Local stdio, project configuration, host-shared integration. | Baseline Codex adapter. |
| [Codex skills](https://learn.chatgpt.com/docs/build-skills) / [config](https://learn.chatgpt.com/docs/config-file/config-basic) | Repository skills and trusted project configuration. | Progressive instructions and safe settings merge. |
| [Claude MCP](https://code.claude.com/docs/en/mcp) / [skills](https://code.claude.com/docs/en/skills) | Project-scoped extension primitives. | Baseline Claude adapter; client-owned trust remains. |
| [Claude hooks](https://code.claude.com/docs/en/hooks) / [plugins](https://code.claude.com/docs/en/plugins-reference) | Lifecycle hooks and bundled extensions have timing/cwd constraints. | Optional conveniences, not durability or enforcement. |
| [Claude workflows](https://code.claude.com/docs/en/workflows) | Saved JavaScript can coordinate agent work with replay. | Another reuse precedent; direct deterministic execution plus explicit caller handoff remains the focus. |
| [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk) | Typed MCP plumbing over supported transports. | Reuse protocol infrastructure; build domain engine separately. |
| [Rust process API](https://doc.rust-lang.org/std/process/struct.Command.html) | Arguments, environment, and cwd controls. | Useful supervision hygiene, not filesystem/network confinement. |
| [Deno security](https://docs.deno.com/runtime/fundamentals/security/) | Permission-based runtime with explicit caveats for native/subprocess access. | Candidate executor direction, not a selected backend. |
| [Wasmtime security](https://docs.wasmtime.dev/security.html) | Wasm isolation and explicit host capabilities. | Candidate for a separately verified module executor. |
| [rquickjs runtime](https://docs.rs/rquickjs/latest/rquickjs/struct.Runtime.html) | JS runtime limits with build-feature caveats. | Evaluate exact features if selected; not a promise of OS isolation. |

## Assessment

Additional implementation references checked 2026-09-16:

| Source | Design use |
|---|---|
| [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html) | Keep Jev optional at build time and inspect the resolved feature/dependency graph rather than trusting a single default declaration. |
| [SQLite backup interface](https://www.sqlite.org/backup.html) | Use a consistent database snapshot plus pinned referenced artifacts for APR backup/restore work. |
| [GitHub licensing guidance](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository) | Distinguish public source visibility from the still-unselected open-source license. |

Agent-authored reusable code is already available in several systems. APR's proposed value is the combined local installation, project/assignment lifecycle, small agent interface, durable caller-owned tool handoff, and usable connection/configuration management across clients.

This is a product hypothesis, not proof of a unique market gap. M0/M5 include a build-versus-extend decision and comparable workflow measurements.

## Verification limits

Current client documentation can change. Test exact supported versions before claiming compatibility. A docs example does not demonstrate a complete install, trust, handoff, disconnect, or recovery journey.

Observed during the original 2026-09-14 research: the app-bundled Codex CLI's version/help supported stdio registration; no APR connection was attempted because no APR binary exists. Claude Code was absent. No service restart, provider effect, sandbox escape, or token-economics test was performed.

## Implemented reuse and redistribution evidence

On 2026-09-16, the [current notice inventory](../../third_party/dependency-notices.json) was regenerated for default `apr-cli` on `aarch64-apple-darwin`. It includes exact Cargo checksums and root/declared license-file hashes for 195 resolved normal/build components. Four upstream license omissions are filled by [commit-pinned supplements](../../third_party/distribution-supplements.json). The Rust standard-library notices are included by the local distribution tool from its packaging toolchain; that is not proof of the supplied binary's build provenance.

The [third-party notice](../../THIRD_PARTY_NOTICES.md) records the narrow Lobster atomic-publication translation, unchanged MCP SDK linkage, MIT SIMD supplements and the SQLite 3.51.3 amalgamation. APR's original license, signed publication, actual client acceptance and comparative token economics remain separate from these executed source and packaging checks.
