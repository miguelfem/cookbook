#!/usr/bin/env python3
"""Render the measured E02 pilot from sanitized data; makes no provider calls."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, required=True)
    parser.add_argument('--setup', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    from matplotlib.ticker import FuncFormatter

    data = json.loads(args.data.read_text())
    setup = {r['condition']: r for r in json.loads(args.setup.read_text())['sessions']}
    conditions = ['direct_agent', 'reusable_script', 'cookbook']
    labels = ['Direct agent', 'Reusable script', 'CookBook']
    groups = [[r for r in data['cases'] if r['condition'] == c] for c in conditions]
    assert all(len(rows) == 3 for rows in groups)
    assert len({(r['case_index'], r['condition']) for r in data['cases']}) == 9
    bg, ink, muted = '#faf8f3', '#252d38', '#656d76'
    blue, orange, pale = '#315d8a', '#bd562f', '#bbcbdc'
    plt.rcParams.update({'font.family': 'DejaVu Sans', 'font.size': 11,
                         'text.color': ink, 'axes.labelcolor': muted,
                         'xtick.color': muted, 'ytick.color': ink,
                         'figure.facecolor': bg, 'axes.facecolor': bg,
                         'axes.edgecolor': '#dddcd6', 'svg.fonttype': 'none'})
    fig, axes = plt.subplots(1, 3, figsize=(14, 5.7), gridspec_kw={'width_ratios': [1, 1.2, 1.4]})
    fig.subplots_adjust(left=.10, right=.985, top=.70, bottom=.28, wspace=.44)
    fig.text(.045, .925, 'CookBook: first measured reuse pilot', fontsize=24, weight='medium')
    fig.text(.045, .865, 'Three matched cases per condition  ·  Codex gpt-5.6-luna  ·  22 September 2026',
             color=muted, fontsize=12)
    accepted = [sum(r['accepted'] for r in g) for g in groups]
    y = [2, 1, 0]
    ax = axes[0]
    ax.set_title('Required output accepted', loc='left', pad=24, fontsize=13)
    ax.barh(y, [3]*3, height=.42, color='#e5e3de')
    ax.barh(y, accepted, height=.42, color=blue)
    for yy, n in zip(y, accepted):
        ax.text(n+.12, yy, f'{n}/3', va='center', fontsize=12, color=ink)
    ax.set_yticks(y, labels)
    ax.set_xlim(0, 3.8)
    ax.set_xticks([0, 1, 2, 3])
    ax.set_xlabel('Cases')
    ax = axes[1]
    ax.set_title('Time per attempt', loc='left', pad=24, fontsize=13)
    for yy, g in zip(y, groups):
        for row in sorted(g, key=lambda r:r['case_index']):
            ax.scatter(row['elapsed_seconds'], yy+(row['case_index']-2)*.10,
                       s=62, marker='o' if row['accepted'] else 'X',
                       color=blue if row['accepted'] else orange, zorder=3)
    ax.set_yticks(y, [])
    ax.set_xlim(0, 80)
    ax.set_xticks([0, 20, 40, 60, 80])
    ax.set_xlabel('Seconds · each mark is one attempt')
    ax.grid(axis='x', color='#e5e3de', linewidth=.7)
    ax = axes[2]
    ax.set_title('Visible client input, all 3 attempts', loc='left', pad=24, fontsize=13)
    total = [sum(r['input_tokens'] for r in g) for g in groups]
    cached = [sum(r['cached_input_tokens'] for r in g) for g in groups]
    uncached = [t-c for t,c in zip(total,cached)]
    ax.barh(y, uncached, height=.42, color=blue, label='Uncached')
    ax.barh(y, cached, left=uncached, height=.42, color=pale, label='Cached (included in total)')
    for yy, n in zip(y,total):
        ax.text(n+22000, yy, f'{n/1e6:.2f}M' if n>=1e6 else f'{n/1e3:.0f}k', va='center', fontsize=11)
    ax.set_yticks(y, [])
    ax.set_xlim(0, 1_430_000)
    ax.set_xticks([0, 500_000, 1_000_000])
    ax.xaxis.set_major_formatter(FuncFormatter(lambda n,_: '0' if n==0 else f'{n/1e6:g}M'))
    ax.set_xlabel('Input tokens · not dollars')
    ax.legend(frameon=False, loc='upper left', bbox_to_anchor=(0,-.22), fontsize=10)
    for ax in axes:
        ax.set_ylim(-.5, 2.5)
        ax.spines[['top','right','left']].set_visible(False)
        ax.tick_params(axis='y', length=0)
    fig.text(.045, .15, '×  CookBook’s failed case stopped at project connection; no recipe ran. Its time and tokens remain included.',
             fontsize=11, color=orange)
    setup_caption = ('Cold setup was separate: direct {direct_agent:.2f} s · script {reusable_script:.2f} s · '
                     'CookBook {cookbook:.2f} s plus manual repair and approval.').format(
                         **{c: setup[c]['elapsed_seconds'] for c in conditions})
    fig.text(.045, .105, setup_caption,
             fontsize=11, color=muted)
    fig.text(.045, .06, 'Small internal pilot. Client billing and reviewer usage are unknown; these observations establish no monetary savings.',
             fontsize=11, color=muted)
    args.output.mkdir(parents=True, exist_ok=True)
    for suffix in ['png', 'svg']:
        fig.savefig(args.output / ('e02-reuse-pilot.'+suffix), dpi=180, facecolor=bg)
    plt.close(fig)
    manifest = {'source_file': args.data.name,
                'source_sha256': hashlib.sha256(args.data.read_bytes()).hexdigest(),
                'setup_sha256': hashlib.sha256(args.setup.read_bytes()).hexdigest(),
                'matplotlib': matplotlib.__version__, 'sample_size':9,
                'accepted_counts':dict(zip(conditions,accepted)),
                'input_tokens_include_cached':True, 'monetary_savings_claim':False}
    (args.output/'chart-provenance.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print(json.dumps(manifest))


if __name__ == '__main__':
    main()
