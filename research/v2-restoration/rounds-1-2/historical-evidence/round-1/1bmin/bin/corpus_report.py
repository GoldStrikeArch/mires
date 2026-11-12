#!/usr/bin/env python3
"""corpus_report.py <tag>: per workload, the Rust arms' p50 and min across the alternating runs of
each arm, the final/base ratios, the A/A control (rust-a/rust-b) per arm, and the load averages."""
import glob, json, statistics, sys
V = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin'
tag = sys.argv[1]
data = {}
loads = {}
for path in sorted(glob.glob(f'{V}/corpus-runs/{tag}/*.json')):
    arm = path.split('/')[-1].split('-')[0]
    d = json.load(open(path))
    for row in d['timing']['rows']:
        if row.get('implementation') != 'rust':
            continue
        key = (row['workload'], row['arm'])
        data.setdefault(key, {}).setdefault(arm, []).append((row['p50'], row['min'], row['checksum']))
        loads.setdefault(arm, []).append(row['loadAverage']['start'][0])
print(f'{"workload/arm":24s} {"n":>3s} {"base p50 med":>12s} {"final p50 med":>13s} {"ratio":>6s} | {"base min":>9s} {"final min":>9s} {"ratio":>6s}  checksums')
for (workload, arm), arms in sorted(data.items()):
    b, f = arms.get('base', []), arms.get('final3', arms.get('final', []))
    if not b or not f:
        continue
    bp, fp = statistics.median(x[0] for x in b), statistics.median(x[0] for x in f)
    bm, fm = min(x[1] for x in b), min(x[1] for x in f)
    sums = {x[2] for x in b + f}
    print(f'{workload + "/" + arm:24s} {len(b):>3d} {bp:12.3f} {fp:13.3f} {fp / bp:6.3f} | {bm:9.3f} {fm:9.3f} {fm / bm:6.3f}  {"equal" if len(sums) == 1 else "DIFFER"}')
for arm, ls in loads.items():
    print(f'load average (1 min) at row start, {arm}: min {min(ls):.1f} median {statistics.median(ls):.1f} max {max(ls):.1f}')
