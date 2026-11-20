// Run after building the requested port. The fixed witnesses come from the
// upstream v5.3.2 unit suite. The second pass compares the published package
// with the production artifact on each bit position, shift width and radix.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { variant } from './bench-variant.mjs'
import { run as reference } from './reference.mjs'

const language = process.argv[2]
if (!/^[a-z]+$/.test(language ?? '')) throw new Error('expected a matrix language id')
const { run } = await import(new URL('../../ports/long/' + language + '/dist/index.min.js', import.meta.url))
const make = (op, overrides = {}) => ({
  op, a: [0, 0], b: [0, 0], u: false, v: false, s: '', n: 0, r: 10, ...overrides,
})
const words = bits => [Number(BigInt.asIntN(32, bits)), Number(BigInt.asIntN(32, bits >> 32n))]
const min = [0, -2147483648], max = [-1, 2147483647], all = [-1, -1]
const rotation = words(0x89abcdef01234567n)
const witnesses = [
  [make('shiftLeft', { a: [1234,14], n: 2147483648, r: 16 }), 'e000004d2'],
  [make('rotl', { a: rotation, n: -4294967292, u: true, r: 16 }), '9abcdef012345678'],
  [make('toString', { a: min }), '-9223372036854775808'],
  [make('toString', { a: max }), '9223372036854775807'],
  [make('toString', { a: all, u: true }), '18446744073709551615'],
  [make('toString', { a: all, u: true, r: 8 }), '1777777777777777777777'],
  [make('fromString', { s: 'zzzzzz', n: 36, r: 36 }), 'zzzzzz'],
  [make('fromString', { s: '-zzzzzz', n: 36, r: 36 }), '-zzzzzz'],
  [make('divide', { a: all, b: max, u: true }), '2'],
  [make('divide', { a: all, b: [-2, -1], u: true }), '1'],
  [make('divide', { a: [0, 8], b: [2656901066 | 0, 0], u: true, v: true }), '12'],
  [make('rotl', { a: rotation, n: 4, u: true, r: 16 }), '9abcdef012345678'],
  [make('rotl', { a: rotation, n: 60, u: true, r: 16 }), '789abcdef0123456'],
  [make('rotl', { a: rotation, n: 32, u: true, r: 16 }), '123456789abcdef'],
  [make('ctz', { a: [0, 1] }), '32'],
  [make('clz', { a: [0, 1] }), '31'],
]
for (const [command, answer] of witnesses) {
  assert.equal(reference({ cases: [command] }), answer)
  assert.equal(run({ cases: [command] }), answer, JSON.stringify(command))
}
let checked = 0
let batch = []
function flush() {
  if (!batch.length) return
  const input = { cases: batch }
  const actual = run(input).split('\n'), expected = reference(input).split('\n')
  assert.equal(actual.length, expected.length)
  for (let i = 0; i < actual.length; i++) assert.equal(actual[i], expected[i], JSON.stringify(batch[i]))
  checked += batch.length
  batch = []
}
function add(op, params) {
  batch.push(make(op, params))
  if (batch.length === 256) flush()
}
for (let bit = 0; bit < 64; bit++) {
  const basis = 1n << BigInt(bit), a = words(basis)
  for (const u of [true, false]) {
    for (let n = 0; n < 64; n++) {
      for (const op of ['rotl', 'shiftLeft', 'shiftRight', 'shiftRightUnsigned']) add(op, { a, u, n, r: 16 })
    }
    for (let radix = 2; radix <= 36; radix++) {
      for (const delta of [-1n, 0n, 1n]) {
        const s = (basis + delta).toString(radix)
        add('fromString', { s, u, n: radix, r: radix })
        add('toString', { a: words(basis + delta), u, r: radix })
      }
    }
    for (let j = 0; j < 64; j++) {
      const b = words((1n << BigInt(j)) + [-1n, 0n, 1n][(bit + j) % 3])
      for (const v of [true, false]) {
        for (const op of ['divide', 'modulo', 'multiply', 'compare']) add(op, { a, b, u, v, r: 16 })
      }
    }
  }
}
flush()
console.log(language + ' passed ' + witnesses.length + ' upstream known answers and ' + checked + ' differential commands')

const corpus = JSON.parse(readFileSync(new URL('corpus.json',import.meta.url)))
for (const {input} of corpus.cases.filter(c=>c.id.startsWith('bench-'))) {
  const nonces = [0,1,2**32-1,2**32,2**32+1,2**33+1,2**52,Number.MAX_SAFE_INTEGER-1,Number.MAX_SAFE_INTEGER]
  const commands = nonces.map(k=>variant(input,k).cases[0])
  assert.equal(new Set(commands.map(c=>JSON.stringify([c.a,c.b]))).size,nonces.length)
  assert.deepEqual(commands[1].a,[777692442,890447437])
  assert.deepEqual(commands[1].b,[1472306978,1234567])
  const {a,b,...settings} = input.cases[0]
  for (const {a:lhs,b:rhs,...rest} of commands) {
    assert.deepEqual(rest,settings)
    for (const operand of [lhs,rhs]) {
      assert.equal(operand.length,2)
      for (const word of operand) assert.equal(word,word|0)
    }
  }
  const high = variant(input,2**32+1).cases[0]
  assert.notDeepEqual(high.a,commands[1].a)
  assert.notDeepEqual(high.b,commands[1].b)
}
