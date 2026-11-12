#!/usr/bin/env python3
"""Writes bench/out/mine3-shim.js: the clone test hoisted out of `_rebinds`, and the box test asked
only of a one-element move of an object (a box buffer holds one element, and only an object can be
written into the object already in one)."""
V = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin'
src = open(V + '/bench/out/mine-shim.js').read()
pairs = [
    ("  if (globalThis.__rt._rebinds(dst, d.buf, clone)) {\n    if (down) {",
     "  if (clone === undefined && globalThis.__rt._rebinds(dst, d.buf, s)) {\n    if (down) {"),
    ("  if (globalThis.__rt._rebinds(dst, d.buf, clone)) {\n    for (let i = 0; i < s.n; i++) d.buf[d.off + i] = s.buf[s.off + i];",
     "  if (clone === undefined && globalThis.__rt._rebinds(dst, d.buf, s)) {\n    for (let i = 0; i < s.n; i++) d.buf[d.off + i] = s.buf[s.off + i];"),
    ("globalThis.__rt._rebinds = (dst, buf, clone) =>\n  clone === undefined && dst.sc === undefined && !globalThis.__rt._boxed.has(buf);",
     "globalThis.__rt._rebinds = (dst, buf, s) => {\n  if (dst.sc !== undefined) return false;\n  const first = s.buf[s.off];\n  return s.n !== 1 || first === null || typeof first !== \"object\" || !globalThis.__rt._boxed.has(buf);\n};"),
]
for old, new in pairs:
    assert src.count(old) == 1, old
    src = src.replace(old, new)
open(V + '/bench/out/mine3-shim.js', 'w').write(src)
print('ok')
