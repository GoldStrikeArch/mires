// The native mutable `String` carrier `{ text, cap, len }` (jsc-link `semantic_ops/string.rs`):
// `len` is the exact UTF-8 byte length of `text`, kept beside it so that no operation rescans the
// text, and the text is always well formed -- exactly the text the baseline `Vec<u8>` String
// decodes after the same appends. These tests pin the runtime half of that rule: every shim helper
// that builds or changes a builder leaves `len` exact and the text well formed, `native_str_wf`
// takes a host's lone surrogates as the baseline's bytes do, and `__rt._sb_check` (what
// `RCGJS_SB_VERIFY=1` emits) catches drift. The compiled half (the inline append, `clear`, drop,
// the constructors) is pinned byte for byte by `jsc-link`'s
// `every_lowering_matches_its_pinned_spelling`; `append` below is that pinned `push_str` spelling.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'

const source = await readFile(new URL('../runtime/shim.js', import.meta.url), 'utf8')
function runtime() {
  const context = vm.createContext({ TextEncoder, TextDecoder, Uint8Array })
  vm.runInContext(source, context)
  return context.__rt
}
const encoder = new TextEncoder()
const decoder = new TextDecoder('utf-8', { ignoreBOM: true })
const bytes = (text) => encoder.encode(text).length
// What the baseline `String` holds after appending `pieces`: their UTF-8 bytes (a lone surrogate
// encodes as U+FFFD), and the text those bytes decode back to.
function baseline(pieces) {
  const parts = pieces.map((piece) => encoder.encode(piece))
  const all = new Uint8Array(parts.reduce((total, part) => total + part.length, 0))
  let at = 0
  for (const part of parts) {
    all.set(part, at)
    at += part.length
  }
  return { text: decoder.decode(all), len: all.length }
}
// Well-formed scalars of every UTF-8 width, as code points.
const points = [0x61, 0x7f, 0xe9, 0x7ff, 0x800, 0x20ac, 0xfeff, 0xffff, 0x10000, 0x1f980, 0x10ffff]

// A deterministic generator, so a failure reproduces.
function generator(seed) {
  let state = seed >>> 0
  return (n) => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0
    return state % n
  }
}

function assertExact(rt, value, label) {
  assert.equal(value.len, bytes(value.text), `${label}: len is the text's UTF-8 length`)
  assert.equal(value.len, rt.str_len(value.text), `${label}: len agrees with str_len`)
  assert.equal(value.len, rt.str_bytes(value.text).len, `${label}: len agrees with the byte view`)
  assert.equal(value.text, value.text.toWellFormed(), `${label}: the text is well formed`)
  assert.ok(value.cap >= value.len, `${label}: cap ${value.cap} covers len ${value.len}`)
  assert.equal(rt._sb_check(value), value, `${label}: _sb_check accepts and returns the value`)
}

// The compiled `push_str`/`write_str` body (its capacity guard aside), as pinned in jsc-link.
function append(rt, value, piece) {
  const wellFormed = rt.native_str_wf(piece)
  const required = value.len + rt.str_len(wellFormed)
  if (value.cap < required) value.cap = required
  value.text += wellFormed
  value.len = required
}

test('native_str_wf takes a string exactly as UTF-8 bytes hold it', () => {
  const rt = runtime()
  const cases = ['', 'plain', '\u00e9\u20ac\u{1f980}', '\ud83d', '\ude00', '\ude00\ud83d', 'a\ud83db', 'a\ude00b',
    '\ud83d\ud83d\ude00', '\ud83d\ude00\ude00', '\ufffd', '\ufeff\ud800', '\udbff\udfff', 'x'.repeat(5000) + '\udc00']
  for (const text of cases) {
    const wellFormed = rt.native_str_wf(text)
    assert.equal(wellFormed, text.toWellFormed(), JSON.stringify(text))
    assert.equal(wellFormed, decoder.decode(encoder.encode(text)), `${JSON.stringify(text)} round trips through bytes`)
    assert.equal(rt.str_len(wellFormed), rt.str_len(text), 'sanitizing never changes the UTF-8 length')
  }
})

test('appending lone and paired surrogates at every junction matches the baseline bytes', () => {
  const rt = runtime()
  const units = ['a', '\u00e9', '\u20ac', '\ud83d', '\ude00', '\ud83d\ude00', '\ufffd', '\ufeff', '\udbff', '\udc00']
  // Every ordered pair of pieces, so each text-tail/piece-head kind meets every other once.
  for (const left of units) {
    for (const right of units) {
      for (const tail of ['', 'x', '\ud83d']) {
        const pieces = [left + tail, right, tail + left]
        const value = { text: '', cap: 0, len: 0 }
        for (const piece of pieces) append(rt, value, piece)
        const expected = baseline(pieces)
        assert.equal(value.text, expected.text, JSON.stringify(pieces))
        assert.equal(value.len, expected.len, JSON.stringify(pieces))
        assertExact(rt, value, JSON.stringify(pieces))
      }
    }
  }
  // Random pieces of up to five units drawn from the same alphabet, appended to random builders.
  const next = generator(11)
  for (let round = 0; round < 300; round++) {
    const value = { text: '', cap: next(8), len: 0 }
    const pieces = []
    for (let k = next(12); k > 0; k--) {
      let piece = ''
      for (let n = next(6); n > 0; n--) piece += units[next(units.length)]
      pieces.push(piece)
      append(rt, value, piece)
    }
    const expected = baseline(pieces)
    assert.equal(value.text, expected.text, JSON.stringify(pieces))
    assert.equal(value.len, expected.len, JSON.stringify(pieces))
    assertExact(rt, value, `round ${round}`)
  }
})

