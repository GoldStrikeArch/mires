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

