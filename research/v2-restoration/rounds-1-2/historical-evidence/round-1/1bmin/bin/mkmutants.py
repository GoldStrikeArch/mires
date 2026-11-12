#!/usr/bin/env python3
"""Writes runtime mutant shims of the element-move rule into 1bmin/mutants/."""
import os
W = '/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef'
V = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin'
os.makedirs(V + '/mutants', exist_ok=True)
src = open(W + '/runtime/shim.js').read()
old = ("globalThis.__rt._rebinds = (dst, buf, s) => {\n"
       "  if (dst.sc !== undefined) return false;\n"
       "  const first = s.buf[s.off];\n"
       "  return s.n !== 1 || first === null || typeof first !== \"object\" ||\n"
       "    !globalThis.__rt._boxed.has(buf);\n"
       "};")
assert src.count(old) == 1
gate = "  if (clone === undefined && globalThis.__rt._rebinds(dst, d.buf, s)) {\n"
assert src.count(gate) == 2
muts = {
    # the rule reverted: every element move writes through again (the integration behaviour)
    'M1_never_rebind': (old, "globalThis.__rt._rebinds = (dst, buf, s) => false;"),
    # a box buffer rebinds too (1b's rule)
    'M2_rebind_boxes': (old, "globalThis.__rt._rebinds = (dst, buf, s) => dst.sc === undefined;"),
    # a byte view rebinds too
    'M2b_rebind_scaled': (old, "globalThis.__rt._rebinds = (dst, buf, s) => {\n  const first = s.buf[s.off];\n  return s.n !== 1 || first === null || typeof first !== \"object\" ||\n    !globalThis.__rt._boxed.has(buf);\n};"),
    # by-value aggregates (a `clone`) rebind too, uncloned
    'M2c_rebind_always': (gate, "  if (globalThis.__rt._rebinds(dst, d.buf, s)) {\n"),
}
for name, (a, b) in muts.items():
    open(f'{V}/mutants/{name}.shim.js', 'w').write(src.replace(a, b))
    print(name)
