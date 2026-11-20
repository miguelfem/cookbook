# Show a prepared report without a model turn

Use a prepared CookBook recipe that returns integer counts in `summary` and an
immutable JSON artifact reference in `report`. The helper calls the existing
managed MCP connection through Codex App Server, verifies that the same project
can read the artifact, and exports it into a new private directory. It writes
a Markdown view with only the counts and a link to the complete report.

The connection must already be installed with owner approval scoped to
`apr_run`. The recipe must be registered in the chosen workspace and its inputs
and execution grants ready. The helper uses a temporary App Server session and
runs without starting a model turn. The Markdown output can be opened in Codex.
Automatic inline MCP display is a separate client capability.

```sh
python3 tools/direct_report.py \
  --project /absolute/project \
  --process-ref sha256:REPLACE_WITH_REGISTERED_VERSION \
  --input-file /absolute/prepared-input.json \
  --request-key unique-report-request \
  --output /absolute/new-result-directory
```

Use a new request key for new logical work. Reuse the same one when retrying
the same input after an interrupted export. This example exports authorized JSON
reports up to 1 MiB. The export verifies the stored digest and refuses an
existing destination.

The installed-build check used a synthetic 300-row ticket SLA report. One
registered recipe run yielded 300 checked, 100 breached, and 50 unanswered.
The verified export contained all 300 rows in order, including a report larger
than the scoped inline preview budget. The full path took 0.58 seconds without
a model turn. The Codex configuration file matched byte for byte before and
after the test. The synthetic assignment was closed.
