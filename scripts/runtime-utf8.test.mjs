import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'

const texts = ['', '\ufeff', '\ufeff[]', '\ufeff\ufeffx', 'a\ufeffb', 'α😀\0', 'a'.repeat(4097), '\ufeff' + 'a'.repeat(4097)]
function runtime(source) {
  const context = vm.createContext({ TextEncoder, TextDecoder, Uint8Array })
  vm.runInContext(source, context)
  return context.__rt
}
function verify(rt) {
  // Rust UTF-8 decoding preserves every valid scalar, including a leading U+FEFF.
  for (const text of texts) {
    const bytes = new TextEncoder().encode(text), padded = new Uint8Array(bytes.length + 4)
    padded.set([0xff, 0xff]); padded.set(bytes, 2); padded.set([0xff, 0xff], bytes.length + 2)
    for (const input of [padded, Array.from(padded)]) {
      assert.equal(rt.bytes_str(input, 2, bytes.length), text)
      assert.equal(rt.bytes_str(input, 2, bytes.length), text, 'warm decoder must preserve the same text')
    }
    const encoded = rt.str_bytes(text)
    assert.equal(rt.bytes_str(encoded.buf, encoded.off, encoded.len), text, 'Rust string byte-view round trip')
  }
  assert.equal(rt.bytes_str(undefined, 0, 0), '')
}
// The historical harness compared two sibling repositories. Exercise this standalone runtime.
for (const compiler of ['mires-v2']) {
  const source = await readFile(new URL('../runtime/shim.js', import.meta.url), 'utf8')
  test(`${compiler} preserves UTF-8 scalars, BOMs and subrange boundaries`, () => verify(runtime(source)))
  test(`${compiler} detects the original BOM-stripping decoder`, () => {
    const faulty = source.replace('new TextDecoder("utf-8", { ignoreBOM: true })', 'new TextDecoder()')
    assert.notEqual(faulty, source)
    assert.throws(() => verify(runtime(faulty)), assert.AssertionError)
  })
}

// ---------------------------------------------------------------------------------------------
// The byte view, its length, and the memo that bounds re-encoding (CONTRACT.md, "`str` is a
// hybrid"). Every expected byte sequence comes from the host's own TextEncoder.

const source = await readFile(new URL('../runtime/shim.js', import.meta.url), 'utf8')
const encoder = new TextEncoder()
const decoder = new TextDecoder('utf-8', { ignoreBOM: true })
const utf8 = (text) => Array.from(encoder.encode(text))
const view = (record) => Array.from(record.buf).slice(record.off, record.off + record.len)
const label = (text) => JSON.stringify(text.slice(0, 12))
// A runtime whose microtasks are collected rather than run, so a test decides when the stack
// "unwinds".
function scheduled() {
  const jobs = []
  const context = vm.createContext({ TextEncoder, TextDecoder, Uint8Array, queueMicrotask: (job) => jobs.push(job) })
  vm.runInContext(source, context)
  return { rt: context.__rt, jobs, unwind: () => { while (jobs.length) jobs.shift()() } }
}

const astral = '😀'
const encodings = [
  '', 'a', 'ascii only, sixteen', 'exactly 16 chars', 'seventeen chars!!', '\0\x7f',
  '\x80', '߿', 'é', 'Grüße', 'ࠀ', '￿', '世界', astral, 'a' + astral + 'b',
  '﻿', '﻿BOM', 'x﻿',
  // Lone and misordered surrogates: the encoder writes U+FFFD (three bytes) for each.
  '\ud800', '\udc00', 'a\ud800', '\ud800b', '\udc00\ud800', '\ud800𐀀', astral + '\ud83d',
  'é'.repeat(4096), '世'.repeat(5000), astral.repeat(3000) + '\ud800',
  'a'.repeat(4096), 'a'.repeat(4097), 'a'.repeat(1 << 20), ('Grüße 世界 ' + astral).repeat(80000),
]

test('str_bytes and str_len agree with TextEncoder on every width, BOMs, lone surrogates and 1 MB', () => {
  const { rt } = scheduled()
  for (const text of encodings) {
    const want = utf8(text)
    const record = rt.str_bytes(text)
    assert.equal(record.off, 0)
    assert.equal(record.len, want.length, `len of ${label(text)}`)
    assert.ok(Array.isArray(record.buf), 'the byte view is a plain array, the kind every [u8] is')
    assert.deepEqual(view(record), want, `bytes of ${label(text)}`)
    assert.equal(rt.str_len(text), want.length, `str_len of ${label(text)}`)
    assert.equal(rt.bytes_str(record.buf, record.off, record.len), decoder.decode(encoder.encode(text)))
  }
  // str_len never needs the memo: a fresh runtime answers before any str_bytes.
  const fresh = scheduled().rt
  for (const text of encodings) assert.equal(fresh.str_len(text), utf8(text).length, `fresh str_len of ${label(text)}`)
  assert.equal(fresh._str_memo.map.size, 0, 'str_len does not populate the memo')
})

