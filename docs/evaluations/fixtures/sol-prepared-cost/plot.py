"""Plot retained aggregate data; requires Matplotlib, never executes model calls."""
import json
from pathlib import Path
import matplotlib.pyplot as plt
from matplotlib.ticker import FuncFormatter

ROOT=Path(__file__).resolve().parent
d=json.loads((ROOT/'metrics.json').read_text())
arms=['direct','script','cookbook']
names=['Direct agent','Prepared script','CookBook']
colors=['#747f8e','#31816c','#334d79']
fig,(ax,bx)=plt.subplots(1,2,figsize=(13,5.8),gridspec_kw={'width_ratios':[1.4,1]})
fig.patch.set_facecolor('#faf9f6')
for a in (ax,bx):
 a.set_facecolor('#faf9f6')
 a.spines[['top','right','left']].set_visible(False)
 a.spines['bottom'].set_color('#d2d6d8')
 a.tick_params(colors='#4a4f54',length=0,pad=9)
 a.grid(axis='y',color='#e4e5e3',linewidth=.7)
 a.set_axisbelow(True)
for arm,name,col in zip(arms,names,colors):
 rows=[r for r in d['rows'] if r['arm']==arm]
 ax.plot(range(6),[r['api_equivalent_usd'] for r in rows],marker='o',lw=1.8,label=name,color=col,markersize=6)
ax.set_xticks(range(6),['Cedar 1','Harbor 1','Quartz 1','Cedar 2','Harbor 2','Quartz 2'],rotation=30,ha='right')
ax.set_ylim(.018,.040)
ax.yaxis.set_major_formatter(FuncFormatter(lambda y,_:f'${y:.3f}'))
ax.set_title('Cost per completed 32-item package',loc='left',fontsize=13,pad=19)
ax.legend(frameon=False,loc='upper right',fontsize=10)
bx.set_title('Six packages · quality tied at 6/6',loc='left',fontsize=13,pad=19)
ys=[d['aggregate'][a]['api_equivalent_usd'] for a in arms]
bx.bar(range(3),ys,color=colors,width=.6)
bx.set_xticks(range(3),['Direct','Script','CookBook'])
bx.set_ylim(0,.20)
bx.yaxis.set_major_formatter(FuncFormatter(lambda y,_:f'${y:.2f}'))
for i,y in enumerate(ys):bx.text(i,y+.004,f'${y:.3f}',ha='center',fontsize=12,color='#252d33')
fig.suptitle('GPT-6 Sol · Medium · prepared reuse',x=.055,y=.98,ha='left',fontsize=20,color='#252d33')
fig.text(.055,.035,'API-equivalent Standard token valuation, not an observed bill. Setup excluded. Three synthetic batches × two repeats.',fontsize=10,color='#545e64')
fig.text(.055,.08,'83% of the CookBook–direct cost difference comes from the first pair. No subjective quality claim.',fontsize=10,color='#545e64')
fig.subplots_adjust(left=.06,right=.97,top=.81,bottom=.25,wspace=.28)
fig.savefig(ROOT/'sol-cost.png',dpi=180,facecolor=fig.get_facecolor())
fig.savefig(ROOT/'sol-cost.svg',facecolor=fig.get_facecolor())
