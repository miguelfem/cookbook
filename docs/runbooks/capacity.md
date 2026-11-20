# Workflow capacity

CookBook APR separates requested workflow budgets, the device owner's acceptance ceilings, and hard implementation bounds. The root has depth zero; decisions within a process do not add nesting. Each child call adds one level.

| Resource | New authoring / owner default | Hard maximum |
|---|---:|---:|
| Call depth | 32 | 128 |
| Child runs per root | 1,024 | 4,096 |
| Step instances per root | 10,000 | 100,000 |
| Distinct reachable process versions | 4,096 (owner policy only) | 4,096 |

New generated nested examples and agent guidance explicitly write the authoring defaults into v0.2 manifests. Omitted fields in older or hand-authored manifests retain their original meanings: depth 8, children 256, steps 10,000. Raising owner policy alone does not change a workflow definition. Register a new version with larger explicit limits and update its callers or schedules deliberately.

## Inspect and change policy

Use the same private state directory as your running service:

```sh
apr --state-dir /absolute/private/state capacity show
apr --state-dir /absolute/private/state capacity set \
  --max-call-depth 64 \
  --max-child-runs 2048 \
  --max-step-instances 20000 \
  --max-graph-versions 4096
```

All four flags are required so an update is explicit and atomic. Invalid settings leave the previous policy intact. SQLite stores the policy; the service observes changes without a restart. This command is an owner operation, absent from attachment authoring and MCP mutation tools. An agent may inspect `apr_inspect` with `{"view":"capacity","detail":true}` and request a change from the owner. These are APR interface boundaries; arbitrary same-user native code is not an OS sandbox.

New root requests and new scheduled occurrences must fit the policy. A denied start creates no partial root or children. Idempotent replay returns the already accepted run even if policy was lowered. Accepted work retains its limits, including across pause/restart/resume. To stop accepted work use explicit pause or cancel. A previously created schedule whose future occurrence no longer fits pauses with an error; adjust the policy or recreate its reviewed definition, then explicitly resume the job.

## Simultaneous jobs

Workflow size and worker count are separate. A recipe can queue many jobs while
only a chosen number run at once. In **Settings → Execution**, choose 4, 8, 16,
32, or **Custom** from 1 to 64. The initial setting is four. A separate local-script
limit also defaults to four; raise it when you want more than four local jobs.
Optional connection limits use the approved connection name and apply across
projects using that name. Distinct aliases do not share a provider-wide cap.

Increasing these settings affects future claims without a restart. Lowering them
lets active jobs finish before admitting more. Recipe item limits, dependencies,
provider quotas and shared cost/call budgets remain enforced. These settings do
not approve tools or change permissions. The card shows running jobs and an
approximate count of queued steps, which may include dependencies and control
steps rather than immediately runnable jobs.

Owner CLI controls use a reviewed revision to reject stale edits:

```sh
apr --state-dir /absolute/private/state execution worker-policy-show
apr --state-dir /absolute/private/state execution worker-status
apr --state-dir /absolute/private/state execution worker-policy-set \
  --policy-file /absolute/path/worker-policy.json --expected-revision 0
```

An example policy matching revision zero is:

```json
{"revision":0,"max_workers":32,"max_local_workers":8,"per_connection":{"image-api":4}}
```

Read the current revision before each change and include that value in both the
file and command. A successful save increments it. Agents can inspect
`apr_inspect` with `{"view":"workers","detail":true}`; counts and named limits
are scoped to their assignment. No MCP mutation changes this owner policy.

64 is an implementation ceiling, not a guarantee that any machine or provider
will sustain that load. Native scripts consume memory/CPU; APIs may throttle.
The bounded pool fills each free slot as soon as an eligible job can run.

## Routing graphs and executed work

A graph may hold many mutually exclusive alternatives without reserving a child run for every alternative. The graph ceiling counts every distinct referenced version, including unused alternatives. Execution bounds count the largest workload among routes of the same decision, sum independent decisions conservatively, and include all stored top-level step rows even when skipped. Repeated calls and foreach consume execution budgets per actual instance. Runtime counters still prevent overspending under siblings, retries and provider work.

All child targets and authority remain explicit and frozen before acceptance. Cycles are rejected. Larger depth does not enable unbounded loops or model-selected arbitrary process targets. Keep payloads flat across wrappers; nested workflow depth is independent of the JSON depth and transport byte limits.

## Try the deeper example

```sh
cargo build --locked -p apr-cli
python3 tools/try_local.py --depth 12
python3 tools/try_local.py --depth 32 --verify-nesting --no-browser
```

The demo has a saved caller review at the leaf. Follow **Nested runs and pending reviews**, submit that leaf's own observed result, then refresh the root. The harness's verification mode uses an explicitly labeled synthetic result and checks stable child identities and duplicate receipts. It performs no provider inference.

For persistent local testing, use `--workspace /absolute/private/playground` with a new or empty directory. Existing nonempty directories are refused. Keep the terminal open; the printed inspector command reopens the saved assignment later. See [local demo](local-demo.md).
