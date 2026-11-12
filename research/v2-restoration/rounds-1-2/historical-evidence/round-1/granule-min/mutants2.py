"""Round-2 mutants of runtime/shim.js, written to mut2/<name>.js next to this script.

Each is one change to the finished shim; the committed test that must fail is named in the report.
"""
G = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min'
W = '/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992'
import os

s = open(W + '/runtime/shim.js').read()
os.makedirs(G + '/mut2', exist_ok=True)

NV = '''globalThis.__rt.drop_block_nv = (buf, off, len, cap) => (globalThis.__rt._addr_release(buf),
  globalThis.__rt.convert_block_nv(buf, off, len, cap));'''
STR = '''globalThis.__rt.drop_block_str = (buf, off, len) => (globalThis.__rt._addr_release(buf),
  globalThis.__rt.bytes_str(buf, off, len));'''
CONVERT_NV = '''  for (let i = 0; i < len; i += 1) copy[i] = buf[off + i];
  return { buf: copy, len, cap };'''
BYTES_STR = '''globalThis.__rt.bytes_str = (buf, off, len) => {
  // A zero length `str` reads nothing'''
SV_NV = 'globalThis.__rt.convert_sv_nv = (s) => ({\n'
MINT = '''    base = globalThis.__rt._addrNext;
    if (base + size > 4294967296) {
      throw __rt.abort_error(new Error("rustc_codegen_js: the 32-bit synthetic address space is exhausted"));
    }
    while (base % size !== 0) {
      let j = 4;
      while (j + 1 < k && base % 2 ** (j + 1) === 0) j++;
      free[j].push(base);
      base += 2 ** j;
    }
    // Never hand out the shared zero-sized address (see `addr`).
    if (base === globalThis.__rt._zst_addr) base += 4096;
'''
MINT_BASE = '''    base = globalThis.__rt._addrNext;
    while (base % size !== 0) {
      let j = 4;
      while (j + 1 < k && base % 2 ** (j + 1) === 0) j++;
      free[j].push(base);
      base += 2 ** j;
    }
    // Never hand out the shared zero-sized address (see `addr`).
    if (base === globalThis.__rt._zst_addr) base += 4096;
    if (base + size > 4294967296) {
      throw __rt.abort_error(new Error("rustc_codegen_js: the 32-bit synthetic address space is exhausted"));
    }
'''
RELEASE_PUSH = 'globalThis.__rt._addrFree[k].push(globalThis.__rt._addrs.get(buf));'

mutants = {
    # (a) a drop release forgets the address entry, so a dangling pointer is re-minted
    'a-forget-entry': [(NV, NV.replace('(globalThis.__rt._addr_release(buf),', '(globalThis.__rt._addr_release(buf), globalThis.__rt._addrs.delete(buf),')),
                       (STR, STR.replace('(globalThis.__rt._addr_release(buf),', '(globalThis.__rt._addr_release(buf), globalThis.__rt._addrs.delete(buf),'))],
    # (b) the call-boundary conversion releases (round 1's bug), and the read-only decoders
    'b1-call-vec-releases': [(CONVERT_NV, CONVERT_NV.replace('  return {', '  globalThis.__rt._addr_release(buf);\n  return {'))],
    'b2-bytes-str-releases': [(BYTES_STR, BYTES_STR.replace('=> {\n', '=> {\n  globalThis.__rt._addr_release(buf);\n'))],
    'b3-sv-nv-releases': [(SV_NV, 'globalThis.__rt.convert_sv_nv = (s) => (globalThis.__rt._addr_release(s.buf), {\n')],
    # (c) the refused bump as at base
    'c-refused-bump-as-base': [(MINT, MINT_BASE)],
    # (d) the string drop path skips the release; the vector one keeps it
    'd-str-skips-release': [(STR, STR.replace('(globalThis.__rt._addr_release(buf),\n  ', '(\n  '))],
    # the verifier's M5c and M6a, re-aimed at the round-2 helper
    'm5c-str-pushes-twice': [(STR, STR.replace('(globalThis.__rt._addr_release(buf),',
        '(globalThis.__rt._addrHeap.has(buf) && globalThis.__rt._addrFree[globalThis.__rt._addrHeap.get(buf)].push(globalThis.__rt._addrs.get(buf)), globalThis.__rt._addr_release(buf),'))],
    'm6a-str-ignores-off': [(STR, STR.replace('bytes_str(buf, off, len)', 'bytes_str(buf, 0, len)'))],
    'm5a-release-pushes-twice': [(RELEASE_PUSH, RELEASE_PUSH + '\n  ' + RELEASE_PUSH)],
}
for name, edits in mutants.items():
    text = s
    for old, new in edits:
        assert text.count(old) == 1, (name, old[:60], text.count(old))
        assert old != new, name
        text = text.replace(old, new)
    open(f'{G}/mut2/{name}.js', 'w').write(text)
print(' '.join(mutants))
