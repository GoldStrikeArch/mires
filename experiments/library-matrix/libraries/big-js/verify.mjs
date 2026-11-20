import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { variant } from './bench-variant.mjs'
import { run as reference } from './reference.mjs'

const language = process.argv[2]
const { run } = language ? await import(`../../ports/big-js/${language}/dist/index.min.js`) : { run:reference }
const vectors = JSON.parse(readFileSync(new URL('upstream-vectors.json',import.meta.url)))
for (const v of vectors) assert.equal(run({cases:[v.case]}),v.expected,`${v.case.op}.js:${v.line}`)
console.log(`${vectors.length} upstream vectors passed`)

const wide = JSON.parse(readFileSync(new URL('exponent-vectors.json',import.meta.url)))
for (const {case:command,expected} of wide) {
  assert.equal(reference({cases:[command]}),expected,JSON.stringify(command))
  assert.equal(run({cases:[command]}),expected,JSON.stringify(command))
}
console.log(`${wide.length} wide exponent cases passed`)

const {input} = JSON.parse(readFileSync(new URL('corpus.json',import.meta.url))).cases.find(c=>c.id==='bench-mixed')
const nonces = [0,1,414657,Number.MAX_SAFE_INTEGER] // 1 and 414657 previously collided.
const variants = nonces.map(k=>variant(input,k))
for (const [i,command] of input.cases.entries()) {
  for (const key of ['x','y']) {
    assert.match(command[key],/^[1-9][0-9]{48}[1-9]$/)
    const operands = variants.map(v=>v.cases[i][key])
    assert.equal(new Set(operands).size,nonces.length)
    for (const operand of operands) {
      assert.equal(operand.length,command[key].length)
      assert.equal(operand[0],command[key][0])
      assert.equal(operand.at(-1),command[key].at(-1))
    }
  }
}

const edges = [2**31, 2**32, 2**48]
const commands = []
for (const edge of edges) for (const sign of [-1,1]) for (const delta of [-1,0,1]) {
  const exponent = sign * edge + delta
  for (const coefficient of ['0.99','-1.01','7.5','99.95']) {
    const x = coefficient+'e'+exponent, y = '1.05e'+(exponent+1)
    for (const op of ['plus','minus','times','div','cmp']) commands.push({op,x,y,dp:7})
    for (const op of ['toPrecision','toExponential','round']) commands.push({op,x,n:2,mode:2})
  }
}
assert.equal(run({cases:commands}),reference({cases:commands}))
console.log(`${commands.length} wide exponent differential cases passed`)
