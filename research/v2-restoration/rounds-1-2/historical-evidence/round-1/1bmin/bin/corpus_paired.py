#!/usr/bin/env python3
"""corpus_paired.py <tag>: the harness's paired rust/js ratio (median per-round ratio, which cancels
machine-wide slowdowns that hit both implementations in a round) and the A/A ratio, per run and arm."""
import glob, json, statistics, sys
V = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin'
tag = sys.argv[1]
cells = {}
for path in sorted(glob.glob(f'{V}/corpus-runs/{tag}/*.json')):
    arm = path.split('/')[-1].split('-')[0]
    d = json.load(open(path))
    for c in d['timing']['comparisons']:
        cells.setdefault((c['workload'], c['comparison']), {}).setdefault(arm, []).append(c['pairedRatio'])
print(f'{"workload":12s} {"comparison":10s} {"base paired (runs)":34s} {"final paired (runs)":34s} final/base(median)')
for (w, comp), arms in sorted(cells.items()):
    b, f = arms.get('base', []), arms.get('final3', arms.get('final', []))
    fmt = lambda xs: ' '.join(f'{x:.2f}' for x in xs)
    r = statistics.median(f) / statistics.median(b) if b and f else float('nan')
    print(f'{w:12s} {comp:10s} {fmt(b):34s} {fmt(f):34s} {r:.3f}')
