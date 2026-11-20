# Try CookBook APR on your device

This opens the working local interface with synthetic data. It is an early technical inspector, with JSON forms and explicit handoffs. It is separate from the older simulated design page.

## Start

Use macOS or Linux with Rust 1.94.0 and Python 3.11 or newer. The repository pins the Rust toolchain. A Rust installation and the platform's compiler/linker tools are prerequisites; the launcher does not install system software.

```sh
git clone https://github.com/miguelfem/cookbook.git
cd cookbook
cargo build --locked -p apr-cli
python3 tools/try_local.py
```

Run these commands **on the device where you want to use the platform**. The browser opens a local address. An address created on another computer, including a remotely controlled Mac, does not reach this computer's service.

The launcher copies the synthetic review fixture into a new private temporary project, creates an assignment, registers the process, and starts one review. It also registers a 12-level root → reusable children → review leaf workflow, starts it until the leaf awaits review, and creates and pauses an hourly schedule. The catalog includes a Jev decision-tuning sample which is registered but never executed. It opens the real inspector and prints its local session URL and demo folder. No Jev account, native computer permissions, Python workers, or changes to Codex/Claude configuration are needed.

If the browser does not open automatically, open the printed URL yourself. Use `--no-browser` to print it without launching a browser, or `--apr /absolute/path/to/apr` to use a different build. When using `CARGO_TARGET_DIR`, the launcher respects that directory.

## Explore

1. **Run history:** select the nested root to see its call diagram and `waiting_for_child` state. Open its child, then the leaf, or use **Nested runs and pending reviews** to reach the leaf directly. The leaf shows `waiting_for_agent`, with four of six steps complete. The standalone sample is also available.
2. **Review:** inspect the run's `request.input`. The record is synthetic and its source validation should report `valid: true`. No agent decision has been invented for you.
3. **Finish the sample:** expand **Submit an observed caller result**. The run selection fills the request identity and continuation token. Keep those fields, set `submission_key` to `my-review-1`, set `output` to `{"approved":true}` or `{"approved":false}`, and set `provenance` to `{"source":"human_review","client":"local-inspector"}`. Submit, then refresh the run. It should succeed with a ZIP artifact. This sample packages either answer; it does not implement approval routing.
4. **Scheduled jobs:** inspect **Sample hourly review**, which starts paused. Resume it only when you want recurring demo work; its saved next time determines the first occurrence. A review awaiting an answer prevents another unfinished run of the same job.
5. **Start another run:** click **Discover**, select `local.review-package`, and copy the JSON from the printed `sample-input.json` path into **Input JSON**. Leave configuration as `{}`, enter a new request key, and click **Preflight**, then **Start run**.

The schedule does not open a Codex or Claude conversation to complete reviews. A caller step waits for an explicit result. Connect a client separately through [managed client setup](client-integration.md).

## Close or reopen

Keep the launcher's terminal open while using the interface. Ctrl+C stops that inspector. The independent local service retains the demo state, and the sample schedule remains paused unless you explicitly resumed it. Pause any job you resumed before leaving if you do not want it to run later. No login service is installed.

The launcher prints an `apr inspector --attachment ... --port 0` command to reopen the same assignment. Run it on the same device while the temporary demo files still exist. Refreshing the browser after its session fragment has been cleared requires reopening the printed session URL. Keep that URL private; it grants access to this demo assignment.

Every launcher invocation creates a new demo. Temporary data can be cleared by the operating system; for lasting projects use [client setup](client-integration.md) with an explicit private state directory, and [backup/restore](continue-on-another-device.md) for private data transfers.

The repository contains the launcher, source, and synthetic fixture. It does not contain the earlier remote machine's database, session URL, credentials, or generated client configuration. See [the device handoff](continue-on-another-device.md) to continue implementation from a fresh clone.

For a repeatable nesting check without opening a browser, run `python3 tools/try_local.py --verify-nesting --no-browser`. It performs an explicitly identified synthetic review and verifies root completion, identical duplicate-submission receipts, stable child identities and same-key root replay. It does not invoke a model. [The nested example](../../examples/nested-review/README.md) explains bottom-up registration and the executable test.

The launcher defaults to `--depth 12` and accepts 1–32. Use `--workspace /absolute/private/playground` for a new persistent private instance; a nonempty directory is never overwritten. The printed reopen command uses the same saved assignment. [Capacity controls](capacity.md) explain owner ceilings and larger authored workflows.
