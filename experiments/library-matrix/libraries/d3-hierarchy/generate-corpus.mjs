import { readFileSync, writeFileSync } from 'node:fs'
const cases = []
const defaults = { source:'hierarchy', width:640, height:480, padding:0, round:false, sort:false, nodeSize:false, tile:'squarify' }
const add = (id, op, args) => cases.push({ id, input:{ ...defaults, op, ...args } })
const datasets = {
  one:{id:'root',value:1}, zero:{id:'empty',value:0},
  asymmetric:{id:'root',value:.125,children:[
    {id:'a',value:0,children:[{id:'a.a',value:0},{id:'a.b',value:1}]},
    {id:'b',value:3.5},{id:'c',children:[{id:'c.a',value:9},{id:'c.b',value:2},{id:'c.c',value:2}]}]},
  wide:{id:'wide',children:Array.from({length:47},(_,i)=>({id:'n'+i,value:(i*71)%23+.5}))},
}
function generated(n, fan) {
  const nodes=Array.from({length:n},(_,i)=>({id:String(i),value:((i*17)%137)+1}))
  for(let i=1;i<n;i++) (nodes[Math.floor((i-1)/fan)].children ??=[]).push(nodes[i])
  return nodes[0]
}
datasets.deep=generated(72,1)
datasets.mixed=generated(153,4)
const flare=JSON.parse(readFileSync(new URL('flare.json',import.meta.url)))
const convert = (n,path='') => ({
  id:path+n.name,...(n.value === undefined ? {} : {value:n.value}),
  ...(n.children ? {children:n.children.map(c=>convert(c,path+n.name+'.'))} : {}),
})
datasets.flare=convert(flare)
for (const [name,data] of Object.entries(datasets)) {
  for(const op of ['hierarchy','iterator','tree','cluster','partition','pack','treemap']) {
    const options = op==='treemap' ? ['squarify','binary','sliceDice'] : ['squarify']
    for(const tile of options) {
      add(name+'-'+op+'-'+tile,op,{data,tile})
      if(name==='asymmetric'||name==='mixed') add(name+'-'+op+'-'+tile+'-options',op,
        {data,tile,sort:true,round:true,padding:2.25,nodeSize:true,width:61.5,height:45.75})
    }
  }
}
for(const [name, rows] of Object.entries({
  ordinary:[{id:'x',parentId:'r',value:4},{id:'r'},{id:'z',parentId:'x',value:2}],
  duplicateLeaves:[{id:'r'},{id:'x',parentId:'r'},{id:'x',parentId:'r'}],
  missing:[{id:'r'},{id:'x',parentId:'nope'}], cycle:[{id:'r'},{id:'x',parentId:'y'},{id:'y',parentId:'x'}],
  ambiguous:[{id:'r'},{id:'x',parentId:'r'},{id:'x',parentId:'r'},{id:'y',parentId:'x'}],
  multiple:[{id:'x'},{id:'y'}], noRoot:[{id:'x',parentId:'x'}], empty:[],
})) add('stratify-'+name,'stratify',{source:'id',rows})
for (const paths of [['a/b','a/c','a//d'], ['one/two/three'], ['a\\/b/c','a\\/b/d','q/'], ['/','/x/','/x/y'],[],['/','//','/a///b']]) {
  add('path-'+cases.length,'stratify',{source:'path',rows:paths.map((id,i)=>({id,value:i+1}))})
}
add('negative-zero-leaf','hierarchy',{data:{id:'root',value:-0}})
add('negative-zero-stratify','stratify',{source:'id',rows:[
  {id:'root',value:-0},{id:'a',parentId:'root',value:-0},{id:'b',parentId:'root',value:0}
]})

// Review 2026-11-20 (build/library-matrix/review-20261120/b7): the failing inputs, then adversarial
// cases for the classes earlier reviews found in ports that shared one generated design.
const leaves = (values, id = 'r') => ({id, children:values.map((value,i)=>({id:'c'+i, value}))})
const chainData = depth => {
  let node = {id:'n'+depth, value:1}
  for (let i = depth - 1; i >= 0; i--) node = {id:'n'+i, value:i % 3, children:[node]}
  return node
}
const chainRows = depth => Array.from({length:depth}, (_, i) => i ? {id:'n'+i, parentId:'n'+(i-1), value:i % 5} : {id:'n0'})

