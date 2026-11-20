// Run with a language id after building that port. Fixed expected answers are
// independent of npm. The second pass exercises a different chunk split per length.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { run as reference } from './reference.mjs'

const language = process.argv[2]
if (!/^[a-z]+$/.test(language ?? '')) throw new Error('expected a matrix language id')
const { run: port } = await import(new URL('../../ports/noble-hashes/'+language+'/dist/index.min.js',import.meta.url))
const vectors = JSON.parse(readFileSync(new URL('known-answers.json',import.meta.url),'utf8'))
for (const { algorithm, message, key, digest } of vectors) {
  const input = { algorithm, bytes: [...Buffer.from(message,'hex')] }
  if (key !== undefined) input.key = [...Buffer.from(key,'hex')]
  assert.equal(reference(input),digest,algorithm+' reference vector')
  assert.equal(port(input),digest,algorithm+' port vector')
}
const algorithms = ['sha256','sha512','sha3_256','keccak_256','blake2b','blake2s','hmac_sha256']
for (let n = 0; n <= 1024; n++) {
  const bytes = Array.from({length:n}, (_,i) => (n*13 + i*29 + (i>>>2)) & 255)
  const cut = (n * 37) % (n + 1)
  for (const algorithm of algorithms) {
    const params = { algorithm }
    if (algorithm === 'hmac_sha256') params.key = bytes.slice(0,n%139)
    if ((algorithm==='blake2b' || algorithm==='blake2s') && n%2) {
      params.dkLen = 1+n%(algorithm==='blake2b'?64:32)
      params.key = [n&255,255]
    }
    const expected = reference({...params,bytes})
    assert.equal(port({...params,chunks:[bytes.slice(0,cut),[],bytes.slice(cut)]}),expected,algorithm+' length '+n)
  }
}
console.log(language + ': ' + vectors.length + ' known answers and ' + (1025*algorithms.length) + ' incremental comparisons passed')
