// The synthetic address space (`__rt.addr`, `_addr_mint`): a usize is 32 bits, so bases must be
// given back and carved by size, or a long-running program's millionth allocation is reported as
// a failed one (`addr(p, 1) >>> 0 === 0` in `NonNull::new`).
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'

const source = await readFile(new URL('../runtime/shim.js', import.meta.url), 'utf8')
function runtime(code = source) {
  const context = vm.createContext({ TextEncoder, TextDecoder, Uint8Array })
  vm.runInContext(code, context)
  return context.__rt
}
// `NonNull::new` on a fresh allocation, exactly as the emitted code spells it.
const nonNull = (rt, p) => rt.addr(p, 1) >>> 0

// Mints well over 2^20 small blocks with interleaved frees, holding a sliding window of live
// blocks, and checks every invariant the pointer model promises for each one.
function churn(rt, total, window, sizes) {
  const live = []
  const owner = new Map()
  let maxAddress = 0
  for (let i = 0; i < total; i++) {
    const size = sizes[i % sizes.length]
    const p = rt.alloc(size, 1)
    const address = nonNull(rt, p)
    if (address === 0) throw new Error(`allocation ${i} of ${size} bytes got the null address`)
    if (address !== rt.addr(p, 1)) throw new Error(`allocation ${i} wrapped past 2^32: ${rt.addr(p, 1)}`)
    if (address < 4096) throw new Error(`allocation ${i} collides with the dangling range: ${address}`)
    if (address === rt._zst_addr) throw new Error(`allocation ${i} was given the zero-sized address`)
    const granule = Math.min(4096, Math.max(16, 2 ** Math.ceil(Math.log2(size))))
    if (address % granule !== 0) throw new Error(`allocation ${i} of ${size} bytes at ${address} is not ${granule}-aligned`)
    if (owner.has(address)) throw new Error(`allocation ${i} duplicates a live block's address ${address}`)
    owner.set(address, p)
    live.push([p, address])
    maxAddress = Math.max(maxAddress, address)
    // Interleaved frees: usually the oldest, sometimes a random live block.
    while (live.length > window) {
      const at = i % 7 === 0 ? (i * 2654435761 >>> 0) % live.length : 0
      const [q, a] = live[at]
      live[at] = live[live.length - 1]
      live.pop()
      if (rt.addr(q, 1) !== a) throw new Error('a live block\'s address moved')
      owner.delete(a)
      rt.dealloc(q, 0, 1)
    }
  }
  for (const [p, a] of live) assert.equal(rt.addr(p, 1), a, 'stable for the block\'s lifetime')
  return { maxAddress, live: live.length }
}

test('more than 2^20 small allocations with interleaved frees: no wrap, no null, no live duplicate', () => {
  const rt = runtime()
  const { maxAddress } = churn(rt, (1 << 20) + 50000, 300, [17, 24, 8, 64, 100, 1, 4096, 33])
  // Reuse keeps the space compact: the window is 300 blocks, so the top stays tiny.
  assert.ok(maxAddress < 64 * 1024 * 1024, `top of the address space reached ${maxAddress}`)
})

test('a large live set of small blocks fits: granules are sized, not 4096 bytes each', () => {
  const rt = runtime()
  // 1.1 million simultaneously live 16-byte blocks would need 4.5 GB of address space at a
  // 4096-byte granule; at 16 bytes they take 18 MB.
  const { maxAddress, live } = churn(rt, 1_100_000, 1_100_000, [16, 12, 8])
  assert.equal(live, 1_100_000)
  assert.ok(maxAddress < 2 ** 26, `top reached ${maxAddress}`)
})