test('str_len and str_bytes match TextEncoder on random UTF-16, including unpaired surrogates', () => {
  const { rt } = scheduled()
  let seed = 12345
  const next = () => (seed = (Math.imul(seed, 1103515245) + 12345) >>> 0)
  const units = [0x41, 0x7f, 0x80, 0x7ff, 0x800, 0xd7ff, 0xd800, 0xdbff, 0xdc00, 0xdfff, 0xe000, 0xfeff, 0xffff]
  for (let i = 0; i < 4000; i++) {
    const n = next() % 90
    let text = ''
    for (let j = 0; j < n; j++) text += String.fromCharCode(next() % 3 ? units[next() % units.length] : next() & 0xffff)
    assert.equal(rt.str_len(text), encoder.encode(text).length, JSON.stringify(text))
    assert.deepEqual(view(rt.str_bytes(text)), utf8(text), JSON.stringify(text))
  }
})

test('an encoder without encodeInto still produces the same bytes', () => {
  class OldEncoder { encode(text) { return encoder.encode(text) } }
  const context = vm.createContext({ TextEncoder: OldEncoder, TextDecoder, Uint8Array })
  vm.runInContext(source, context)
  for (const text of ['Grüße', '世界' + astral, '\ud800x', 'é'.repeat(5000), 'ascii']) {
    assert.deepEqual(view(context.__rt.str_bytes(text)), utf8(text), label(text))
  }
})

test('bytes_str reads i8 elements and typed arrays as bytes', () => {
  const { rt } = scheduled()
  // `é` is C3 A9; as i8 elements those are -61 and -87. A negative byte is not ASCII.
  assert.equal(rt.bytes_str([0x61, -61, -87, 0x62], 0, 4), 'aéb')
  assert.equal(rt.bytes_str([-16, -97, -104, -128], 0, 4), astral)
  const typed = encoder.encode('xx' + 'Grüße' + 'yy')
  assert.equal(rt.bytes_str(typed, 2, typed.length - 4), 'Grüße')
  assert.equal(rt.bytes_str(encoder.encode('plain ascii'), 0, 11), 'plain ascii')
  const long = 'é'.repeat(3000)
  assert.equal(rt.bytes_str(utf8(long), 0, utf8(long).length), long)
})

test('within one operation a string has one byte view, and the memo empties when the stack unwinds', () => {
  const { rt, jobs, unwind } = scheduled()
  const doc = 'line one\nline two\n'.repeat(500)
  const first = rt.str_bytes(doc)
  assert.equal(jobs.length, 1, 'the first insertion queues exactly one clearing microtask')
  // Many other strings pass through between two reads of the document -- the shape that evicted
  // the old four-entry ring on every source line.
  for (let i = 0; i < 1000; i++) rt.str_bytes(`token ${i}`)
  assert.equal(rt.str_bytes(doc), first, 'same record within the operation')
  assert.equal(rt.str_bytes(doc.split('').join('')), first, 'equal content is the same key')
  assert.equal(jobs.length, 1, 'no further microtasks while armed')
  assert.equal(rt.str_len(doc), utf8(doc).length)
  unwind()
  assert.equal(rt._str_memo.map.size, 0)
  assert.equal(rt._str_memo.bytes, 0)
  const second = rt.str_bytes(doc)
  assert.notEqual(second, first, 'a new operation encodes again')
  assert.deepEqual(view(second), view(first))
  assert.equal(jobs.length, 1, 'the next operation re-arms the clear')
})

test('the memo is bounded by bytes beyond its capacity, and a huge string survives its slices', () => {
  const { rt } = scheduled()
  const memo = rt._str_memo
  const budget = memo.budget
  assert.equal(budget, 4 * 1024 * 1024)
  // Well past capacity with distinct medium strings: always correct, never above the budget.
  const chunk = 'm'.repeat(64 * 1024 - 8)
  for (let i = 0; i < 200; i++) {
    const text = chunk + String(i).padStart(8, '0')
    const record = rt.str_bytes(text)
    assert.equal(record.len, text.length)
    assert.equal(record.buf[record.len - 1], text.charCodeAt(text.length - 1))
    assert.ok(memo.bytes <= budget, `held ${memo.bytes} bytes`)
    let total = 0
    for (const [key, value] of memo.map) if (!memo.huge.includes(key)) total += value.len
    assert.equal(total, memo.bytes, 'the byte count is exact')
  }
  // A document larger than budget / 4 is held apart: churning more than the whole budget through
  // the memo does not evict it, so slicing it stays linear.
  const doc = ('# heading\n\n' + 'text '.repeat(40) + '\n').repeat(6000)
  assert.ok(doc.length > budget / 4)
  const record = rt.str_bytes(doc)
  for (let i = 0; i < 400; i++) {
    rt.str_bytes(chunk + 'x' + i)
    assert.equal(rt.str_bytes(doc), record, `document evicted after ${i} slices`)
  }
  // At most two huge strings are held; a third evicts the oldest.
  const second = doc + 'b', third = doc + 'c'
  const r2 = rt.str_bytes(second)
  rt.str_bytes(third)
  assert.equal(memo.huge.length, 2)
  assert.equal(rt.str_bytes(second), r2)
  assert.notEqual(rt.str_bytes(doc), record, 'the oldest huge string was evicted')
})

