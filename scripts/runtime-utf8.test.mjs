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
