#!/usr/bin/env python3
"""Bounded real Codex sessions; raw JSONL stays in the private evidence directory."""
import argparse
import datetime as dt
import json
import pathlib
import selectors
import subprocess
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--codex', required=True)
    parser.add_argument('--apr')
    parser.add_argument('--state')
    parser.add_argument('--add-dir', action='append', default=[])
    parser.add_argument('--project', required=True)
    parser.add_argument('--prompt', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--model', default='gpt-5.6-luna')
    parser.add_argument('--seconds', type=int, default=300)
    parser.add_argument('--max-tools', type=int, default=32)
    args = parser.parse_args()
    output = pathlib.Path(args.output)
    output.mkdir(parents=True, exist_ok=False)
    command = [args.codex, 'exec', '--ignore-user-config', '--ephemeral',
               '--skip-git-repo-check', '--json', '--color', 'never',
               '--approve-for-me', '-C', args.project, '-m', args.model,
               '-c', 'model_reasoning_effort="low"',
               '-o', str(output/'final.txt')]
    if args.apr:
        command += ['-c', 'mcp_servers.cookbook.command='+json.dumps(args.apr),
                    '-c', 'mcp_servers.cookbook.args='+json.dumps(
                        ['--state-dir', args.state, 'mcp', '--projects'])]
    for directory in args.add_dir:
        command += ['--add-dir', directory]
    command += ['-']
    started = time.perf_counter()
    summary = {'started_utc':dt.datetime.now(dt.timezone.utc).isoformat(),
               'model':args.model, 'reasoning_effort':'low',
               'client_approval_mode':'approve-for-me; workspace-write',
               'usage':None, 'tool_calls':[], 'stop_reason':None,
               'provider_cost_usd':None, 'client_billed_cost_usd':None}
    with (output/'events.jsonl').open('w') as log, (output/'stderr.log').open('w') as err, (output/'event-times.jsonl').open('w') as timing:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=err, text=True, bufsize=1)
        process.stdin.write(pathlib.Path(args.prompt).read_text())
        process.stdin.close()
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        count = 0
        starts = {}
        while process.poll() is None:
            if time.perf_counter()-started > args.seconds:
                summary['stop_reason']='wall_clock_limit'
                process.terminate()
                break
            for key, _ in selector.select(timeout=0.25):
                line = key.fileobj.readline()
                if not line:
                    continue
                log.write(line)
                log.flush()
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                elapsed=time.perf_counter()-started
                item=event.get('item',{})
                timing.write(json.dumps({'elapsed_seconds':elapsed,'event':event.get('type'),
                    'item_id':item.get('id'),'item_type':item.get('type'),'tool':item.get('tool')})+'\n')
                timing.flush()
                if event.get('type') == 'item.started':
                    starts[item.get('id')]=elapsed
                if event.get('type') == 'turn.completed':
                    summary['usage'] = event.get('usage')
                if event.get('type') == 'item.completed':
                    item = event.get('item', {})
                    if item.get('type') in ('command_execution','mcp_tool_call','web_search'):
                        count += 1
                        summary['tool_calls'].append({'type':item.get('type'),
                            'tool':item.get('tool'), 'status':item.get('status'),
                            'exit_code':item.get('exit_code'),
                            'elapsed_seconds':elapsed-starts[item.get('id')] if item.get('id') in starts else None,
                            'result_bytes':len(json.dumps(item).encode())})
                        if count >= args.max_tools:
                            summary['stop_reason']='tool_call_limit'
                            process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        rest = process.stdout.read()
        log.write(rest)
        # A process can exit before the reader observes its final usage event.
        for line in rest.splitlines():
            try:
                event=json.loads(line)
                if event.get('type') == 'turn.completed':
                    summary['usage']=event.get('usage')
            except json.JSONDecodeError:
                pass
        selector.close()
    summary.update(exit_code=process.returncode, elapsed_seconds=time.perf_counter()-started,
                   event_bytes=(output/'events.jsonl').stat().st_size)
    (output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(json.dumps({k:v for k,v in summary.items() if k!='tool_calls'}))


if __name__ == '__main__':
    main()
