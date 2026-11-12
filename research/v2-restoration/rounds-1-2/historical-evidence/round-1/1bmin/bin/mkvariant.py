#!/usr/bin/env python3
"""Writes bench/out/mine2-shim.js: the change with the clone test hoisted out of `_rebinds`."""
V = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin'
src = open(V + '/bench/out/mine-shim.js').read()
pairs = [
    ("  if (globalThis.__rt._rebinds(dst, d.buf, clone)) {\n    if (down) {",
     "  if (clone === undefined && globalThis.__rt._rebinds(dst, d.buf)) {\n    if (down) {"),
    ("  if (globalThis.__rt._rebinds(dst, d.buf, clone)) {\n    for (let i = 0; i < s.n; i++) d.buf[d.off + i] = s.buf[s.off + i];",
     "  if (clone === undefined && globalThis.__rt._rebinds(dst, d.buf)) {\n    for (let i = 0; i < s.n; i++) d.buf[d.off + i] = s.buf[s.off + i];"),
    ("globalThis.__rt._rebinds = (dst, buf, clone) =>\n  clone === undefined && dst.sc === undefined && !globalThis.__rt._boxed.has(buf);",
     "globalThis.__rt._rebinds = (dst, buf) =>\n  dst.sc === undefined && !globalThis.__rt._boxed.has(buf);"),
]
for old, new in pairs:
    assert src.count(old) == 1, old
    src = src.replace(old, new)
open(V + '/bench/out/mine2-shim.js', 'w').write(src)
print('ok')