test('push, pop and truncate keep the cached length exact over mixed widths', () => {
  const rt = runtime()
  const next = generator(7)
  const value = { text: '', cap: 0, len: 0 }
  for (let step = 0; step < 4000; step++) {
    const choice = next(10)
    if (choice < 7) {
      const point = points[next(points.length)]
      // The compiled append reserves first (cap only grows); the helper appends.
      const required = value.len + rt._utf8_width(point)
      if (value.cap < required) value.cap = required
      rt.native_str_push(value, point)
    } else if (choice < 9) {
      const before = value.text
      const popped = rt.native_str_pop(value)
      if (before === '') assert.equal(popped, -1)
      else assert.equal(value.text + String.fromCodePoint(popped), before, 'pop removes the last scalar')
    } else {
      // A char boundary somewhere in [0, len], or one past it (a no-op).
      const boundaries = [0]
      for (const scalar of value.text) boundaries.push(boundaries[boundaries.length - 1] + bytes(scalar))
      const index = next(3) === 0 ? value.len + 1 + next(4) : boundaries[next(boundaries.length)]
      const cap = value.cap
      rt.native_str_truncate(value, index)
      assert.equal(value.cap, cap, 'truncate keeps the capacity')
    }
    assertExact(rt, value, `step ${step}`)
  }
})

test('truncate rejects a byte index inside a scalar and leaves the builder untouched', () => {
  const rt = runtime()
  const text = 'a\u00e9\u20ac\u{1f980}z'
  const value = { text, cap: 40, len: bytes(text) }
  for (const index of [2, 4, 5, 7, 8, 9, -1, 1.5, NaN]) {
    // The runtime lives in its own realm: its RangeError is not this module's.
    assert.throws(() => rt.native_str_truncate(value, index), (error) => error.name === 'RangeError' && rt.is_abort(error))
    assert.deepEqual({ ...value }, { text, cap: 40, len: 11 })
  }
  for (const index of [11, 12, 1000]) {
    rt.native_str_truncate(value, index)
    assert.deepEqual({ ...value }, { text, cap: 40, len: 11 }, `${index} is at or past the end: a no-op`)
  }
})

test('the runtime constructors take their text as Rust holds it, measured once, exact-fit', () => {
  const rt = runtime()
  const texts = ['', 'plain', 'Gr\u00fc\u00dfe', '\u20ac\u{1f980}\ufeff', 'a'.repeat(5000), 'a\u00e9'.repeat(3000), '\ud83d', 'x\udc00y', '\ude00\ud83d']
  for (const text of texts) {
    const value = rt.convert_host_sb(text)
    assert.deepEqual(Object.keys(value), ['text', 'cap', 'len'], 'one property order for every builder')
    // A lone surrogate is U+FFFD, three bytes, exactly as the baseline's bytes hold it.
    assert.deepEqual({ ...value }, { text: baseline([text]).text, cap: baseline([text]).len, len: baseline([text]).len })
    assertExact(rt, value, JSON.stringify(text))
  }
  for (const number of [0, -0, 1 / 3, 1e23, Number.MAX_VALUE, NaN, -Infinity]) {
    const value = rt.native_f64_to_string(number)
    assert.deepEqual(Object.keys(value), ['text', 'cap', 'len'])
    assert.equal(value.len, bytes(value.text))
    assert.equal(value.cap, value.len)
  }
})

test('a builder converted to a baseline block carries its length and capacity', () => {
  const rt = runtime()
  for (const text of ['', 'abc', '\u00e9\u{1f980}', 'z'.repeat(5000), '\ud83d']) {
    const value = rt.convert_host_sb(text)
    value.cap += 9
    const block = rt.convert_sb_block(value)
    assert.equal(block.len, value.len)
    assert.equal(block.cap, value.cap)
    assert.equal(rt.bytes_str(block.buf, 0, block.len), value.text)
  }
})

test('_sb_check reports drift as an abort, never as a wrong length', () => {
  const rt = runtime()
  const drifted = [
    { text: 'abc', cap: 8, len: 2 },
    { text: '\u00e9', cap: 8, len: 1 },
    { text: '\u{1f980}', cap: 8, len: 2 },
    { text: 'abc', cap: 2, len: 3 },
    { text: 'abc', cap: 8 },
    { text: 'abc', cap: undefined, len: 3 },
    // The right length for a lone surrogate, but text the baseline could never hold.
    { text: '\ud83d', cap: 3, len: 3 },
    { text: '\ud83d\ude00', cap: 6, len: 6 },
  ]
  for (const value of drifted) {
    assert.throws(
      () => rt._sb_check(value),
      (error) => error.name === 'RangeError' && rt.is_abort(error) && /native String length drift/.test(error.message),
      JSON.stringify(value),
    )
  }
})

test('appending never measures the accumulated text', () => {
  const rt = runtime()
  const measure = rt.str_len
  let measured = 0
  rt.str_len = (text) => {
    measured += text.length
    return measure(text)
  }
  const value = rt.convert_host_sb('')
  value.cap = 4 * 20000
  measured = 0
  for (let i = 0; i < 20000; i++) rt.native_str_push(value, points[i % points.length])
  assert.equal(measured, 0, 'native_str_push advances the length by the scalar width alone')
  let appended = 0
  for (let i = 0; i < 2000; i++) {
    const piece = i % 3 === 0 ? 'ab\u20ac' : i % 3 === 1 ? '\ud83d' : '\ude00x'
    appended += piece.length
    append(rt, value, piece)
  }
  assert.equal(measured, appended, 'push_str measures the piece alone')
  rt.str_len = measure
  assertExact(rt, value, 'after 20000 pushes and 2000 appends')
})
