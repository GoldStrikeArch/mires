#!/usr/bin/env python3
"""Writes bench/out/mine4-shim.js from the BASE shim: the same rule, spelled as a choice of element
store (`_put` or `_rebind`) so the copy loops stay exactly what they were."""
V = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin'
src = open(V + '/bench/out/base-shim.js').read()
pairs = [
    # copy
    ("  const each = clone === undefined ? (v) => v : clone;\n  const put = globalThis.__rt._put;\n  if (d.buf === s.buf && d.off > s.off) {",
     "  const each = clone === undefined ? (v) => v : clone;\n  const put = clone === undefined && globalThis.__rt._rebinds(dst, d.buf, s)\n    ? globalThis.__rt._rebind\n    : globalThis.__rt._put;\n  if (d.buf === s.buf && d.off > s.off) {"),
    # copy_nonoverlapping
    ("  const each = clone === undefined ? (v) => v : clone;\n  for (let i = 0; i < s.n; i++) globalThis.__rt._put(d.buf, d.off + i, each(s.buf[s.off + i]));",
     "  const each = clone === undefined ? (v) => v : clone;\n  const put = clone === undefined && globalThis.__rt._rebinds(dst, d.buf, s)\n    ? globalThis.__rt._rebind\n    : globalThis.__rt._put;\n  for (let i = 0; i < s.n; i++) put(d.buf, d.off + i, each(s.buf[s.off + i]));"),
    # the two helpers, before `_put`
    ("// The one element store the copies share, and the one place where storing is\n// not an assignment.\n",
     "globalThis.__rt._rebinds = (dst, buf, s) => {\n  if (dst.sc !== undefined) return false;\n  const first = s.buf[s.off];\n  return s.n !== 1 || first === null || typeof first !== \"object\" || !globalThis.__rt._boxed.has(buf);\n};\n\nglobalThis.__rt._rebind = (buf, off, value) => {\n  buf[off] = value;\n};\n\n// The one element store the copies share, and the one place where storing is\n// not an assignment.\n"),
]
for old, new in pairs:
    assert src.count(old) == 1, old
    src = src.replace(old, new)
open(V + '/bench/out/mine4-shim.js', 'w').write(src)
print('ok')
