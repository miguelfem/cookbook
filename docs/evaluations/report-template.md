# Evaluation report template

Template only; no result is implied. Copy this into a dated report and replace each field. Use **unknown** for unavailable measurements and **not applicable** only with a reason. Launch reports are maintainer-run on the unsigned macOS ARM64 package with the client binaries actually available on the test device; they do not require recruited participants or an outside reviewer. Follow the [plan](../plans/evaluations.md) and [measurement rules](../architecture/measurement.md).

## Question and decision

- Study ID, date and owner/reviewer:
- Question and task/domain:
- Status: not run / in progress / complete / inconclusive / stopped:
- Launch scope: macOS ARM64 package, device, available client binaries and owned accounts:
- Unavailable clients, platforms or capabilities recorded as untested:
- Observed conclusion and the decision it supports:
- What this report does not establish:

## Frozen protocol

- Protocol revision/hash and date frozen:
- Baseline(s), candidate and why the comparison is fair:
- Planned samples, repetitions and stopping/exclusion rules:
- Frozen acceptance criteria and quality rubric; internal review procedure:
- Maximum spend, authorized accounts and live/synthetic scope:
- Development/holdout construction, entity overlap checks and label custody:
- Counterbalancing, context/cache controls and grading/blinding method:
- Deviations from the registered protocol, including when and why:

## Reproduction

| Item | Recorded value |
| --- | --- |
| Git commit and dirty-source fingerprint | To record |
| App/package version and checksum | To record |
| OS, architecture and relevant prerequisites | To record |
| Client version, model/reasoning settings | To record |
| External provider/model requested and reported; unresolved aliases | To record |
| Recipe/script/schema/config/policy hashes | To record |
| Input/dataset versions and hashes | To record |
| Enabled features and permitted operations, without secrets | To record |
| Exact procedure/commands and timing method | To record |
| Redacted aggregate data and redistributable fixture locations | To record |

## Observed results

Add one row per comparison condition. Report counts before percentages and the variation across cases/repetitions. Leave measured fields unknown until observed.

| Condition | Attempted / accepted | Quality failures and repairs | Setup/authoring | Repeat execution cost | Time and human effort |
| --- | --- | --- | --- | --- | --- |
| To record | Unknown | Unknown | Unknown | Unknown | Unknown |

- Required-output/evidence coverage and internal review disagreements:
- All visible caller/subagent/provider tokens, cache usage and missing telemetry:
- Total attributable cost, including failed attempts; estimate vs provider-reported amount:
- Cost per accepted result and denominator; undefined if none accepted:
- Observed cumulative cost and separately labeled break-even estimate, if supported:
- Human setup, approvals, corrections and reconciliation:
- Runtime/provider/model/human waiting time and sampling limits:
- Exclusions, timeouts, stopped or uncertain calls and their costs:
- Recovery/replay or authority checks, kept separate from ordinary performance trials:

## Artifacts and review

- Reproducible commands and synthetic fixtures:
- Reviewed aggregate dataset and chart source, if available:
- Remaining unknowns and task/client/platform limits:
- Independent review outcome and evidence checked:
- Adopt / expand / fix / stop recommendation and next specific experiment:

Before adding this report to the [results index](README.md), remove credentials, raw private content, device-local attachment files and owner/account identifiers that are not needed to reproduce the result. Record any device or client recordings only when the owner has permission to publish them. Retain failures and inconclusive findings; a report is not a marketing-only subset of the experiment.
