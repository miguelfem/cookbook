# Continue on another device

Updated 2026-09-16.

Source `f7faac0` includes durable scheduled jobs and an optional macOS computer-use scaffold with Jev or caller decisions, and passes the complete macOS/Ubuntu CI matrix. [The validation log](../logs/runtime-validation.md) records local and hosted tests and packaging, separately from older performance measurements. Live GUI operation and live provider quality remain unverified. Documentation commits may follow that tested source revision.

## Open the platform on this device

Run these commands on the computer where you want to use the interface. A remote computer's `127.0.0.1` address does not point to your current device.

```sh
git clone https://github.com/miguelfem/cookbook.git
cd cookbook
cargo build --locked -p apr-cli
python3 tools/try_local.py
```

The [demo guide](local-demo.md) explains the sample review, paused schedule, and session lifetime. It creates fresh local data, without transferring the earlier remote demo or modifying Codex/Claude settings. The public product is **CookBook APR**; the command remains `apr`, and Jev stays optional.

## Source and tests

```sh
git clone https://github.com/miguelfem/cookbook.git
cd cookbook
cargo build --locked -p apr-cli
cargo test --locked --workspace
cargo test --locked --workspace --all-features
python3 tools/check_docs.py
```

The repository pins Rust 1.94.0. Python 3.11 or newer is used by the documentation/demo/distribution tools and native-worker tests. The optional `jev` build needs no credential to compile or run contract tests. Fault-injection builds are only for validation. No Node, Tauri, hosted database or inference account is required for core use.

Open the folder in Codex or Claude Code and read [start here](../00_START_HERE.md). Follow [client setup](client-integration.md) for a new local attachment and [the inspector runbook](local-inspector.md) for the live interface.

Follow [scheduled jobs](scheduled-jobs.md) to create once/interval/UTC-cron jobs and optionally generate an OS service file. Follow [computer use](computer-use.md) to scaffold a process and prepare its local interpreter, dependencies, permissions and execution grants. These are separate setup actions; cloning/building never installs them or operates the desktop. Computer use currently requires macOS, while core scheduling also supports Linux.

## Transfer saved runs separately

Use `apr --state-dir SOURCE backup --output ABSENT_BACKUP_DIRECTORY` on the source device. The output contains private run data and artifacts, so transfer it privately and keep it outside Git. On the new device, use `apr --state-dir ABSENT_STATE_DIRECTORY restore BACKUP_DIRECTORY`.

Restore verifies the inventory and hashes before publishing the new state. Credentials and attachment/handoff bearer files are excluded. All old attachments are revoked, execution/promotion grants disabled and schedules paused. Use `project relocate PROJECT_ID LOCAL_PROJECT_PATH` to establish the new local project identity, then issue fresh attachments. Explicitly renew a waiting handoff. Re-establish required owner grants and use `execution rebind RUN_ID` before continuing affected independent work. Resume a schedule only if its original frozen authority remains valid; otherwise recreate it after reviewing the new grants. Saved uncertainty is never converted into automatic retry.

Backups preserve source snapshots and completed results. They do not install Python dependencies, move client conversations, restore credentials, or approve a different provider account. Completed step results remain authoritative after restore.

## Paste into the next conversation

> Continue CookBook APR (Agent Process Runtime) in this repository. Read AGENTS.md, docs/00_START_HERE.md, docs/plans/task.md, docs/plans/computer-use-and-schedules.md and docs/logs/runtime-validation.md before changing code.
>
> APR is a general local Rust workflow runtime for Codex/Claude, scoped by project and assignment. It supports independent steps and durable explicit caller handoffs. Preserve immutable versions/config/source, protected bindings, SQLite recovery, uncertainty reconciliation and run-tree budgets. Jev stays optional and absent from the normal CLI graph. Native Python is trusted execution, not an OS sandbox.
>
> Scheduled jobs freeze their inputs, graph/configuration and grants, coalesce downtime, and prevent overlapping waiting/uncertain runs. Restored schedules stay paused. Computer scaffolds use bounded native actions and explicit completion evidence or caller review; retain the upstream credit/license and never replace per-action recovery with a blind replay loop.
>
> Build on the current implementation; do not restart the durable core or replace the live inspector with the older mockup. Run focused public-boundary tests, then the pinned workspace checks. Keep actual model-driven client journeys, live provider quality, token economics and signed release separate from protocol/contract tests. Licensing is still an owner decision unless the repository now contains an explicit license. Read the tracker for the exact remaining gates and CI status.

## Preserved and private material

Git contains the implementation, fixtures, specification, plans, protocol, visual concept and evidence. It does not contain private documents, client settings, API credentials, runtime databases, continuation tokens or generated production media.

Historical client observations in the research documents are not assumptions about this device. Run integration diagnostics against the clients actually installed here. Permission/trust prompts remain under each client's own policy.