test('without queueMicrotask the byte bound alone applies', () => {
  const rt = runtime(source)
  for (let i = 0; i < 100; i++) rt.str_bytes('n'.repeat(100 * 1024) + i)
  assert.ok(rt._str_memo.bytes <= rt._str_memo.budget)
  assert.equal(rt._str_memo.armed, true)
})

test('an owned String converted from a host string never aliases the memoized view', () => {
  const { rt } = scheduled()
  for (const text of ['owned', 'Grüße ' + astral, 'a'.repeat(5000)]) {
    const shared = rt.str_bytes(text)
    const before = view(shared)
    const owned = rt.convert_str_block(text)
    assert.notEqual(owned.buf, shared.buf)
    owned.buf[0] = 0x21
    owned.buf.push(0x21)
    assert.deepEqual(view(rt.str_bytes(text)), before)
    const builder = rt.convert_sb_block({ text, cap: before.length + 8, len: before.length })
    assert.notEqual(builder.buf, shared.buf)
    builder.buf[1] = 0x21
    assert.deepEqual(view(rt.str_bytes(text)), before)
    // A mutated String derefs to a different JavaScript string: never a stale view.
    const mutated = rt.bytes_str(owned.buf, 0, before.length)
    assert.notEqual(mutated, text)
    assert.deepEqual(view(rt.str_bytes(mutated)), utf8(mutated))
  }
})

test('a mutable byte view (as_bytes_mut) is fresh, so a write through it never reaches the memo', () => {
  const { rt } = scheduled()
  const text = 'Hello World'
  const shared = rt.str_bytes(text)
  const mutable = rt.str_bytes_mut(text)
  assert.notEqual(mutable, shared)
  assert.notEqual(mutable.buf, shared.buf)
  assert.deepEqual(view(mutable), utf8(text))
  for (let i = 0; i < mutable.len; i++) mutable.buf[i] |= 0x20
  assert.deepEqual(view(rt.str_bytes(text)), utf8(text), 'the shared view is untouched')
  assert.equal(rt.str_len(text), 11)
  assert.notEqual(rt.str_bytes_mut(text), mutable, 'never memoized')
})

// Timing, printed and never asserted: the old conversion (`Array.prototype.slice` over the
// encoder's typed array) against `_utf8`, on fresh strings (a miss every time), and a memo hit.
test('str_bytes timing (printed, not asserted)', () => {
  const { rt, unwind } = scheduled()
  const ascii = ('## Section 12\n\n- **browser** compiler\n- [report](https://example.test/12)\n\n').repeat(120).slice(0, 8192)
  const mixed = ('Grüße, 世界! emoji ' + astral + ' and ascii, ').repeat(300).slice(0, 8192)
  const old = (s) => Array.prototype.slice.call(encoder.encode(s))
  const time = (f, iterations) => {
    for (let i = 0; i < 20; i++) f(i)
    const samples = []
    for (let r = 0; r < 7; r++) {
      const start = process.hrtime.bigint()
      for (let i = 0; i < iterations; i++) f(i)
      samples.push(Number(process.hrtime.bigint() - start) / iterations / 1000)
    }
    return samples.sort((a, b) => a - b)[3].toFixed(2)
  }
  const rows = []
  for (const [name, text] of [['8 KB ASCII', ascii], ['8 KB mixed', mixed]]) {
    // A distinct string object per call, so no memo applies to the miss columns.
    const fresh = Array.from({ length: 64 }, () => JSON.parse(JSON.stringify(text)))
    rows.push({
      input: name,
      'before: slice.call(encode) µs': time((i) => old(fresh[i & 63]), 200),
      'after: miss µs': time((i) => { unwind(); return rt.str_bytes(fresh[i & 63]) }, 200),
      'after: memo hit µs': time(() => rt.str_bytes(text), 2000),
      'str_len, no memo µs': time((i) => { unwind(); return rt.str_len(fresh[i & 63]) }, 200),
    })
  }
  console.log('# str_bytes timing (median of 7)')
  console.table(rows)
})

