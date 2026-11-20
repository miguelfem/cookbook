# A reusable review inside a reusable delivery process

This synthetic example runs **document delivery → reusable review layers → local document review**. The default uses twelve nested calls: the root is depth 0 and the review leaf is depth 12. Only the leaf asks the caller to review the supplied record. Its committed decision and ZIP artifact return through every parent without expanding the output shape. No Jev account or native execution grant is needed.

Run the real interface from the repository root:

```sh
cargo build --locked -p apr-cli
python3 tools/try_local.py
```

The launcher includes a standalone review and a paused hourly schedule, plus this nested workflow. Select `nested.document-delivery` in run history. Follow child runs to the pending `local.review-package` leaf, or find its **Review needed** entry in the run family. Submit the leaf's saved handoff. All parents finish automatically after the leaf succeeds. Submit to the **leaf run ID**, not its waiting parent. Refresh to see the completed root's final `approved` and `package` outputs.

## Author once, call one process

[register.py](register.py) writes the requested chain of manifest 0.2 definitions in the demo's private attached project and registers them bottom-up. The existing [local review](../local-review/process.json) is registered first. Each parent pins the exact `sha256:` reference returned for its child; no placeholder hash or device state is committed here. Changing a child requires registering it and its affected parents again. Already accepted runs retain the original graph.

Agents use the ordinary `apr_manage` registration, `apr_discover`, `apr_run`, `apr_inspect`, and `apr_resume` tools. A reused root needs only its input contract in the agent's context. Child definitions execute within APR; retrieve a child schema or full handoff only when it is needed. This avoids enumerating every internal operation as a separate exposed tool. It does not claim measured token savings or arbitrary code execution composition.

Use `--depth N` to choose 1–32 call levels (default 12). New demo manifests explicitly declare a depth allowance of 32, a shared child allowance of 1,024, and a shared step allowance of 10,000. These are declarations, not authority: the runtime also enforces owner capacity and never raises it through this helper. Previously registered manifests that omit these fields retain their legacy meanings. The registration helper can generate up to 128 levels for explicitly owner-configured workflows, but the launcher stays within the default owner depth capacity. Each wrapper returns the same flat `approved` and `package` fields; payload JSON does not become deeper with each call.

## Explore Jev tuning without running Jev

The launcher also registers `demo.jev-intake-routing` in the private project, based on the existing [Jev intake fixture](../jev-intake/process.json). Discover it in the process catalog to see a question node, a confidence-based choice decision, and synthetic operations/engineering/unclear branches. Select the Jev question or routing node to edit supported instructions, criteria, confidence threshold, and fallback routes; preview before publishing a separate immutable version.

This sample is **registered but never started** by the launcher. Viewing, previewing, and publishing its definition work without a provider account. No connection, grant, credential, or inference request is created. Live execution requires separately building/configuring the optional Jev adapter and an authorized protected connection. The synthetic branch steps only return data; they do not send tickets or perform business actions.

## Reproduce the nested handoff verification

```sh
python3 tools/try_local.py --verify-nesting --no-browser
```

This starts real CLI and inspector subprocesses against a fresh private project. It reaches the leaf, explicitly submits a **synthetic fixture answer**, replays that exact submission, waits for the root result, and retries the root start with the same key. It dynamically traverses the actual child chain, checks the paginated saved family, matching parent identities, unchanged child IDs, matching final package references, and one accepted receipt. It also confirms that the sample schedule remains paused. Verification closes the inspector; private saved demo state remains, as with the normal launcher. The supplied answer tests execution, not review quality or a live agent journey.

Use `--apr /absolute/path/to/apr` or `CARGO_TARGET_DIR` to select the build. Demo manifests, handoff tokens, input artifact references, and SQLite state stay in the launcher's printed private directory. No owner capacity, client configuration, login service, or credential binding is installed.

## Keep a local instance

```sh
python3 tools/try_local.py --workspace ~/.cookbook-apr-playground --depth 12
```

The supplied directory must be new or empty and owned by the current user. The launcher sets directory permissions to private and refuses nonempty destinations or symlinks; it never overwrites an existing instance. Without `--workspace`, it creates disposable temporary storage. Keep persistent instances outside the source checkout.

After closing the inspector, reopen the same saved instance with the exact `apr inspector --attachment ... --port 0` command printed by the launcher. Re-running the launcher against that nonempty directory is deliberately refused. Closing the inspector keeps the independent service and saved runs available. The demo's recurring job starts paused; resume it only when intended, and pause it again before leaving if it should not continue.
