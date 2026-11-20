# Work in this repository

Read [docs/00_START_HERE.md](docs/00_START_HERE.md), then the canonical documents relevant to the task. [docs/INDEX.md](docs/INDEX.md) maps the rest.

The Rust runtime includes scoped authoring/configuration, managed client setup, a live local inspector, native workers, protected connections, durable controls, backups/retention, versioned routing/composition, durable scheduled jobs and an optional computer-use scaffold. Read the tracker and validation log for precise acceptance evidence. The older design HTML remains simulated. Actual model-driven client journeys, provider quality and token savings must not be inferred from protocol tests. Proposed future features must remain labeled.

Before implementation, read the current slice in [docs/plans/task.md](docs/plans/task.md), its work packages in [docs/plans/implementation.md](docs/plans/implementation.md), and required evidence in [docs/plans/verification.md](docs/plans/verification.md). Keep product behavior in the product spec, invariants in architecture, execution tasks in the plan, and decisions in the decision log. Update affected canonical documents when a decision changes.

Preserve these requirements: local Rust core; agent-driven process authoring; Codex and Claude Code adapters; project and assignment scoping; independent execution plus explicit durable caller handoff; protected credential bindings; general workflow routing across domains. Public examples must use synthetic, general-purpose workflows and must not describe the owner's personal projects. Routing/composition uses manifest version 0.2; preserve legacy 0.1 admission and saved histories. Do not assume access to native tools in another application or that native scripts are sandboxed.

For decision/provider changes, read [docs/architecture/decision-providers.md](docs/architecture/decision-providers.md). Jev is optional and excluded from default builds/setup; core implementation and tests must work without its credentials or inference calls. This is a public source preview. CookBook’s project license is undecided.

For schedules or computer use, read [scheduled jobs](docs/architecture/scheduled-jobs.md), [computer use](docs/architecture/computer-use.md) and their [implementation plan](docs/plans/computer-use-and-schedules.md). Schedule occurrences must remain atomic, scoped and non-overlapping across waiting/uncertain work. Computer actions require fresh observations and individual effect intents; native fixtures must not operate the user's desktop. Preserve the upstream MIT license and adaptation credit.

Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo test --workspace --features apr-cli/fault-injection --locked` for runtime changes. Run `python3 tools/check_docs.py` after documentation/contract changes. Exercise the live inspector against disposable service state after UI changes. Record precisely what was verified. Fault injection must remain excluded from ordinary builds.

Do not commit credentials, local grants, runtime databases, generated artifacts, or device-specific client paths. Do not install MCP entries or alter a user's client settings unless the current task calls for it.
