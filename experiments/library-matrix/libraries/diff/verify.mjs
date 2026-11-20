// Differential checks exercise Myers ties, token boundaries, whitespace cleanup,
// independent patch parsing, mismatched sources and edits at both ends of a file.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { variant } from './bench-variant.mjs'
import { run as reference } from './reference.mjs'
import { canonicalText } from '../../tools/lib/canonical.mjs'
const language=process.argv[2]
if (!/^[a-z]+$/.test(language??'')) throw new Error('expected a matrix language id')
const {run}=await import(new URL('../../ports/diff/'+language+'/dist/index.min.js',import.meta.url))
let seed=0x9bb51
function next(limit) {
  seed=(Math.imul(seed,1664525)+1013904223)>>>0
  return seed%limit
}
function sequence(tokens,limit) {
  return Array.from({length:next(limit)},()=>tokens[next(tokens.length)])
}
let total=0
function check(input) {
  assert.equal(canonicalText(run(input),'json'),canonicalText(reference(input),'json'),JSON.stringify(input))
  total++
}
const suite = JSON.parse(readFileSync(new URL('corpus.json', import.meta.url)))
if (process.argv[3] === '--utf16') {
  for (const row of suite.cases.filter(c => c.id.startsWith('utf16-'))) check(row.input)
  console.log(language+' matched '+total+' UTF-16 boundary cases')
  process.exit(0)
}
// A broken string iterator can stall the engine before it returns a value.
const boundary = spawnSync(process.execPath, [fileURLToPath(import.meta.url), language, '--utf16'],
  { timeout: 15000, encoding: 'utf8' })
if (boundary.error) throw boundary.error
assert.equal(boundary.status, 0, boundary.stderr)
process.stdout.write(boundary.stdout)

for (const row of suite.cases.filter(c => c.id.startsWith('bench-'))) {
  const a = variant(row.input, 0), b = variant(row.input, 2**32)
  for (const field of ['before','after']) {
    assert.equal(a[field].length, b[field].length)
    const linesA = a[field].trimEnd().split('\n'), linesB = b[field].trimEnd().split('\n')
    assert.equal(linesA.length, linesB.length)
    assert.ok(linesA.every((line, i) => line !== linesB[i]))
  }
}
const words=['a','B','x','A','œ','🙂',' ','.','\t','  ','\n','ç','⁕','𝟚','　']
const scalars=[null,false,0,true,1,1.5,'','x','x','𝟚']
for(let i=0;i<400;i++){
  const before=sequence(words,45).join(''),after=sequence(words,45).join('')
  const options={ignoreCase:Boolean(i%2),oneChangePerToken:i%7===0}
  check({op:'diffChars',before,after,options})
  check({op:'diffWords',before,after,options})
  check({op:'diffArrays',before:sequence(scalars,30),after:sequence(scalars,30)})
  check({op:'diffJson',before:{a:sequence(scalars,6),w:before},after:{w:after,z:i,a:sequence(scalars,6)}})
}
for(let i=0;i<200;i++){
  const lines=sequence(['a','long row','B','', 'B','𝟚'],30)
  const changed=lines.slice()
  changed.splice(next(lines.length+1),next(5),...sequence(['A','c','B','', 'é'],6))
  const nl=i%3?'\n':'\r\n'
  const before=lines.join(nl)+(i%2?nl:''),after=changed.join(nl)+(i%3===2?'':nl)
  const args={before,after,oldFileName:i%2?'a/dir x':'a/qué',newFileName:i%2?'b/dir x':'b/qué',options:{context:next(6)}}
  check({op:'diffLines',before,after,options:{ignoreCase:i%3===1,ignoreWhitespace:i%3===2,newlineIsToken:!(i%7),ignoreNewlineAtEof:!(i%5),stripTrailingCr:!(i%4)}})
  check({op:'structuredPatch',...args})
  check({op:'createTwoFilesPatch',...args})
  const patch=reference({op:'createTwoFilesPatch',...args})
  const parsed=reference({op:'parsePatch',patch})
  check({op:'parsePatch',patch})
  check({op:'reversePatch',patch:parsed})
  check({op:'applyPatch',before,patch})
  check({op:'applyPatch',before:'prefix'+nl+before,patch:parsed})
}
console.log(language+' matched '+total+' deterministic differential cases')
