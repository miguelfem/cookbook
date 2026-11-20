"""Render retained E02 metrics without treating cheap failures as savings."""
import argparse
import csv
from pathlib import Path

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.ticker import FuncFormatter

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--data', required=True, type=Path)
parser.add_argument('--output', required=True, type=Path)
args = parser.parse_args()
with args.data.open() as source:
    rows = list(csv.DictReader(source))
all_arms = ['direct_luna', 'cookbook_luna_compact', 'direct_sol', 'cookbook_sol_compact']
names = dict(zip(all_arms, ['Direct Luna', 'CookBook Luna', 'Direct Sol', 'CookBook Sol']))
data = {arm: [row for row in rows if row['arm'] == arm and row['phase'] == 'holdout'] for arm in all_arms}
for arm, arm_rows in data.items():
    if len(arm_rows) != 3 or {row['case_id'] for row in arm_rows} != {'page-amber', 'page-cobalt', 'page-linden'}:
        raise ValueError(f'Missing or duplicated cases for {arm}; retain every attempted arm')
quality = {arm: sum(row['exact'].lower() == 'true' and row['source_pages_exact'].lower() == 'true' for row in arm_rows) for arm, arm_rows in data.items()}
# Only compare whole matched pairs whose output and page coverage passed.
# Failures remain visible in the quality banner, report and source CSV.
arms = []
for pair in [all_arms[:2], all_arms[2:]]:
    if all(quality[arm] == 3 for arm in pair):
        arms.extend(pair)
if not arms:
    raise ValueError('No complete matched pair; publish the failure report instead of a savings chart')

def totals(key):
    return [sum(float(row[key]) for row in data[arm]) for arm in arms]

colors = ['#818891' if arm.startswith('direct') else '#245D72' for arm in arms]
plt.rcParams.update({'font.family': 'DejaVu Sans', 'font.size': 11, 'axes.spines.top': False, 'axes.spines.right': False, 'axes.spines.left': False, 'axes.spines.bottom': False, 'axes.titleweight': 'bold', 'figure.facecolor': '#FAF8F3', 'axes.facecolor': '#FAF8F3'})
fig, axes = plt.subplots(2, 2, figsize=(12, 8.6))
fig.subplots_adjust(left=.09, right=.97, top=.79, bottom=.13, hspace=.45, wspace=.28)
fig.suptitle('Prepared reuse: what the client actually consumed', x=.09, y=.97, ha='left', fontsize=21, weight='bold')
fig.text(.09, .922, 'Exact outputs + full page coverage:  ' + ' · '.join(f'{names[arm]} {quality[arm]}/3' for arm in all_arms), fontsize=10.5, color='#47505B')
fig.text(.09, .882, 'Charts show complete matched pairs only. Setup is recorded separately.', fontsize=12, color='#47505B')
for ax in axes.flat:
    ax.set_axisbelow(True)
    ax.grid(axis='y', color='#DDDCD6', linewidth=.6)
    ax.set_xticks(range(len(arms)), [names[arm] for arm in arms])
    ax.tick_params(axis='both', length=0, pad=7)
    ax.yaxis.set_major_formatter(FuncFormatter(lambda value, pos: f'{value:,.0f}'))
ax = axes[0, 0]
cache, fresh = totals('cached_input_tokens'), totals('noncached_input_tokens')
ax.bar(range(len(arms)), cache, color='#C7CBCD', label='Cached input', width=.6)
ax.bar(range(len(arms)), fresh, bottom=cache, color=colors, label='Noncached input', width=.6)
ax.set_title('Input tokens, including cached context', loc='left', pad=15)
ax.legend(frameon=False, fontsize=9, loc='upper left')
ax.set_ylim(0, max(c + n for c, n in zip(cache, fresh)) * 1.33)
for i, (c, n) in enumerate(zip(cache, fresh)):
    ax.text(i, c + n + max(cache) * .025, f'{c + n:,.0f}', ha='center', fontsize=10)
for ax, key, title, fmt in [(axes[0, 1], 'noncached_input_tokens', 'Noncached input tokens', '{:,.0f}'), (axes[1, 0], 'output_tokens', 'Output tokens', '{:,.0f}'), (axes[1, 1], 'wall_seconds', 'Complete client time (seconds)', '{:.2f}')]:
    values = totals(key)
    ax.bar(range(len(arms)), values, color=colors, width=.6)
    ax.set_title(title, loc='left', pad=15)
    ax.set_ylim(0, max(values) * 1.22)
    for i, value in enumerate(values):
        ax.text(i, value + max(values) * .035, fmt.format(value), ha='center', fontsize=10)
fig.text(.09, .042, 'Three reuse cases per arm; low reasoning requested. Failed error paths are excluded from comparisons.\nCached input is part of input. These are observed tokens and time, not billed cost or broad quality evidence.', fontsize=10, color='#47505B')
args.output.parent.mkdir(parents=True, exist_ok=True)
fig.savefig(args.output, dpi=170, facecolor=fig.get_facecolor())
print(args.output)
