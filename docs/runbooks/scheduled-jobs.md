# Set up scheduled jobs

Build the CLI with `cargo build --locked -p apr-cli`. Set up a project, assignment and private author attachment using the [client integration runbook](client-integration.md). Register the process normally. Scheduling uses the attachment's state directory, so it cannot select a different assignment by passing an ID.

## CLI

Use the actual attachment path and registered process reference in these examples. Input and configuration files contain ordinary JSON, never credentials.

```sh
apr schedule create --attachment /private/state/attachments/author.json \
  --name "Hourly report" --process-ref 'sha256:ACTUAL_PROCESS_DIGEST' \
  --input-file report-input.json --request-key hourly-report-v1 --every 3600

apr schedule create --attachment /private/state/attachments/author.json \
  --name "Weekday review" --process-ref 'sha256:ACTUAL_PROCESS_DIGEST' \
  --input-file review-input.json --request-key weekday-review-v1 --cron '0 15 * * 1-5'

apr schedule create --attachment /private/state/attachments/author.json \
  --name "One-time review" --process-ref 'sha256:ACTUAL_PROCESS_DIGEST' \
  --input-file review-input.json --request-key one-review-v1 --at '2026-09-18T09:00:00-06:00'
```

`--every` is seconds, with its first occurrence one interval from creation. Add `--start-at` with an explicit RFC3339 timezone to set the first interval occurrence. Cron is always UTC: the example is 15:00 UTC on weekdays. APR does not infer the machine's timezone or daylight-saving transitions. Use `--overrides-file` for process-declared ordinary overrides; the resolved configuration is frozen when the schedule is created.

```sh
apr schedule list --attachment /private/state/attachments/author.json
apr schedule inspect SCHEDULE_ID --attachment /private/state/attachments/author.json
apr schedule pause SCHEDULE_ID --attachment /private/state/attachments/author.json
apr schedule resume SCHEDULE_ID --attachment /private/state/attachments/author.json
apr schedule delete SCHEDULE_ID --attachment /private/state/attachments/author.json
```

Creation starts the local service on demand. Closing the CLI or conversation leaves accepted jobs in that service. Reuse the same creation request key after an uncertain response. To change timing, inputs, configuration or execution bindings, delete the old schedule and create a new one with a new key. Pause/delete leave existing runs intact. A one-time schedule's `completed` state means it fired; inspect its `last_occurrence.run_id` for the actual outcome.

## Through an agent or the inspector

An author attachment uses `apr_manage` with `schedule.create`, `schedule.pause`, `schedule.resume`, or `schedule.delete`. `apr_inspect` supports `view:"schedules"` and `view:"schedule"`. There are no extra MCP tools or separate scheduling credentials.

```json
{
  "action": "schedule.create",
  "name": "Hourly report",
  "process_ref": "sha256:ACTUAL_PROCESS_DIGEST",
  "input": {},
  "config_overrides": {},
  "request_key": "hourly-report-v1",
  "trigger": {"kind": "interval", "every_seconds": 3600}
}
```

Other trigger shapes are `{"kind":"once","at_ms":1790000000000}` and `{"kind":"cron","expression":"0 15 * * 1-5"}`. Exact-time wire values are UTC Unix milliseconds. Request `detail:true` when inspecting a full saved template/history. The local inspector shows jobs to both attachment modes; author mode can create a schedule using the selected catalog process and control its future occurrences.

The scheduler cannot open a conversation and make an agent work. A caller-assisted step becomes a durable pending request, visible through the normal inspector/tools. Unfinished work prevents overlapping runs of that job; missed windows do not accumulate.

## Keep the service available after login

The foreground command is `apr --state-dir /absolute/private/state service`. To prepare a user-service file that points to the current executable and existing state directory:

```sh
# macOS: create a new file, review it, then explicitly install it.
apr --state-dir /absolute/private/state schedule service-file \
  --platform launchd --output local.apr.service.plist

# Linux: same behavior for a systemd user service.
apr --state-dir /absolute/private/state schedule service-file \
  --platform systemd --output apr.service
```

These commands write files only. On macOS, place the reviewed plist in `~/Library/LaunchAgents/local.apr.service.plist` and load it with `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/local.apr.service.plist`. On Linux, place the unit in `~/.config/systemd/user/apr.service`, run `systemctl --user daemon-reload`, then `systemctl --user enable --now apr.service`. Choose a stable installed binary path before generating the file. The generated label/unit is for one APR state directory; multiple supervisors need distinct user-service names. An already-running on-demand APR supervisor owns the same state lock; switch supervision when that process has stopped rather than launching a second writer.

The computer must be awake. macOS desktop automation additionally needs a logged-in, unlocked graphical session, approved Screen Recording/Accessibility permissions, and the prepared interpreter. After a restore, reattach the project, configure required grants, and explicitly resume compatible schedules. A schedule frozen to a revoked or replaced binding must be recreated after review.