// Math.round, not floor(x + 0.5): halves round up, -0 and [-0.5, 0) round to -0, and an odd
// integer past 2^52 stays as it is.
add('round-almost-half','treemap',{data:{id:'r',value:1},round:true,width:0.49999999999999994,height:2.5})
add('round-odd-past-2p52','treemap',{data:{id:'r',value:1},round:true,width:2**52+1,height:3})
add('round-partition-past-2p52','partition',{data:{id:'r',value:1},round:true,width:2**52+1,height:2**53-2})
add('round-negative-halves','treemap',{data:leaves([1,1,1,1]),round:true,width:-5,height:-3})
add('round-negative-partition','partition',{data:leaves([1,1,1]),round:true,width:-2.5,height:-1.5,padding:.5})
add('round-slicedice-past-2p53','treemap',{data:leaves([1,1,1]),round:true,tile:'sliceDice',width:3*(2**52+1),height:7})
add('round-halves-binary','treemap',{data:leaves([1,1,2,4,0.5]),round:true,tile:'binary',width:5,height:3,padding:.5})

// Pack: zero or negative size with padding, and radii whose squares overflow. d3 throws on several
// ("something is very wrong" in enclose), and the port must throw too.
for (const [name, values, width, height, padding] of [
  ['zero-height-3',[1,1,1],1,0,1], ['zero-height-4',[1,2,3,4],1,0,1], ['zero-width-5',[1,2,3,4,5],0,10,.5],
  ['overflow-3',[1e308,1e308,1e308],10,10,0], ['overflow-5',[1e300,1e308,2e307,1e308,5e307],10,10,0],
  ['all-zero',[0,0,0,0],10,10,0], ['negative-width',[1,2,3,4],-10,10,1], ['negative-padding',[1,2,3,4,5,6],100,80,-3],
  ['huge-padding',[3,1,4,1,5,9,2,6],100.5,33.25,1000], ['tiny-values',[5e-324,1e-300,5e-324,3e-301,0],640,480,1],
]) add('pack-'+name,'pack',{data:leaves(values),width,height,padding})
add('pack-zero-root','pack',{data:{id:'r',value:0},width:10,height:10,padding:1})

// JavaScript truthiness of NaN (`value ? … : y1`, `!sumValue`, `if (d2)`): weights near 1e308
// overflow the aggregates to Infinity, and Infinity - Infinity is NaN.
add('overflow-binary','treemap',{tile:'binary',data:leaves([1e308,1e308,1e308])})
add('overflow-binary-nested','treemap',{tile:'binary',round:true,width:0,padding:2.25,data:{id:'r',children:[
  {id:'a',children:[{id:'a0',value:1e308},{id:'a1',value:1e308},{id:'a2',children:[{id:'a20',value:1e308},{id:'a21',value:7}]}]},
  {id:'b',value:1e308},{id:'c',value:3}]}})
add('overflow-squarify','treemap',{data:leaves([1e308,1e308,1,1e308])})
add('overflow-squarify-padded','treemap',{width:100.5,height:33.25,padding:2.25,data:leaves([1e308,9.7e299,1e308,4.2e299,1e308])})
add('overflow-slicedice','treemap',{tile:'sliceDice',data:leaves([1e308,1e308,2])})
add('overflow-partition','partition',{data:leaves([1e308,1e308,5])})
add('overflow-sort','hierarchy',{sort:true,data:{id:'r',children:[
  {id:'a',children:[{id:'a1',value:1e308},{id:'a2',value:1e308}]}, {id:'b',value:5},
  {id:'c',children:[{id:'c1',value:-1e308},{id:'c2',value:-1e308}]},
  {id:'d',children:[{id:'d1',value:1e308},{id:'d2',value:1e308},{id:'d3',value:-1e308},{id:'d4',value:-1e308}]},
  {id:'e',value:7}]}})
