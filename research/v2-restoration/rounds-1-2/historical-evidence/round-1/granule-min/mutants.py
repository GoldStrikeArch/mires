"""Writes the four mutant shims (a)-(d) next to this script, from the finished shim."""
G = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min'
W = '/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992'
s = open(W + '/runtime/shim.js').read()


def mutate(text, old, new, count=1):
    assert text.count(old) == count, (old, text.count(old))
    return text.replace(old, new)


# (a) the release forgets the base: a dangling pointer is given a new address when asked again.
a = mutate(s, '  globalThis.__rt._addr_release(buf);\n  return { buf: copy, len, cap };',
           '  globalThis.__rt._addr_release(buf);\n  globalThis.__rt._addrs.delete(buf);\n  return { buf: copy, len, cap };')
a = mutate(a, '  globalThis.__rt._addr_release(buf);\n  return text;',
           '  globalThis.__rt._addr_release(buf);\n  globalThis.__rt._addrs.delete(buf);\n  return text;')
# (b) a read-only conversion releases: the slice view's window copy gives back the block it borrows.
b = mutate(s, 'globalThis.__rt.convert_sv_nv = (s) => ({\n',
           'globalThis.__rt.convert_sv_nv = (s) => (globalThis.__rt._addr_release(s.buf), {\n')
# (c) the refused bump as it was: the gap is carved before the refusal is decided.
c = mutate(s, '''    let gap = globalThis.__rt._addrNext;
    base = gap + (size - gap % size) % size;
    // Never hand out the shared zero-sized address (see `addr`).
    if (base === globalThis.__rt._zst_addr) base += 4096;
    if (base + size > 4294967296) {
      throw __rt.abort_error(new Error("rustc_codegen_js: the 32-bit synthetic address space is exhausted"));
    }
    while (gap % size !== 0) {
      let j = 4;
      while (j + 1 < k && gap % 2 ** (j + 1) === 0) j++;
      free[j].push(gap);
      gap += 2 ** j;
    }
''', '''    base = globalThis.__rt._addrNext;
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
''')
# (d) the string path skips the release; the vector path keeps it.
d = mutate(s, '  const text = globalThis.__rt.bytes_str(buf, off, len);\n  globalThis.__rt._addr_release(buf);\n',
           '  const text = globalThis.__rt.bytes_str(buf, off, len);\n')
for name, text in [('a', a), ('b', b), ('c', c), ('d', d)]:
    open(f'{G}/mut/{name}.js', 'w').write(text)
print('wrote a b c d')
