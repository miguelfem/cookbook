# Credits

CookBook benefits from existing projects and the people who built them. The entries below distinguish adapted source, dependencies and design influences. Attribution does not imply endorsement or equivalent behavior.

## Adapted source

| Project and author | What CookBook uses | Provenance and license |
| --- | --- | --- |
| [typesafe-computer-use](https://github.com/awlevin/typesafe-computer-use) · **Aaron Levin** | macOS event posting, accessibility lookup and OCR block merging in the optional [computer worker](integrations/computer-use/computer_worker.py). CookBook supplies its own bounded workflow, protected connections and recovery. | Commit `7d120f1944d0156773d5089b81f8e46d108a6c8d`; original `macos.py` and `perception.py`. [Provenance](third_party/typesafe-computer-use.json), [adaptation notes](integrations/computer-use/NOTICE.md), [MIT license](third_party/licenses/typesafe-computer-use-7d120f19/LICENSE). Copyright (c) 2026 Aaron Levin. |
| [OpenClaw Lobster](https://github.com/openclaw/lobster) · **Vignesh** | The atomic exclusive file-publication sequence, translated to Rust in [artifact storage](crates/apr-runtime/src/artifacts.rs). This is a narrow adaptation, not the Lobster workflow engine. | Commit `041124ad4ae5f05179302970a235fe5d34bac45b`; [`writeFileAtomicExclusive`](https://github.com/openclaw/lobster/blob/041124ad4ae5f05179302970a235fe5d34bac45b/src/state/atomic_file.ts). [MIT license](third_party/licenses/lobster-041124ad/LICENSE). Copyright (c) 2026 Vignesh. |

The complete upstream texts remain in the repository. Computer-use scaffolds also carry their attribution and license. The adapted worker stays pinned; [upstream review](docs/architecture/computer-use.md#review-before-an-upstream-update) stages changes for review rather than silently replacing it.

## Libraries and tools we build on

- **[Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)** (`rmcp` / `rmcp-macros`) provides protocol infrastructure. The unmodified dependency's [complete transition and license notice](third_party/licenses/rmcp-3.4.0/LICENSE) is retained.
- **[Tauri](https://v2.tauri.app/)** provides the native desktop shell and platform integration.
- **[Microsoft Playwright](https://github.com/microsoft/playwright-python)** supplies optional headless browser automation under Apache-2.0. Its browser binaries and Python package are explicitly installed separately; CookBook's isolated-session helper is original orchestration code, not copied Playwright source.
- **[SQLite](https://www.sqlite.org/)** through **[rusqlite](https://github.com/rusqlite/rusqlite)** provides persistent local state. The [SQLite notice](third_party/licenses/sqlite-3.51.3/PUBLIC-DOMAIN-NOTICE.txt) is preserved.
- The **Rust ecosystem** supplies serialization, schemas, HTTP transport, protected credential storage, process support and packaging. The [dependency notices](THIRD_PARTY_NOTICES.md) and [machine-readable inventory](third_party/dependency-notices.json) record the checked dependency graph and its coverage limits.

**[TypeSafe / Jev](https://typesafe.ai/)** is an optional external decision provider. CookBook implements its adapter; it does not bundle Jev's model. Provider quality and pricing claims belong to their sources and require separate CookBook evaluations.

## Design influences

| Source | Influence |
| --- | --- |
| [OpenClaw Lobster](https://github.com/openclaw/lobster) | Reusable local workflows and resumable work, in addition to the specific source adaptation above. |
| [Cloudflare Code Mode](https://developers.cloudflare.com/agents/tools/codemode/) | Code-driven tool composition and compact, progressive tool discovery. |
| [Windmill](https://www.windmill.dev/docs/core_concepts/mcp) | Exposing reusable scripts and workflows through MCP. |
| [Grep](https://grep.ai/) | Recorded-run review, evaluation before adoption and optimization of repeated workflows. Design reference only; no Grep implementation is incorporated. |
| [Linear's design writing](https://linear.app/now/how-we-redesigned-the-linear-ui) | Restrained navigation, alignment and text hierarchy in the [cookbook design system](docs/specs/design-system.md). No Linear code or assets were copied. |

Cloudflare Code Mode and Windmill are research references, not embedded runtimes. The [source audit](docs/architecture/source-audit.md) records earlier research and the [recipe improvement contract](docs/architecture/recipe-improvement.md) defines CookBook's narrower implemented behavior.

## Keeping credit accurate

For new source reuse, record the exact source files, immutable revision, original author, full license text and local changes before merging. For inspiration, describe the idea and link its source without implying copied code. Keep this overview aligned with [Third-party notices](THIRD_PARTY_NOTICES.md); it is not a replacement for the license inventory.

CookBook's own software license remains undecided. The upstream licenses above apply to their respective contributions.