for (const [op, tile] of [['treemap','squarify'],['treemap','binary'],['treemap','sliceDice'],['partition'],['pack'],['tree']]) {
  add('nan-aggregate-'+op+(tile ? '-'+tile : ''),op,{tile:tile ?? 'squarify',data:{id:'r',children:[
    {id:'a',children:[{id:'a0',value:1e308},{id:'a1',value:-1e308},{id:'a2',value:1e308}]},
    {id:'b',value:2},{id:'c',children:[{id:'c0',value:-1e308},{id:'c1',value:1e308}]},{id:'d',value:1}]}})
}

// Negative and tiny weights, which Elm's NaN comparisons broke: a row's sum can be 0 or negative,
// and alpha * tiny can underflow.
for (const [op, tile] of [['treemap','squarify'],['treemap','binary'],['treemap','sliceDice'],['partition'],['pack'],['cluster']]) {
  add('negative-weights-'+op+(tile ? '-'+tile : ''),op,{tile:tile ?? 'squarify',data:leaves([3,-1,2,-4])})
}
add('negative-weights-nested','treemap',{sort:true,padding:1,data:{id:'r',children:[
  leaves([2,-5,1],'a'),{id:'b',value:-3},leaves([-1,-1,4,0],'c'),{id:'d',value:6}]}})
for (const tile of ['squarify','binary','sliceDice']) {
  add('tiny-weights-'+tile,'treemap',{tile,data:leaves([5e-324,1e-300,5e-324,3e-301,2e-310,0,7e-305])})
}

// Math.max and Math.min with -0 and NaN: a zero-sized rectangle makes squarify's ratios NaN, and a
// width or height of -0 makes pack's Math.min(dx, dy) -0 whichever operand it is.
for (const [width, height] of [[0,0],[-0,480],[640,-0],[-0,0],[0,-0]]) {
  const size = (Object.is(width,-0) ? 'n' : '')+width+'x'+(Object.is(height,-0) ? 'n' : '')+height
  for (const [op, tile] of [['treemap','squarify'],['treemap','binary'],['pack'],['partition'],['tree'],['cluster']]) {
    add('size-'+size+'-'+op+(tile ? '-'+tile : ''),op,{tile:tile ?? 'squarify',width,height,padding:op === 'pack' ? 0 : 1,data:datasets.asymmetric})
  }
}

// Error order: d3 throws the first error in row order, and a cycle only after every row linked.
for (const [name, rows] of Object.entries({
  rootsBeforeMissing:[{id:'a'},{id:'b'},{id:'c',parentId:'z'}],
  missingBeforeRoots:[{id:'c',parentId:'z'},{id:'a'},{id:'b'}],
  ambiguousBeforeMissing:[{id:'r'},{id:'x',parentId:'r'},{id:'x',parentId:'r'},{id:'y',parentId:'x'},{id:'z',parentId:'q'}],
  cycleAndRoot:[{id:'r'},{id:'a',parentId:'b'},{id:'b',parentId:'a'},{id:'c',parentId:'r'}],
  selfParent:[{id:'r'},{id:'s',parentId:'s'}],
  emptyIds:[{id:'r'},{id:'',parentId:'r',value:2},{id:'',parentId:'r',value:3},{id:'x',parentId:'r'}],
  unicodeIds:[{id:'🌻'},{id:'',parentId:'🌻',value:1},{id:'é',parentId:'🌻',value:2},{id:'日本',parentId:'',value:3}],
})) add('stratify-'+name,'stratify',{source:'id',rows})
for (const paths of [['🌻/','🌻/é/日本','\\/x/y'], ['a\\\\/b','a\\\\\\/b/c','a\\'], ['x//','//y//','///']]) {
  add('path-unicode-'+cases.length,'stratify',{source:'path',rows:paths.map((id,i)=>({id,value:i+1}))})
}

// Past int32 and past 2^53: weights whose sums round, and sizes beyond 32-bit integers.
add('weights-past-2p53','treemap',{tile:'binary',data:leaves([2**53,1,1,2**53+2,3])})
add('weights-past-2p53-squarify','treemap',{data:leaves([2**53,1,1,2**53+2,3])})
add('weights-past-int32','hierarchy',{sort:true,data:leaves([2**31,2**32+1,-(2**31)-1,3e9])})
for (const op of ['tree','cluster','pack','partition','treemap']) {
  add('size-past-int32-'+op,op,{width:2**31+0.5,height:2**32,nodeSize:op === 'tree',data:datasets.mixed})
}