test('the minting this replaced wraps to null after 2^20 blocks', () => {
  const old = source.replace(
    /base = globalThis\.__rt\._addr_mint\(key, [^;]*;/,
    'base = 4096 * (globalThis.__rt._addrOld = (globalThis.__rt._addrOld || 0) + 1);' +
      ' if (base === globalThis.__rt._zst_addr) base = 4096 * ++globalThis.__rt._addrOld;' +
      ' globalThis.__rt._addrs.set(key, base);',
  )
  assert.notEqual(old, source)
  assert.throws(() => churn(runtime(old), (1 << 20) + 10, 300, [17]), /null address|wrapped/)
})

test('a freed block keeps its address for dangling pointers, and the address is reused', () => {
  const rt = runtime()
  const a = rt.alloc(24, 8)
  const address = nonNull(rt, a)
  const element = rt.addr({ buf: a.buf, off: 3 }, 1)
  assert.equal(element, address + 3)
  rt.dealloc(a, 24, 8)
  assert.equal(rt.addr(a, 1), address, 'a dangling pointer answers the address it had')
  const b = rt.alloc(20, 4)
  assert.equal(nonNull(rt, b), address, 'the freed granule is reused, as a native allocator would')
  const c = rt.alloc(20, 4)
  assert.notEqual(nonNull(rt, c), address)
})

test('a block realloc grows past its granule re-mints instead of overlapping its neighbours', () => {
  const rt = runtime()
  const v = rt.alloc(8, 1)
  const before = nonNull(rt, v)
  const neighbour = rt.alloc(16, 1)
  const next = nonNull(rt, neighbour)
  assert.equal(next, before + 16)
  // Growing within the granule keeps the address.
  const same = rt.realloc(v, 8, 1, 16)
  assert.equal(rt.addr(same, 1), before)
  // Growing past it moves the address (a reallocation may move) to a granule of the new size.
  const grown = rt.realloc(same, 16, 1, 100)
  const after = nonNull(rt, grown)
  assert.notEqual(after, before)
  assert.equal(after % 128, 0)
  assert.ok(after + 100 <= next || after >= next + 16, 'no overlap with the live neighbour')
  assert.equal(rt.addr({ buf: grown.buf, off: 99 }, 1), after + 99)
  // The old granule is free again.
  assert.equal(nonNull(rt, rt.alloc(16, 1)), before)
})

test('alignment follows the block size; elements stay multiples of their size', () => {
  const rt = runtime()
  for (const [bytes, align] of [[1, 16], [16, 16], [17, 32], [64, 64], [100, 128], [4096, 4096], [10000, 4096]]) {
    const p = rt.alloc(bytes, 1)
    assert.equal(nonNull(rt, p) % align, 0, `${bytes} bytes`)
  }
  // A non-heap buffer (a stack array) is sized by its length and the element size asked.
  const words = [1, 2, 3, 4, 5, 6, 7, 8]
  const at = rt.addr({ buf: words, off: 2 }, 8)
  assert.equal((at - 16) % 64, 0)
  assert.equal(at % 8, 0)
})

test('the bump never hands out the zero-sized address', () => {
  const rt = runtime()
  rt._addrNext = rt._zst_addr - 16
  assert.equal(nonNull(rt, rt.alloc(16, 1)), rt._zst_addr - 16)
  assert.equal(nonNull(rt, rt.alloc(16, 1)), rt._zst_addr + 4096)
})

test('the alignment gap the bump leaves is reused, not lost', () => {
  const rt = runtime()
  const small = nonNull(rt, rt.alloc(16, 1))
  assert.equal(small, 4096)
  const page = nonNull(rt, rt.alloc(4096, 1))
  assert.equal(page, 8192)
  // [4112, 8192) was carved into 16, 32, ..., 2048-byte free granules.
  assert.equal(nonNull(rt, rt.alloc(2048, 1)), 4096 + 2048)
  assert.equal(nonNull(rt, rt.alloc(16, 1)), 4112)
})

test('the top of the space is used to its last granule, then refused; freed granules still serve', () => {
  const rt = runtime()
  rt._addrNext = 2 ** 32 - 32
  const a = rt.alloc(16, 1)
  assert.deepEqual([nonNull(rt, a), nonNull(rt, rt.alloc(16, 1))], [2 ** 32 - 32, 2 ** 32 - 16])
  assert.throws(() => nonNull(rt, rt.alloc(16, 1)), /address space is exhausted/)
  rt.dealloc(a, 16, 1)
  assert.equal(nonNull(rt, rt.alloc(16, 1)), 2 ** 32 - 32, 'a freed granule is reused even at the top')
})

// The allocator's own state, for comparing before and after.
const freeLists = (rt) => JSON.stringify(rt._addrFree)

test('a refused bump leaves the free lists as it found them, so a retry hands out no address twice', () => {
  const rt = runtime()
  // A 16-byte block has just taken the first granule of the last page.
  rt._addrNext = 2 ** 32 - 4096 + 16
  const before = freeLists(rt)
  for (let attempt = 0; attempt < 3; attempt++) {
    assert.throws(() => nonNull(rt, rt.alloc(4096, 1)), /address space is exhausted/)
    assert.equal(freeLists(rt), before, `refusal ${attempt} changed the free lists`)
    assert.equal(rt._addrNext, 2 ** 32 - 4096 + 16, `refusal ${attempt} moved the bump pointer`)
  }
  // The rest of the last page is 255 granules of 16 bytes, each handed out exactly once.
  const seen = new Set()
  for (let i = 0; i < 255; i++) {
    const address = nonNull(rt, rt.alloc(16, 1))
    assert.ok(!seen.has(address), `address ${address} handed to two live blocks`)
    assert.ok(address >= 2 ** 32 - 4096 + 16 && address < 2 ** 32, `address ${address} outside the page`)
    seen.add(address)
  }
  assert.throws(() => nonNull(rt, rt.alloc(16, 1)), /address space is exhausted/)
})

test('the refused-bump retry, reached through alloc and addr alone: no live duplicate', () => {
  const rt = runtime()
  const live = []
  const owner = new Set()
  const take = (bytes) => {
    const p = rt.alloc(bytes, 1)
    const address = nonNull(rt, p)
    assert.ok(!owner.has(address), `address ${address} handed to two live blocks`)
    owner.add(address)
    live.push(p)
    return address
  }
  // Every page below the last one, kept alive. A 4096-byte block is a register call, not an
  // array of 4096 holes: only its Layout size reaches the address space.
  for (;;) {
    const p = rt._register([], 4096, false)
    const address = nonNull(rt, p)
    live.push(p)
    owner.add(address)
    if (address === 2 ** 32 - 8192) break
  }
  take(16)
  assert.throws(() => take(4096), /address space is exhausted/)
  for (let i = 0; i < 255; i++) take(16)
  assert.throws(() => take(16), /address space is exhausted/)
  assert.ok(live.length > 2 ** 20 - 2)
})

// A baseline String's bytes as the String adapters hand them over: a registered byte block of
// `cap` bytes, the text's UTF-8 bytes at `off` (zero unless asked), behind `off` filler bytes.
function stringBlock(rt, text, cap = 64, off = 0) {
  const bytes = new TextEncoder().encode(text)
  const p = rt.alloc(Math.max(cap, off + bytes.length), 1)
  for (let i = 0; i < off; i++) p.buf[i] = 0x3f
  bytes.forEach((byte, i) => { p.buf[off + i] = byte })
  return { p, off, len: bytes.length }
}

// The conversions out of a baseline block, as the adapters spell them: at a call boundary (the
// value lives on in its new carrier) and at a drop cut (the value's allocation ends).
const atCall = {
  vec: (rt, buf, off, len, cap) => rt.convert_block_nv(buf, off, len, cap),
  str: (rt, buf, off, len) => rt.bytes_str(buf, off, len),
}
const atDrop = {
  vec: (rt, buf, off, len, cap) => rt.drop_block_nv(buf, off, len, cap),
  str: (rt, buf, off, len) => rt.drop_block_str(buf, off, len),
}

test('a Vec dropped through its native carrier gives the granule back, as dealloc does', () => {
  const rt = runtime()
  const v = rt.alloc(100, 1)
  v.buf[0] = 7
  v.buf[1] = 8
  const address = nonNull(rt, v)
  const element = rt.addr({ buf: v.buf, off: 9 }, 1)
  const native = atDrop.vec(rt, v.buf, 1, 1, 100)
  assert.deepEqual([native.len, native.cap, native.buf.length, native.buf[0]], [1, 100, 100, 8])
  assert.equal(rt.addr(v, 1), address, 'a dangling pointer answers the address it had')
  assert.equal(rt.addr({ buf: v.buf, off: 9 }, 1), element)
  assert.equal(nonNull(rt, rt.alloc(100, 1)), address, 'the granule serves the next block of its size')
  assert.equal(rt.addr(v, 1), address, 'and the dangling pointer still answers the same address')
  assert.notEqual(nonNull(rt, rt.alloc(100, 1)), address, 'the granule is handed out once')
})

test('a String dropped through its native carrier decodes at its offset and gives the granule back', () => {
  const rt = runtime()
  for (const off of [0, 3]) {
    for (const text of ['', 'ascii only', 'Grüße, 世界 🦀', 'x'.repeat(5000)]) {
      const { p, len } = stringBlock(rt, text, 64, off)
      const address = nonNull(rt, p)
      assert.equal(atDrop.str(rt, p.buf, off, len), text)
      assert.equal(rt.addr(p, 1), address, 'a dangling pointer answers the address it had')
      const next = stringBlock(rt, 'next', Math.max(64, off + len))
      assert.equal(nonNull(rt, next.p), address, `the granule of ${JSON.stringify(text.slice(0, 12))} is free again`)
      assert.equal(rt.addr(p, 1), address)
      assert.notEqual(nonNull(rt, stringBlock(rt, 'third', Math.max(64, off + len)).p), address)
    }
  }
  // A String that never allocated: its data pointer is `NonNull::dangling()`, a bare number.
  const dangling = 1
  assert.equal(atDrop.str(rt, dangling.buf, dangling.off, 0), '')
})

test('a conversion at a call boundary keeps the granule: the value lives on in its new carrier', () => {
  const rt = runtime()
  // `make` observed the buffer's address; the Vec (or String) then moved into a callee whose
  // parameter is native. Natively the moved value is still that allocation, so no block made
  // while it lives may be given its address.
  const v = rt.alloc(100, 1)
  const vAddress = nonNull(rt, v)
  const s = stringBlock(rt, 'moved', 100)
  const sAddress = nonNull(rt, s.p)
  assert.notEqual(vAddress, sAddress)
  const lists = freeLists(rt)
  const native = atCall.vec(rt, v.buf, 0, 0, 100)
  assert.equal(atCall.str(rt, s.p.buf, 0, s.len), 'moved')
  assert.equal(freeLists(rt), lists, 'a call-boundary conversion gave a granule back')
  for (let i = 0; i < 8; i++) {
    const other = nonNull(rt, rt.alloc(100, 1))
    assert.ok(other !== vAddress && other !== sAddress, `a moved value's address ${other} was handed out again`)
  }
  assert.equal(native.cap, 100)
})

test('a converted block whose address was never asked costs nothing: no mint, no entry', () => {
  const rt = runtime()
  nonNull(rt, rt.alloc(24, 1)) // something minted already, so the free lists are not empty
  const blocks = [rt.alloc(100, 1), rt.alloc(100, 1)]
  const strings = [stringBlock(rt, 'never asked'), stringBlock(rt, 'never asked')]
  const lists = freeLists(rt)
  const next = rt._addrNext
  atDrop.vec(rt, blocks[0].buf, 0, 0, 100)
  atCall.vec(rt, blocks[1].buf, 0, 0, 100)
  atDrop.str(rt, strings[0].p.buf, 0, strings[0].len)
  atCall.str(rt, strings[1].p.buf, 0, strings[1].len)
  assert.equal(freeLists(rt), lists)
  assert.equal(rt._addrNext, next)
  for (const buf of [...blocks.map((b) => b.buf), ...strings.map((s) => s.p.buf)]) {
    assert.equal(rt._addrs.has(buf) || rt._addrHeap.has(buf), false)
  }
})

test('a block dropped through its native carrier gives its granule back once, whatever releases it again', () => {
  for (const kind of ['vec', 'str']) {
    // Once: the next two blocks of the size are given two different addresses.
    const rt = runtime()
    const v = rt.alloc(48, 1)
    const address = nonNull(rt, v)
    atDrop[kind](rt, v.buf, 0, 0, 48)
    const a = nonNull(rt, rt.alloc(48, 1))
    const b = nonNull(rt, rt.alloc(48, 1))
    assert.deepEqual([a === address, b !== address, a !== b], [true, true, true], kind)
    // Released again, by itself and by a `dealloc`: still one granule.
    const again = runtime()
    const w = again.alloc(48, 1)
    const there = nonNull(again, w)
    atDrop[kind](again, w.buf, 0, 0, 48)
    atDrop[kind](again, w.buf, 0, 0, 48)
    again.dealloc(w, 48, 1)
    const c = nonNull(again, again.alloc(48, 1))
    const d = nonNull(again, again.alloc(48, 1))
    assert.deepEqual([c === there, d !== there], [true, true], `${kind}, released three times`)
  }
})

test('a conversion that only reads a live block keeps its granule', () => {
  const rt = runtime()
  // Both live blocks are addressed first, so neither can be the other's freed granule.
  const s = stringBlock(rt, 'still alive')
  const sAddress = nonNull(rt, s.p)
  const v = rt.alloc(64, 1)
  v.buf.fill(3, 0, 8)
  const vAddress = nonNull(rt, v)
  assert.notEqual(sAddress, vAddress)
  // `bytes_str` decodes bytes a live String still owns (`as_str`, `from_utf8_unchecked`, a
  // `&str` from raw parts); `convert_sv_nv` copies a slice view's window out of the block of the
  // Vec it borrows from.
  assert.equal(rt.bytes_str(s.p.buf, s.p.off, s.len), 'still alive')
  const copy = rt.convert_sv_nv({ buf: v.buf, off: 2, len: 4 })
  assert.deepEqual([copy.len, Array.from(copy.buf)], [4, [3, 3, 3, 3]])
  for (let i = 0; i < 8; i++) {
    const other = nonNull(rt, rt.alloc(64, 1))
    assert.ok(other !== sAddress && other !== vAddress, `a live block's address ${other} was handed out again`)
  }
  assert.equal(rt.addr(s.p, 1), sAddress)
  assert.equal(rt.addr(v, 1), vAddress)
})

test('three million rounds of allocate, ask the address, drop through a native carrier: no exhaustion, no live duplicate', () => {
  const rt = runtime()
  const window = 64
  const live = []
  const owner = new Set()
  const text = new TextEncoder().encode('round')
  for (let i = 0; i < 3_000_000; i++) {
    // A page-sized block takes the largest granule, so a leak would exhaust the space after
    // about 2^20 rounds; the backing array is short because only its Layout size matters here.
    const p = rt._register(Array.from(text), 4096, false)
    const address = nonNull(rt, p)
    if (address === 0 || owner.has(address)) throw new Error(`round ${i}: address ${address} is null or live`)
    owner.add(address)
    live.push([p, address])
    if (live.length > window) {
      const [q, a] = live.shift()
      owner.delete(a)
      if (i % 2 === 0) atDrop.vec(rt, q.buf, 0, text.length, text.length)
      else if (atDrop.str(rt, q.buf, 0, text.length) !== 'round') throw new Error(`round ${i}: decode`)
      if (rt.addr(q, 1) !== a) throw new Error(`round ${i}: a dangling pointer changed its address`)
    }
  }
  assert.ok(rt._addrNext < 2 ** 24, `the bump pointer reached ${rt._addrNext}`)
})

test('the conversions read and write nothing on user objects, whatever their field names', () => {
  const rt = runtime()
  // Every name the allocator and the conversions use, as the fields of a user struct.
  const names = ['buf', 'off', 'len', 'cap', 'text', 'copy', 'k', 'base', 'gap', 'size', 'bytes',
    'es', 'sc', 'fresh', 'zeroed', 'length', '_addrs', '_addrHeap', '_addrFree', '_addrNext']
  const touched = []
  const trap = (kind) => (target, key, ...rest) => {
    touched.push(`${kind} ${String(key)}`)
    return Reflect[kind](target, key, ...rest)
  }
  const user = () => new Proxy(Object.fromEntries(names.map((name, i) => [name, i + 1])), {
    get: trap('get'), set: trap('set'), has: trap('has'), defineProperty: trap('defineProperty'),
    deleteProperty: trap('deleteProperty'), getOwnPropertyDescriptor: trap('getOwnPropertyDescriptor'),
    ownKeys: (target) => { touched.push('ownKeys'); return Reflect.ownKeys(target) },
    getPrototypeOf: (target) => { touched.push('getPrototypeOf'); return Reflect.getPrototypeOf(target) },
  })
  // A `Vec` of four 48-byte structs, already element-typed: its elements are the objects.
  const elements = [user(), user(), user(), user()]
  const v = rt._register(elements.slice(), 4 * 48, false)
  const address = nonNull(rt, v)
  const native = atDrop.vec(rt, v.buf, 0, 4, 4)
  // A user object handed to the release itself is only ever a key.
  const lists = freeLists(rt)
  rt._addr_release(user())
  assert.equal(freeLists(rt), lists)
  assert.deepEqual(touched, [], 'a user object was read or written')
  assert.ok(native.buf.length === 4 && native.buf.every((element, i) => element === elements[i]))
  assert.equal(rt.addr(v, 48), address)
  assert.equal(nonNull(rt, rt.alloc(4 * 48, 1)), address)
})
