#!/usr/bin/env python3
"""Read E01 evidence only; export selected metrics without paths or MCP secrets."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import sqlite3


SESSIONS = (
    'e01-first-session', 'e01-second-session', 'e01-adapter-session',
    'e01-recovery-session', 'e01-adapter-use-session',
    'e01-recovery-corrected-session',
)


def error_code(item):
    result = item.get('result') or {}
    structured = result.get('structured_content') or {}
    if isinstance(structured, dict) and isinstance(structured.get('error'), dict):
        return structured['error'].get('code')
    if any('failed to deserialize parameters' in c.get('text', '')
           for c in result.get('content', []) if isinstance(c, dict)):
        return 'PARAMETER_DESERIALIZATION'
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--state', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    rows = []
    calls = []
    for name in SESSIONS:
        summary = json.loads((args.evidence / name / 'summary.json').read_text())
        usage = summary.get('usage') or {}
        errors = []
        for line in (args.evidence / name / 'events.jsonl').read_text().splitlines():
            event = json.loads(line)
            item = event.get('item', {})
            if event.get('type') != 'item.completed' or item.get('type') != 'mcp_tool_call':
                continue
            code = error_code(item)
            if code:
                errors.append(code)
            calls.append({'session': name, 'tool': item.get('tool'), 'error_code': code})
        rows.append({
            'session': name, 'elapsed_seconds': summary['elapsed_seconds'],
            'tool_calls': len(summary['tool_calls']),
            'stop_reason': summary['stop_reason'], 'exit_code': summary['exit_code'],
            'input_tokens': usage.get('input_tokens'),
            'cached_input_tokens': usage.get('cached_input_tokens'),
            'output_tokens': usage.get('output_tokens'),
            'reasoning_output_tokens': usage.get('reasoning_output_tokens'),
            'mcp_error_count': len(errors), 'event_bytes': summary['event_bytes'],
            'client_billed_cost_usd': None,
        })
    db = sqlite3.connect((args.state / 'state.sqlite3').resolve().as_uri() + '?mode=ro', uri=True)
    db.row_factory = sqlite3.Row
    runs = [dict(row) for row in db.execute('SELECT id,state,version,output FROM runs ORDER BY created_at')]
    protocol = json.loads((args.evidence / 'e01-protocol.json').read_text())
    expected = protocol['first_notes'] + ['Second project note']
    observed = []
    for run in runs:
        output = json.loads(run.pop('output')) if run['output'] else None
        run['output_kind'] = 'note_artifact' if isinstance(output, dict) and 'saved_note' in output else 'caller_choice'
        if run['output_kind'] == 'note_artifact':
            ref = output['saved_note']
            data = (args.state / 'objects/sha256' / ref['sha256']).read_bytes()
            assert hashlib.sha256(data).hexdigest() == ref['sha256']
            observed.append(json.loads(data))
            run['artifact_hash_verified'] = True
        elif output is not None:
            run['expected_choice_verified'] = output.get('choice') == 'A'
    assert observed == expected, 'Saved-note artifacts differ from frozen inputs'
    assert len(runs) == 5 and sum(r['state'] == 'succeeded' for r in runs) == 4
    assert db.execute('SELECT count(*) FROM effect_intents').fetchone()[0] == 0
    result = {
        'study': 'E01', 'status': 'incomplete_local_acceptance',
        'protocol_sha256': hashlib.sha256((args.evidence / 'e01-protocol.json').read_bytes()).hexdigest(),
        'sessions': rows, 'mcp_calls': calls, 'runtime_runs': runs,
        'saved_notes_exact': 3, 'saved_notes_attempted': 3,
        'provider_dispatches': 0, 'client_cost_complete': False,
        'owner_approval_surface': 'CLI in disposable state; no native approval-flow claim',
        'labels_exported': False,
    }
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / 'metrics.json').write_text(json.dumps(result, indent=2) + '\n')
    with (args.output / 'sessions.csv').open('w', newline='') as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]), lineterminator='\n')
        writer.writeheader()
        writer.writerows(rows)
    print(json.dumps({'sessions': len(rows), 'runs': len(runs), 'notes_verified': len(observed),
                      'unknown_usage_sessions': sum(r['input_tokens'] is None for r in rows)}))


if __name__ == '__main__':
    main()
