"""Optional first-use Codex probes with raw CSV, task rules and no provided script."""
import json, shutil, sys
from pathlib import Path
from benchmark_codex_head_to_head import app_server, canonical, fixture, parse_reply, save

root=Path(sys.argv[1])
root.mkdir(mode=0o700)
rules={
 "inventory": """Input fields id,warehouse,expected,counted. Compute delta as counted minus expected.
 Keep one row per input in original order with fields id, warehouse, expected, counted, delta, status.
 Status is 'short' for negative delta, 'over' for positive delta and 'match' otherwise.
 Return summary keys checked, mismatched, net_delta.""",
 "sla": """Input fields id,queue,priority,opened_min,first_response_min. The SLA limit in minutes is P1=30,
 P2=120, P3=480. Blank first response is unanswered. Otherwise compare elapsed minutes (response minus opened) to
 the SLA limit. Breach means strictly greater than the limit. Keep all rows in original order with fields id, queue,
 priority, elapsed_minutes (null for unanswered), limit_minutes, status ('unanswered', 'breach' or 'within').
 Return summary keys checked, breached, unanswered.""",
}
rows=[]
for task,rule in rules.items():
    work=root/task
    path,expected,details,_=fixture(task,144,work)
    prompt=f"""Process the synthetic CSV file {path.name}. Use the terminal and write or run any local code you need.
{rule}
Write the full report to report.json as an object with a single rows array.
Reply with the summary counts as plain JSON. No network."""
    with (work/"stderr.log").open("x") as log, app_server("codex",work,{},log) as client:
        thread=client.thread(work,"gpt-6-sol")
        status=client.call("mcpServerStatus/list",{"threadId":thread,"detail":"toolsAndAuthOnly"})
        assert not any(server.get("tools") for server in status["data"])
        final,calls,row=client.turn(thread,prompt)
    report=work/"report.json"
    try:
        exact=json.loads(report.read_text())=={"rows":details}
    except (OSError,ValueError):
        exact=False
    row.update(task=task,arm="codex_from_scratch",input_rows=144,tool_output_exact=exact,
               chat_reply_exact=parse_reply(final)==expected)
    rows.append(row)
    save(root/"metrics.json",rows)
    print(json.dumps(row),flush=True)
