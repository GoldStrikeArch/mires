import assert from 'node:assert/strict'
import {variant} from './bench-variant.mjs'
import { readFileSync,writeFileSync } from 'node:fs'
import { run as original } from './reference.mjs'

const [language='reference',flag] = process.argv.slice(2)
if(flag==='--capture' && language!=='reference') throw Error('Only the pinned package can regenerate expected outputs')
const run=language==='reference'?original:(await import(`../../ports/ky/${language}/dist/index.min.js`)).run
const cases=JSON.parse(readFileSync(new URL('corpus.json',import.meta.url))).cases
const bench = cases.find(c=>c.id==='bench-requests').input
const seeds = [1, 2**32+1, 2**52+1, Number.MAX_SAFE_INTEGER]
const variants = seeds.map(k=>variant(bench,k))
for (const input of variants) assert.equal(input.cases.length,bench.cases.length)
assert.ok(bench.cases.length)
for (const i of bench.cases.keys()) {
  const targets = variants.map(input=>input.cases[i].url)
  assert.equal(new Set(targets).size,seeds.length)
  assert.equal(new Set(targets.map(target=>target.length)).size,1)
}
const expected=new URL('expected.json',import.meta.url)
const outputs={}
for(const c of cases) outputs[c.id]=await run(c.input)
const overlapIds = ['get-json', 'text']
const overlapCases = overlapIds.map(id => {
  const entry = cases.find(c => c.id === id)
  assert.ok(entry, `missing overlap corpus case ${id}`)
  return entry
})
const overlapTimeout = Symbol('overlap-timeout')
let overlapTimer
const overlapOutputs = await Promise.race([
  Promise.all(overlapCases.map(c => run(structuredClone(c.input)))),
  new Promise(resolve => { overlapTimer = setTimeout(() => resolve(overlapTimeout), 2000) }),
])
clearTimeout(overlapTimer)
assert.notEqual(overlapOutputs, overlapTimeout, `${language}: concurrent run(${overlapIds.join(', ')}) hung past timeout`)
for (const [i, c] of overlapCases.entries()) {
  assert.deepEqual(overlapOutputs[i], outputs[c.id], `${language}: concurrent overlap mismatch on ${c.id}`)
}
if(flag==='--capture') writeFileSync(expected,JSON.stringify(outputs,null,2)+'\n')
else assert.deepEqual(outputs,JSON.parse(readFileSync(expected)))
console.log(`${language}: ${cases.length} captured call groups matched (plus concurrent overlap)`)
