import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { run as reference } from './reference.mjs'

import { variant } from './bench-variant.mjs'

const language = process.argv[2]
if (!/^[a-z]+$/.test(language ?? '')) throw new Error('expected a matrix language id')
const { run } = await import(new URL('../../ports/d3-hierarchy/'+language+'/dist/index.min.js',import.meta.url))
const flare = JSON.parse(readFileSync(new URL('flare.json', import.meta.url)))
function alphabetic(n) {
  return { id:n.name, value:n.value ?? 0, ...(n.children ? {
    children:[...n.children].sort((a,b)=>a.name.localeCompare(b.name)).map(alphabetic),
  } : {}) }
}
// SHA-256 of the upstream flare-pack.json answers as preorder [name,x,y,r]
// rows, rounded to two decimal places as in upstream test/pack/flare-test.js.
const expected = 'e71746fe8a8036211b414f44cfa359d09c6741988f98bd2f9de520a80777a286'
const base = {op:'pack',source:'hierarchy',data:alphabetic(flare),width:960,height:960,padding:0,
  tile:'squarify',round:false,nodeSize:false,sort:true}
for (const impl of [reference,run]) {
  const rows = impl(base).map(row=>[row[0],...row.slice(5).map(x=>Math.round(x*100)/100)])
  assert.equal(rows.length,252)
  assert.equal(createHash('sha256').update(JSON.stringify(rows)).digest('hex'),expected)
}
const corpus = JSON.parse(readFileSync(new URL('corpus.json', import.meta.url))).cases
for (const {id,input} of corpus.filter(c => c.id.startsWith('negative-zero-'))) {
  assert.ok(Object.is(input.data?.value ?? input.rows[0].value,-0),id)
  for (const impl of [reference,run]) {
    for (const row of impl(input)) assert.ok(Object.is(row[4],0),id)
  }
}
// The Node checker hands its cells the corpus through JSON.stringify, which writes -0 as 0, so in
// Node the sizes of -0 are checked here, on the file's own values.
function hasNegativeZero(value) {
  const pending = [value]
  while (pending.length) {
    const v = pending.pop()
    if (Object.is(v,-0)) return true
    if (v && typeof v === 'object') pending.push(...Object.values(v))
  }
  return false
}
let negativeZeroCases = 0
for (const {id,input} of corpus.filter(c => !c.id.startsWith('bench-') && hasNegativeZero(c.input))) {
  assert.deepStrictEqual(Array.from(run(input), row => Array.from(row)),reference(input),id)
  negativeZeroCases++
}
for (const {input} of corpus.filter(c => c.id.startsWith('bench-'))) {
  const first = variant(input,1), later = variant(input,806)
  function compareNodes(original,a,b) {
    assert.equal(a.id,original.id)
    assert.equal(b.id,original.id)
    assert.notEqual(a.value,original.value)
    assert.notEqual(a.value,b.value)
    for (const [i,node] of (original.children ?? []).entries()) compareNodes(node,a.children[i],b.children[i])
  }
  compareNodes(input.data,first.data,later.data)
  for (const [left,right] of [[1,1 + 2 ** 32],[Number.MAX_SAFE_INTEGER - 1,Number.MAX_SAFE_INTEGER]]) {
    const a = variant(input,left), b = variant(input,right)
    const pending = [[a.data,b.data]]
    while (pending.length) {
      const [x,y] = pending.pop()
      assert.ok(x.id !== y.id || x.value !== y.value)
      for (const [i,child] of (x.children ?? []).entries()) pending.push([child,y.children[i]])
    }
  }
}
let state=0xc764ab2
function random() { state^=state<<13;state^=state>>>17;state^=state<<5;return state>>>0 }
let checks=0
for (let k=1;k<=64;k++) {
  const nodes=Array.from({length:2*k},(_,i)=>({id:'n'+i,value:random()%107/10,children:[]}))
  for(let i=1;i<nodes.length;i++) nodes[random()%i].children.push(nodes[i])
  for(const [op,tile] of [['hierarchy'],['iterator'],['tree'],['cluster'],['partition'],['pack'],
    ['treemap','squarify'],['treemap','binary'],['treemap','sliceDice']]) {
    const input={...base,op,data:nodes[0],tile:tile??'squarify',width:100+k/3,height:200-k/5,
      sort:k%2===0,nodeSize:k%3===0,round:k%4===0,padding:(k%5)/2}
    // deepStrictEqual compares numbers with Object.is: -0 differs from 0, and NaN equals NaN.
    assert.deepStrictEqual(Array.from(run(input), row => Array.from(row)),reference(input),op+' '+tile+' on tree '+k)
    checks++
  }
}
// d3 builds hierarchy() and walks every tree on explicit stacks. A single-child chain 10,000
// levels deep is nested beyond what the checker's structuredClone carries (about 1,500 levels),
// so it is checked here, on one input object per layout that neither side may change.
function chain(depth) {
  let node = {id:'n'+depth, value:1}
  for (let i = depth - 1; i >= 0; i--) node = {id:'n'+i, value:i % 3, children:[node]}
  return node
}
const deep = chain(10000)
let deepChecks = 0
for (const [op,tile] of [['hierarchy'],['iterator'],['tree'],['cluster'],['partition'],['pack'],['treemap','squarify'],['treemap','binary']]) {
  const input = {...base,op,data:deep,tile:tile??'squarify',width:640,height:480,padding:1,sort:true}
  assert.deepStrictEqual(Array.from(run(input), row => Array.from(row)),reference(input),op+' '+(tile??'')+' on a 10,000-level chain')
  deepChecks++
}
console.log(language+' passed the 252-node upstream pack fixture, '+checks+' exact layout comparisons, '+negativeZeroCases+' corpus cases with -0 inputs and '+deepChecks+' layouts of a 10,000-level chain')
