import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { run as reference } from './reference.mjs'
import { canonicalText } from '../../tools/lib/canonical.mjs'

const language = process.argv[2]
if (!/^[a-z]+$/.test(language ?? '')) throw new TypeError('expected a language id')
const {run} = await import(new URL('../../ports/jmespath/'+language+'/dist/index.min.js', import.meta.url))
const suite = JSON.parse(readFileSync(new URL('corpus.json',import.meta.url)))
// Version 0.16.0 predates a handful of compliance expectations. Assert the
// exact upstream return in each disagreement so changes on either side surface.
const deviations = new Map([
  ['boolean-4:emptylist < one',{value:true}],
  ['boolean-4:emptylist < nullvalue',{value:false}],
  ['boolean-4:emptylist < boolvalue',{value:false}],
  ['boolean-4:one < boolvalue',{value:false}],
  ['functions-1:avg(empty_list)',{value:NaN}],
  ['functions-3:max_by(`[]`, &age)',{value:undefined}],
  ['functions-3:min_by(`[]`, &age)',{value:undefined}],
  ['syntax-5:@(foo)',{error:'unknown-function'}],
  ['syntax-9:[:::]',{value:null}],
])
let searches=0
const seen = new Set()
for(const {id,input,expected} of suite.cases) {
  const npm=reference(input), port=run(input)
  assert.equal(canonicalText(port,'json'),canonicalText(npm,'json'),id)
  if (!expected) continue
  for (const [i,oracle] of expected.entries()) {
    const key=id+':'+input.queries[i*2].expression
    if(deviations.has(key)) seen.add(key)
    assert.deepEqual(npm[i*2],deviations.get(key) ?? oracle,key)
    searches++
  }
}
assert.equal(seen.size,deviations.size)
for(let i=0;i<400;i++){
  const data={rows:Array.from({length:i%13},(_,j)=>({key:(j*31+i)%7, value:i-j, label:'id'+j}))}
  const expr=['sort_by(rows,&key)[*].label','rows[?value > `'+i%19+'`].{x:key,y:label}',
    'rows[::-'+(1+i%4)+'].value','map(&[key, value], rows)','sum(rows[*].key)',
    'rows['+(i%16-8)+'].label || `"none"`','{xs:rows[*].value, first:rows[0]}']
  const input={data,queries:expr.flatMap(expression=>['search','compile'].map(op=>({op,expression})))}
  assert.equal(canonicalText(run(input),'json'),canonicalText(reference(input),'json'),'generated '+i)
}
// Exercise coercions across primitive types and recursive JSON values. In
// particular JS Number accepts hex, binary and octal strings in comparisons.
const values=[null,false,true,0,1,1.23,-2,'',' 12','0x12','0b101','0o17','1e3','bad','\t','😀a','É','Ä',[],[1],['2'],[null,true,4],{a:1},['a','é','D','Ä'],['10','2','-5'],[0,1,-2,1.234]]
const operators=['<','<=','>','>=','==','!=','||','&&']
const binary=[...operators.map(operator=>'a'+operator+'b'),'contains(a,b)','join(a,b)','starts_with(a,b)','ends_with(a,b)','not_null(a,b)']
const unary=['abs','avg','ceil','floor','length','max','min','reverse','sort','sum','to_array','to_number','to_string','type','values','keys']
let comparisons=0
for(const a of values) for(const [i,b] of values.entries()){
  const expressions=i===0 ? [...binary,...unary.map(f=>f+'(a)')] : binary
  const input={data:{a,b},queries:expressions.map(expression=>({op:'search',expression}))}
  assert.equal(canonicalText(run(structuredClone(input)),'json'),canonicalText(reference(input),'json'),JSON.stringify(input.data))
  comparisons+=expressions.length
}
console.log(language+' passed '+searches+' compliance searches and their ASTs, 100 benchmark searches, 5,600 generated checks and '+comparisons+' coercion checks')
const own = run({data:JSON.parse('{"__proto__":7}'),queries:[{op:'search',expression:'@'}]})[0].value
assert.ok(Object.hasOwn(own, '__proto__'))
assert.equal(own.__proto__, 7)
const {variant} = await import('./bench-variant.mjs')
const bench = suite.cases.find(c => c.id === 'bench-search').input
const first = variant(bench, 0)
for (const k of [1, 1024, 2048, 2**32]) {
  const next = variant(bench, k)
  assert.equal(next.data.items.length, first.data.items.length)
  assert.equal(next.queries, bench.queries)
  assert.ok(next.data.items.every((row, i) => row.name !== first.data.items[i].name))
}
