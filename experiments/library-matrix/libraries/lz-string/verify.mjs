// Compare the production port with npm's original CommonJS entry as well as
// the browser-loadable reference. Include independently generated damaged data.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { variant } from './bench-variant.mjs'
import { createRequire } from 'node:module'
import { run as reference, units } from './reference.mjs'

const language = process.argv[2]
if (!/^[a-z]+$/.test(language ?? '')) throw new Error('expected a matrix language id')
const { run } = await import(new URL('../../ports/lz-string/' + language + '/dist/index.min.js', import.meta.url))
const upstream = createRequire(import.meta.url)('lz-string')
const fixture=JSON.parse(readFileSync(new URL('corpus.json',import.meta.url))).cases.find(c=>c.id==='bench-base64').input
const count=new Set()
for(let k=1;k<=9003;k++) {
  const record=String.fromCharCode(...variant(fixture,k).data)
  assert.equal(record.length,fixture.data.length)
  JSON.parse(record)
  count.add(record)
}
assert.equal(count.size,9003)
const distant=[1,9001,2**32+1,Number.MAX_SAFE_INTEGER].map(k=>String.fromCharCode(...variant(fixture,k).data))
assert.equal(new Set(distant).size,distant.length)
for(const record of distant) assert.equal(upstream.decompressFromBase64(upstream.compressToBase64(record)),record)
const formats = [['raw','compress'],['base64','compressToBase64'],['utf16','compressToUTF16'],['uri','compressToEncodedURIComponent'],['bytes','compressToUint8Array']]
let seed = 0x5810f7
function random() { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed }
function capture(fn, input) { try { return fn(input) } catch { return 'throws' } }
let checked = 0
for (let n = 0; n < 256; n++) {
  const size = n === 255 ? 20000 : n
  const chars = Array.from({length:size}, (_,i) => n%3===0 ? random()>>>16 : n%3===1 ? random()%11 : i%47)
  for (const [format, method] of formats) {
    const encoded = upstream[method](String.fromCharCode(...chars))
    const packed = format === 'bytes' ? Array.from(encoded) : units(encoded)
    assert.deepEqual(run({op:'compress',format,data:chars}), packed, 'encode '+format+' '+n)
    assert.deepEqual(run({op:'decompress',format,data:packed}), chars, 'decode '+format+' '+n)
    const damaged = packed.slice()
    if (damaged.length) damaged[random()%damaged.length] ^= 1 << (random() % (format==='bytes'?8:16))
    const input = {op:'decompress',format,data:damaged}
    assert.deepEqual(capture(run,input),capture(reference,input), 'damaged '+format+' '+n)
    checked += 3
  }
}
console.log(language + ': ' + checked + ' wire-format comparisons passed')
