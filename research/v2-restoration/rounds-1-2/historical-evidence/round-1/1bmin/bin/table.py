#!/usr/bin/env python3
"""table.py [armA] [armB] [modes...]: per-fixture wrong-line table, armA (base) vs armB (mine), every set and mode.
Wrong line = line i of .expected that the arm's output does not reproduce (missing lines count as wrong).
Prints REGRESSED (right in A, wrong in B) and FIXED lines explicitly."""
import sys, os, glob
V = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin'
args = sys.argv[1:]
A = args[0] if len(args) > 0 else 'base'
B = args[1] if len(args) > 1 else 'mine'
modes = args[2:] or ['rb', 'rbopt0', 'rbco', 'jir0', 'def']
sets = ['r1', 'r3new', 'fuse']

def lines(p):
    try:
        return open(p, errors='replace').read().split('\n')[:-1]
    except FileNotFoundError:
        return None

def status(arm, mode, s, n, E):
    js = f'{V}/out/{arm}/{mode}/{s}-{n}.js'
    rc = lines(js + '.rc')
    if rc is None:
        return None, 'NOT-RUN'
    if rc and rc[0] == 'COMPILE-FAILED':
        return None, 'CF'
    out = lines(js + '.out') or []
    bad = [i + 1 for i in range(len(E)) if i >= len(out) or out[i] != E[i]]
    extra = len(out) > len(E)
    return bad, ('rc=' + rc[0] if rc and rc[0] != '0' else '') + ('+extra' if extra else '')

tot = {'A': 0, 'B': 0, 'reg': 0, 'fix': 0}
regressions = []
rows = []
for s in sets:
    for p in sorted(glob.glob(f'{V}/corpus/{s}/*.rs')):
        n = os.path.basename(p)[:-3]
        if n == 'prelude':
            continue
        E = lines(f'{V}/corpus/{s}/{n}.expected')
        cells = []
        for m in modes:
            ba, na = status(A, m, s, n, E)
            bb, nb = status(B, m, s, n, E)
            def show(bad, note):
                if bad is None:
                    return note
                return f'{len(bad)}' + (f'[{note}]' if note else '')
            reg = fix = []
            if ba is not None and bb is not None:
                reg = [i for i in bb if i not in ba]
                fix = [i for i in ba if i not in bb]
                tot['A'] += len(ba); tot['B'] += len(bb); tot['reg'] += len(reg); tot['fix'] += len(fix)
            elif ba is not None and bb is None:
                reg = ['ALL(compile/run)']
            if reg:
                regressions.append(f'{s}/{n} {m}: REGRESSED lines {reg}')
            mark = ' !!' if reg else (' +' if fix else '')
            cells.append(f'{show(ba, na)}->{show(bb, nb)}{mark}')
        rows.append((f'{s}/{n}', len(E), cells))
w = max(len(r[0]) for r in rows)
print(f'{"fixture":{w}s} {"lines":>5s}  ' + '  '.join(f'{m:>14s}' for m in modes))
for name, nl, cells in rows:
    print(f'{name:{w}s} {nl:5d}  ' + '  '.join(f'{c:>14s}' for c in cells))
print()
print(f'TOTAL wrong lines {A}={tot["A"]} {B}={tot["B"]}  regressed={tot["reg"]} fixed={tot["fix"]}')
for r in regressions:
    print(r)
