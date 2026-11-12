// The synthetic address space (`__rt.addr`, `_addr_mint`): a usize is 32 bits, so bases must be
// given back and carved by size, or a long-running program's millionth allocation is reported as
// a failed one (`addr(p, 1) >>> 0 === 0` in `NonNull::new`).
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'

const source = await readFile(new URL('../runtime/shim.js', import.meta.url), 'utf8')
function runtime(code = source) {
  const context = vm.createContext({})
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
    const granule = Math.max(16, 2 ** Math.ceil(Math.log2(size)))
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
  for (const [bytes, align] of [[1, 16], [16, 16], [17, 32], [64, 64], [100, 128], [4096, 4096], [10000, 16384],
    [61287, 65536], [1 << 20, 1 << 20]]) {
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

// ---------------------------------------------------------------------------------------------
// Blocks larger than a page. Every byte of a live block is its own: `[base, base + granule)` is
// exclusively the block's, so element addresses never meet another live block's.
// ---------------------------------------------------------------------------------------------

const MB = 1 << 20
const granuleOf = (bytes) => Math.max(16, 2 ** Math.ceil(Math.log2(Math.max(1, bytes))))
// A heap block of `bytes` whose backing store is a stand-in rather than an array of that length:
// the allocator sizes a block by its `Layout` (`block.bytes`), never by its array, and a thousand
// real one-megabyte arrays would hold 8 GB.
const block = (rt, bytes) => rt._register({ length: 0 }, bytes, false)

function assertDisjoint(ranges, what) {
  const sorted = [...ranges].sort((a, b) => a[0] - b[0])
  for (let i = 1; i < sorted.length; i++) {
    if (sorted[i - 1][0] + sorted[i - 1][1] > sorted[i][0]) {
      throw new Error(`${what}: [${sorted[i - 1]}] overlaps [${sorted[i]}]`)
    }
  }
}

test('a 61,287-byte block owns its whole range: no element of it is a later block\'s address', () => {
  // The first element of the big block whose address lands in the small one, or null.
  const collide = (rt) => {
    const big = rt.alloc(61287, 1)
    const start = nonNull(rt, big)
    const small = rt.alloc(4, 4)
    const base = rt.addr(small, 4)
    for (let i = 0; i < 61287; i++) {
      const at = rt.addr({ buf: big.buf, off: i }, 1)
      if (at !== start + i) return `element ${i} moved`
      if (at >= base && at < base + 4) return `element ${i} of the big block is at ${at}, inside the small block at ${base}`
    }
    return null
  }
  assert.equal(collide(runtime()), null)
  // The granule cap this replaced handed the 61 KB block 4096 bytes, and the next block sat at its
  // byte 4096.
  const capped = source.replace('while (k < 33 && 2 ** k < bytes) k++;', 'while (k < 12 && 2 ** k < bytes) k++;')
  assert.notEqual(capped, source)
  assert.match(String(collide(runtime(capped))), /element 4096 of the big block is at 8192, inside the small block at 8192/)
})

test('a block that grows past a page takes a granule of its new size; shrinking keeps it', () => {
  const rt = runtime()
  const v = rt.alloc(4096, 1)
  const before = nonNull(rt, v)
  const neighbour = rt.alloc(16, 1)
  const next = nonNull(rt, neighbour)
  const grown = rt.realloc(v, 4096, 1, 70000)
  const after = nonNull(rt, grown)
  assert.notEqual(after, before)
  assert.equal(after % 131072, 0)
  assert.ok(after + 131072 <= next || after >= next + 16, 'no overlap with the live neighbour')
  assert.equal(rt.addr({ buf: grown.buf, off: 69999 }, 1), after + 69999)
  const shrunk = rt.realloc(grown, 70000, 1, 100)
  assert.equal(nonNull(rt, shrunk), after)
  // The page it left is free again.
  const pages = [rt.alloc(4096, 1), rt.alloc(4096, 1)].map((p) => nonNull(rt, p))
  assert.ok(pages.includes(before), `${before} is not among ${pages}`)
})

test('random sizes up to 2^20, interleaved frees and reallocs: live granules stay pairwise disjoint', () => {
  const rt = runtime()
  let seed = 0x9e3779b9
  const random = () => {
    seed ^= seed << 13
    seed ^= seed >>> 17
    seed ^= seed << 5
    return (seed >>> 0) / 2 ** 32
  }
  // Half uniform in 1..2^20, half log-uniform, so every granule size is exercised.
  const size = () => random() < 0.5
    ? 1 + Math.floor(random() * MB)
    : Math.max(1, Math.floor(2 ** (random() * 20)))
  const live = []
  let top = 0
  let peak = 0
  const admit = (entry, who) => {
    const { base, granule } = entry
    if (base < 4096 || base + granule > 2 ** 32) throw new Error(`${who}: ${base} is outside the space`)
    if (base % granule !== 0) throw new Error(`${who}: ${base} is not ${granule}-aligned`)
    if (base <= rt._zst_addr && rt._zst_addr < base + granule) throw new Error(`${who}: covers the zero-sized address`)
    for (const other of live) {
      if (other !== entry && base < other.base + other.granule && other.base < base + granule) {
        throw new Error(`${who}: [${base}, +${granule}) overlaps a live [${other.base}, +${other.granule})`)
      }
    }
    const last = rt.addr({ buf: entry.p.buf, off: entry.bytes - 1 }, 1)
    if (last !== base + entry.bytes - 1) throw new Error(`${who}: last byte at ${last}`)
  }
  for (let op = 0; op < 60000; op++) {
    const r = random()
    if (live.length < 16 || (r < 0.45 && live.length < 400)) {
      const bytes = size()
      const p = block(rt, bytes)
      const entry = { p, bytes, base: nonNull(rt, p), granule: granuleOf(bytes) }
      admit(entry, `op ${op}: a fresh ${bytes}-byte block`)
      live.push(entry)
    } else if (r < 0.8) {
      const at = Math.floor(random() * live.length)
      const entry = live[at]
      assert.equal(rt.addr(entry.p, 1), entry.base, 'stable while live')
      live[at] = live[live.length - 1]
      live.pop()
      rt.dealloc(entry.p, entry.bytes, 1)
    } else {
      const entry = live[Math.floor(random() * live.length)]
      const bytes = size()
      entry.p = rt.realloc(entry.p, entry.bytes, 1, bytes)
      const base = nonNull(rt, entry.p)
      if (bytes <= entry.granule) {
        assert.equal(base, entry.base, 'a reallocation within the granule keeps the address')
      } else {
        entry.granule = granuleOf(bytes)
      }
      entry.bytes = bytes
      entry.base = base
      admit(entry, `op ${op}: a block reallocated to ${bytes} bytes`)
    }
    top = Math.max(top, rt._addrNext)
    peak = Math.max(peak, live.reduce((sum, entry) => sum + entry.granule, 0))
    if (op % 5000 === 0) assertDisjoint(live.map((entry) => [entry.base, entry.granule]), `op ${op}`)
  }
  assertDisjoint(live.map((entry) => [entry.base, entry.granule]), 'at the end')
  // Freed space is reused: the top stays within a small multiple of the largest live footprint.
  assert.ok(top < 2 * peak + 64 * MB, `top ${top} against a peak live footprint of ${peak}`)
})

test('1,000 one-megabyte blocks allocated and freed cyclically reuse their addresses', () => {
  const rt = runtime()
  let first
  let top
  for (let cycle = 0; cycle < 20; cycle++) {
    const blocks = Array.from({ length: 1000 }, () => block(rt, MB))
    const bases = blocks.map((p) => nonNull(rt, p))
    for (const base of bases) assert.equal(base % MB, 0)
    assertDisjoint(bases.map((base) => [base, MB]), `cycle ${cycle}`)
    if (cycle === 0) {
      first = new Set(bases)
      top = rt._addrNext
    } else {
      assert.equal(rt._addrNext, top, `cycle ${cycle} took fresh address space`)
      for (const base of bases) assert.ok(first.has(base), `cycle ${cycle} used ${base}, which cycle 0 did not`)
    }
    for (const p of blocks) rt.dealloc(p, MB, 1)
  }
  // 1000 MB of granules, the megabyte below the first, and the megabyte at the zero-sized address.
  assert.ok(top <= 1002 * MB, `top ${top}`)
  // One block at a time, some of them real arrays: freed and reused in place.
  const once = rt.alloc(MB, 1)
  const at = nonNull(rt, once)
  rt.dealloc(once, MB, 1)
  for (let i = 0; i < 1000; i++) {
    const p = i % 100 === 0 ? rt.alloc(MB, 1) : block(rt, MB)
    assert.equal(nonNull(rt, p), at)
    rt.dealloc(p, MB, 1)
  }
  assert.equal(rt._addrNext, top)
})

test('space freed at one size serves the next: large granules merge and split', () => {
  const rt = runtime()
  const phase = (count, bytes) => {
    const blocks = Array.from({ length: count }, () => block(rt, bytes))
    const bases = blocks.map((p) => nonNull(rt, p))
    assertDisjoint(bases.map((base) => [base, granuleOf(bytes)]), `${count} x ${bytes}`)
    for (const p of blocks) rt.dealloc(p, bytes, 1)
  }
  phase(1000, MB)
  const top = rt._addrNext
  // Without merging, each of these phases would need a fresh gigabyte, and the fourth would not
  // fit in 2^32.
  phase(500, 2 * MB)
  phase(250, 4 * MB)
  phase(120, 8 * MB)
  phase(4000, MB / 4)
  phase(1000, MB / 2 + 1)
  phase(1000, MB)
  phase(200000, 24)
  phase(1000, MB)
  assert.ok(rt._addrNext - top <= 64 * MB, `the top grew from ${top} to ${rt._addrNext}`)
})

test('a collected buffer gives its granule back, whatever its size', () => {
  const held = []
  let collect
  class Registry {
    constructor(callback) { collect = callback }
    register(_key, value) { held.push(value) }
    unregister() {}
  }
  const context = vm.createContext({ FinalizationRegistry: Registry })
  vm.runInContext(source, context)
  const rt = context.__rt
  // A stack array of 100,000 eight-byte elements is 800,000 bytes: a 2^20 granule.
  const stack = new Array(100000).fill(0)
  const at = rt.addr({ buf: stack, off: 0 }, 8)
  assert.equal(at % MB, 0)
  assert.equal(rt.addr({ buf: stack, off: 99999 }, 8), at + 799992)
  assert.equal(held.length, 1)
  const after = nonNull(rt, rt.alloc(16, 1))
  assert.ok(after >= at + MB || after < at, 'a later block stays out of the array\'s range')
  collect(held[0])
  assert.equal(nonNull(rt, block(rt, MB)), at, 'the collected granule is reused')
})

test('no granule, however large, covers the zero-sized address', () => {
  const rt = runtime()
  rt._addrNext = rt._zst_addr - 16
  const live = []
  for (const bytes of [MB, 4096, 16, 1 << 16, 1 << 28, 100, 3 * MB]) {
    const base = nonNull(rt, block(rt, bytes))
    const granule = granuleOf(bytes)
    assert.ok(!(base <= rt._zst_addr && rt._zst_addr < base + granule), `${bytes} bytes at ${base}`)
    live.push([base, granule])
  }
  assertDisjoint(live, 'around the zero-sized address')
  // The megabyte that stepped over it left the rest of its pages free, and they serve.
  assert.equal(live[0][0], rt._zst_addr + MB)
  assert.equal(live[1][0], rt._zst_addr + 4096)
})

test('a large request at the top is refused without disturbing the space', () => {
  const rt = runtime()
  rt._addrNext = 2 ** 32 - 2 ** 21
  const a = block(rt, 2 ** 21)
  assert.equal(nonNull(rt, a), 2 ** 32 - 2 ** 21)
  assert.throws(() => nonNull(rt, block(rt, MB)), /address space is exhausted/)
  rt.dealloc(a, 2 ** 21, 1)
  assert.deepEqual([nonNull(rt, block(rt, MB)), nonNull(rt, block(rt, MB))], [2 ** 32 - 2 ** 21, 2 ** 32 - MB])
  // A block larger than the whole space has nowhere to go either.
  assert.throws(() => nonNull(rt, block(rt, 2 ** 32 + 1)), /address space is exhausted/)
})

test('a refused bump hands out no granule twice', () => {
  const rt = runtime()
  rt._addrNext = 2 ** 32 - 4080
  assert.throws(() => nonNull(rt, rt.alloc(4096, 1)), /address space is exhausted/)
  assert.equal(rt._addrNext, 2 ** 32 - 4080)
  // What is left is 4080 bytes: 255 sixteen-byte granules, each handed out once.
  const seen = new Set()
  for (let i = 0; i < 255; i++) {
    const at = nonNull(rt, rt.alloc(16, 1))
    assert.ok(!seen.has(at), `${at} handed out twice`)
    seen.add(at)
  }
  assert.throws(() => nonNull(rt, rt.alloc(16, 1)), /address space is exhausted/)
})

test('once the bump pointer is at the top, a small request cuts a page out of a free large granule', () => {
  const rt = runtime()
  // A 2 GiB block takes the upper half of the space; the gap below it is free large granules.
  const huge = block(rt, 2 ** 31)
  assert.equal(nonNull(rt, huge), 2 ** 31)
  assert.equal(rt._addrNext, 2 ** 32)
  const small = Array.from({ length: 300 }, (_, i) => nonNull(rt, rt.alloc(i % 2 === 0 ? 16 : 100, 1)))
  assert.equal(new Set(small).size, small.length)
  for (const at of small) assert.ok(at >= 4096 && at < 2 ** 31, `${at}`)
  assertDisjoint(small.map((at, i) => [at, i % 2 === 0 ? 16 : 128]), 'small blocks from cut pages')
  // A large request is still served from what is left.
  const page = nonNull(rt, block(rt, 1 << 20))
  assert.ok(page < 2 ** 31 && page % (1 << 20) === 0)
})

// ---------------------------------------------------------------------------------------------
// The size of a key is the caller's to say (`addr`), and only an array or a typed array is a
// buffer of `length` elements: a struct is a JavaScript object whose fields carry their Rust
// names, and one of them may well be called `length`.
// ---------------------------------------------------------------------------------------------

test('a struct with a field called `length` is one element, whatever the field holds', () => {
  for (const length of [100000, 4000000000, Infinity, -Infinity, NaN, -5, 2 ** 53, '1000000', 1e300]) {
    const rt = runtime()
    const s = { length, kind: 7 }
    // `&s.kind as *const u8 as usize`: a field slot names the struct object as its buffer.
    const field = rt.addr({ buf: s, off: 'kind' }, 1)
    // `&s as *const S as usize`, the object form.
    const whole = rt.addr(s, 8)
    assert.equal(whole, field, `length ${String(length)}: one object, one base`)
    assert.ok(Number.isInteger(field) && field >= 4096 && field % 16 === 0, `length ${String(length)}: base ${field}`)
    assert.equal(rt._addrNext, 4096 + 16, `length ${String(length)}: one 16-byte granule`)
    const after = nonNull(rt, rt.alloc(16, 1))
    assert.equal(after, 4096 + 16, `length ${String(length)}: the space is undisturbed`)
  }
  // Many of them live at once take a granule each, not a megabyte each.
  const rt = runtime()
  const keep = []
  for (let i = 0; i < 5000; i++) {
    const s = { length: 1_000_000 + i, start: i }
    keep.push(s)
    rt.addr({ buf: s, off: 'start' }, 8)
  }
  assert.ok(rt._addrNext <= 4096 + 5000 * 16, `top ${rt._addrNext}`)
})

test('a buffer is sized by its length: arrays and typed arrays', () => {
  const rt = runtime()
  const words = new Array(3000).fill(0)
  const at = rt.addr({ buf: words, off: 0 }, 4)
  assert.equal(at % 16384, 0, '12,000 bytes: a 16 KiB granule')
  const bytes = new Uint8Array(70000)
  const b = rt.addr({ buf: bytes, off: 0 }, 1)
  assert.equal(b % 131072, 0, '70,000 bytes: a 128 KiB granule')
  assertDisjoint([[at, 16384], [b, 131072], [nonNull(rt, rt.alloc(16, 1)), 16]], 'array, typed array, block')
})

test('a heap block is sized by its Layout, whatever the caller says', () => {
  const rt = runtime()
  const p = rt.alloc(70000, 1)
  // An erased first ask (pointee size 0) still gives the block its whole 128 KiB.
  assert.equal(rt.addr(p, 0) % 131072, 0)
  assert.equal(rt._addrNext, 131072 * 2)
})

// ---------------------------------------------------------------------------------------------
// Blocks that are never `dealloc`ed. A baseline `Vec` converted into the native carrier is
// consumed by the conversion, and the native carrier's drop is the engine's.
// ---------------------------------------------------------------------------------------------

test('a Vec converted into the native carrier gives its block\'s address back', () => {
  const rt = runtime()
  const p = rt.alloc(100000, 1)
  p.buf[0] = 42
  const at = nonNull(rt, p)
  const native = rt.convert_block_nv(p.buf, p.off, 1, 100000)
  assert.equal(native.cap, 100000)
  assert.equal(native.buf[0], 42)
  // The source is not poisoned: a raw pointer into it still reads what it held.
  assert.equal(p.buf[0], 42)
  // The granule is free again: the next block of that size takes it.
  const q = rt.alloc(100000, 1)
  assert.equal(nonNull(rt, q), at)
  // The source block is still registered, unfreed: asked again it gets a base of its own...
  const again = rt.addr(p, 1)
  assert.notEqual(again, at)
  assertDisjoint([[at, 131072], [again, 131072]], 'converted source asked again')
  // ...and a later `dealloc` of it releases that one granule, once.
  rt.dealloc(p, 100000, 1)
  const r = nonNull(rt, rt.alloc(100000, 1))
  const s = nonNull(rt, rt.alloc(100000, 1))
  assert.equal(r, again)
  assert.notEqual(s, again)
  assert.notEqual(s, at)
})

test('a converted block that is then deallocated is released once', () => {
  const rt = runtime()
  const p = rt.alloc(5000, 1)
  const at = nonNull(rt, p)
  rt.convert_block_nv(p.buf, p.off, 0, 5000)
  rt.dealloc(p, 5000, 1)
  const seen = [nonNull(rt, rt.alloc(5000, 1)), nonNull(rt, rt.alloc(5000, 1)), nonNull(rt, rt.alloc(5000, 1))]
  assert.equal(seen[0], at)
  assert.equal(new Set(seen).size, 3, `${seen}`)
})

test('forty thousand 100 KB Vecs, each addressed and dropped through the native carrier, fit', () => {
  const rt = runtime()
  let top = 0
  for (let i = 0; i < 40000; i++) {
    const p = block(rt, 100000)
    if (nonNull(rt, p) === 0) throw new Error(`Vec ${i} got the null address`)
    rt.convert_block_nv(p.buf, p.off, 0, 0)
    top = Math.max(top, rt._addrNext)
  }
  assert.ok(top <= 2 * 131072 + 4096, `top ${top}`)
})

test('a heap block of a page or more is registered with the registry, and unregistered when it is given back', () => {
  const held = []
  const gone = []
  let collect
  class Registry {
    constructor(callback) { collect = callback }
    register(key, value, token) { held.push([key, value, token]) }
    unregister(token) { gone.push(token) }
  }
  const context = vm.createContext({ FinalizationRegistry: Registry })
  vm.runInContext(source, context)
  const rt = context.__rt
  const a = rt.alloc(100000, 1)
  const at = nonNull(rt, a)
  const b = rt.alloc(100000, 1)
  nonNull(rt, b)
  assert.equal(held.length, 2)
  // A heap block below a page is not registered: that would cost more than its mint and release.
  nonNull(rt, rt.alloc(16, 1))
  assert.equal(held.length, 2)
  assert.equal(held[0][0], a.buf)
  assert.equal(held[0][2], a.buf, 'the block is its own unregister token')
  // `dealloc` gives the granule back itself, so the registry must not.
  rt.dealloc(b, 100000, 1)
  assert.deepEqual(gone, [b.buf])
  // `a` is abandoned without `dealloc`: the engine collects it, and its granule serves again.
  collect(held[0][1])
  assert.equal(nonNull(rt, rt.alloc(100000, 1)), at)
})