// Deep and wide inputs, against stack overflow. d3 builds hierarchy() and walks every tree on
// explicit stacks; nested data deeper than about 1,500 levels does not survive the checker's
// structuredClone, so the deep cases beyond that are flat rows (verify.mjs checks 10,000 nested
// levels directly).
for (const op of ['hierarchy','tree','pack','treemap']) add('deep-nested-1000-'+op,op,{data:chainData(1000)})
for (const op of ['iterator','tree','cluster','pack','partition','treemap']) add('deep-rows-10000-'+op,op,{source:'id',rows:chainRows(10000)})
// One path row imputes 2,999 ancestors; the row /p0 stops d3's root trimming at the top.
add('deep-path-3000','iterator',{source:'path',rows:[{id:'p0'},{id:Array.from({length:3000},(_, i)=>'p'+(i % 10)).join('/'),value:1}]})
add('wide-pack-10000','pack',{padding:.5,data:leaves(Array.from({length:10000},(_, i)=>1 + (i % 7)))})
add('wide-squarify-20000','treemap',{data:leaves(Array.from({length:20000},(_, i)=>1 + (i % 7)))})
add('wide-sort-10000','treemap',{sort:true,tile:'sliceDice',data:leaves(Array.from({length:10000},(_, i)=>i % 7))})
// binary.js recurses once per split: zero weights split one child off at a time.
add('wide-binary-zeros-1000','treemap',{tile:'binary',data:leaves(Array.from({length:1000},()=>0))})

// null where d3 reads the field with `!= null` or `??` means absent: parentId null marks a root,
// children null a leaf, value null weighs 0, and a missing or null id comes back as the reference's
// `n.data.id`, undefined or null.
const nullRows = [{id:'a',parentId:null,value:1},{id:'b',parentId:'a',value:null},{id:'c',parentId:'a',value:2,children:null}]
add('null-parentId','stratify',{source:'id',rows:nullRows})
add('null-parentId-treemap','treemap',{source:'id',rows:nullRows})
add('null-parentId-roots','stratify',{source:'id',rows:[{id:'a',parentId:null},{id:'b',parentId:null}]})
const nullData = {id:'r',value:null,children:[{id:'a',value:1,children:null},{id:'b',value:2,children:[]},
  {id:'c',value:null,children:[{id:'c0',value:3,children:null},{id:'c1',value:null}]}]}
for (const op of ['hierarchy','treemap','pack','tree','partition']) add('null-fields-'+op,op,{data:nullData})
const nullIds = {id:null,children:[{id:'a',value:1},{value:2},{id:null,value:3,children:[{value:4},{id:'',value:5}]}]}
for (const op of ['hierarchy','iterator','cluster']) add('null-ids-'+op,op,{data:nullIds})
const nullIdRows = [{id:'r'},{id:null,parentId:'r',value:1},{parentId:'r',value:2},{id:'',parentId:'r',value:3},{id:'x',parentId:'r'}]
for (const op of ['stratify','iterator','treemap']) add('null-ids-rows-'+op,op,{source:'id',rows:nullIdRows})

// sort with a NaN aggregate among two siblings: d3's comparator returns NaN, counted as 0, so the
// two keep their order (among three or more the order is the engine's: CONTRACT.md).
const infinite = (id, sign) => ({id, children:[{id:id+'0',value:sign*1e308},{id:id+'1',value:sign*1e308}]})
const nanNode = id => ({id, children:[infinite(id+'p',1), infinite(id+'n',-1)]})
for (const op of ['hierarchy','treemap','tree','partition']) {
  add('nan-sort-two-'+op,op,{sort:true,data:{id:'r',children:[nanNode('a'),{id:'b',value:3}]}})
}
add('nan-sort-two-reversed','hierarchy',{sort:true,data:{id:'r',children:[{id:'b',value:3},nanNode('a')]}})
add('nan-sort-two-both','treemap',{sort:true,data:{id:'r',children:[nanNode('a'),nanNode('c')]}})

for(const op of ['treemap','pack'])add('bench-'+op,op,{data:generated(10000,6),sort:true,padding:.15})
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'d3-hierarchy',cases}, (_key,value) => Object.is(value,-0) ? JSON.rawJSON('-0') : value)+'\n')
