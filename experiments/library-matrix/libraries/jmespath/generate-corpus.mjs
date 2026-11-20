// Supply a checkout of jmespath/jmespath.test at the revision in CONTRACT.md.
import { readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
const dir = process.argv[2]
if (!dir) throw new Error('expected the path to the compliance checkout')
const cases = []
for (const file of readdirSync(join(dir, 'tests')).sort()) {
  let i = 0
  const groups = JSON.parse(readFileSync(join(dir,'tests',file)))
  for (const { given, cases: group } of groups) {
    const queries = []
    const expected = []
    for (const c of group) {
      if (c.bench) continue
      queries.push({op:'search',expression:c.expression},{op:'compile',expression:c.expression})
      expected.push(c.error ? {error:c.error} : {value:c.result})
    }
    if (queries.length) cases.push({id:file.replace('.json','')+'-'+(++i),input:{data:given,queries},expected})
  }
}
const boundaryData = JSON.parse('{"foo":["a","b"],"__proto__":7,"~__proto__":{"__proto__":[1]}}')
const boundaryQueries = ['@', 'to_string(@)', 'keys(@)', '"__proto__"', '"~__proto__"',
  'keys({__proto__:foo[0]})', 'keys(merge(@))',
  'foo[0:4294967296]', 'foo[-4294967296:]', 'foo[4294967296]', 'foo[-4294967296]',
  'foo[::-4294967296]', 'foo[::4294967296]', 'foo[4294967296:-4294967296:-1]']
cases.push({id:'property-and-subscript-boundaries',input:{data:boundaryData,
  queries:boundaryQueries.flatMap(expression=>['search','compile'].map(op=>({op,expression})))}})
const data = {items: Array.from({length:1024},(_,i)=>({
  rank: (i*517)%1024, name: 'record-' + i, detail: ('entry '+i+' 日本語 | ').repeat(8), group: i%17
}))}
const queries = Array.from({length:100},(_,i)=>({op:'search',expression:i%2 ?
  'sum(items[?group == `'+(i%17)+'`].rank)' : 'items['+(i*17%1024)+'].name'}))
cases.push({id:'bench-search',input:{data,queries}})
writeFileSync(new URL('corpus.json',import.meta.url), JSON.stringify({schema:1,library:'jmespath',cases})+'\n')