function runtimeNoCodec() {
  const jobs = []
  const context = vm.createContext({ Uint8Array, queueMicrotask: (job) => jobs.push(job) })
  vm.runInContext(source, context)
  return { rt: context.__rt, jobs, unwind: () => { while (jobs.length) jobs.shift()() } }
}

test('pure-JS UTF-8 fallback without TextEncoder/TextDecoder matches native codecs on all inputs', () => {
  const { rt } = runtimeNoCodec()
  assert.equal(rt._encoder.value, null)
  assert.equal(rt._decoder.value, null)
  for (const text of [...texts, ...encodings]) {
    const want = utf8(text)
    const rec = rt.str_bytes(text)
    assert.equal(rec.off, 0)
    assert.equal(rec.len, want.length, `no-codec len of ${label(text)}`)
    assert.deepEqual(view(rec), want, `no-codec bytes of ${label(text)}`)
    assert.equal(rt.str_len(text), want.length)
    assert.equal(rt.bytes_str(rec.buf, rec.off, rec.len), decoder.decode(encoder.encode(text)))
    const encoded = rt._encoder.encode(text)
    assert.ok(encoded instanceof Uint8Array)
    assert.deepEqual(Array.from(encoded), want)
  }
  // Subrange boundaries & i8 negative bytes without TextDecoder.
  for (const text of texts) {
    const bytes = encoder.encode(text), padded = new Uint8Array(bytes.length + 4)
    padded.set([0xff, 0xff]); padded.set(bytes, 2); padded.set([0xff, 0xff], bytes.length + 2)
    for (const input of [padded, Array.from(padded)]) {
      assert.equal(rt.bytes_str(input, 2, bytes.length), text)
    }
  }
  assert.equal(rt.bytes_str([0x61, -61, -87, 0x62], 0, 4), 'aéb')
  assert.equal(rt.bytes_str([-16, -97, -104, -128], 0, 4), astral)
})

test('pure-JS UTF-8 decoder matches TextDecoder(ignoreBOM: true) on invalid and random byte sequences', () => {
  const { rt } = runtimeNoCodec()
  const edgeBytes = [
    [], [0xef, 0xbb, 0xbf], [0xef, 0xbb, 0xbf, 0x41],
    [0xc0, 0x80], [0xc1, 0xbf], [0xc2, 0x7f], [0xc2, 0x80], [0xdf, 0xbf],
    [0xe0, 0x80, 0x80], [0xe0, 0x9f, 0xbf], [0xe0, 0xa0, 0x80], [0xe0, 0xa0],
    [0xe1, 0x80, 0x41], [0xe1, 0x80], [0xed, 0x9f, 0xbf], [0xed, 0xa0, 0x80], [0xed, 0xbf, 0xbf],
    [0xee, 0x80, 0x80], [0xef, 0xbf, 0xbf],
    [0xf0, 0x80, 0x80, 0x80], [0xf0, 0x8f, 0xbf, 0xbf], [0xf0, 0x90, 0x80, 0x80],
    [0xf0, 0x90, 0x80, 0x41], [0xf0, 0x90, 0x80], [0xf0, 0x90], [0xf0],
    [0xf4, 0x8f, 0xbf, 0xbf], [0xf4, 0x90, 0x80, 0x80], [0xf5, 0x80, 0x80, 0x80],
    [0x80], [0xbf], [0xff], [0xfe],
  ]
  for (const b of edgeBytes) {
    const u8 = new Uint8Array(b)
    assert.equal(rt.bytes_str(u8, 0, u8.length), decoder.decode(u8), JSON.stringify(b))
  }
  let seed = 987654321
  const next = () => (seed = (Math.imul(seed, 1103515245) + 12345) >>> 0)
  const interesting = [0x00, 0x41, 0x7f, 0x80, 0x8f, 0x90, 0x9f, 0xa0, 0xbf, 0xc0, 0xc1, 0xc2, 0xdf, 0xe0, 0xed, 0xee, 0xef, 0xf0, 0xf4, 0xf5, 0xff]
  for (let i = 0; i < 4000; i++) {
    const len = next() % 32
    const u8 = new Uint8Array(len)
    for (let j = 0; j < len; j++) u8[j] = (next() & 1) ? interesting[next() % interesting.length] : next() & 0xff
    assert.equal(rt.bytes_str(u8, 0, u8.length), decoder.decode(u8))
  }
  const units = [0x41, 0x7f, 0x80, 0x7ff, 0x800, 0xd7ff, 0xd800, 0xdbff, 0xdc00, 0xdfff, 0xe000, 0xfeff, 0xffff]
  for (let i = 0; i < 2000; i++) {
    const n = next() % 90
    let text = ''
    for (let j = 0; j < n; j++) text += String.fromCharCode(next() % 3 ? units[next() % units.length] : next() & 0xffff)
    assert.deepEqual(view(rt.str_bytes(text)), utf8(text))
  }
})
