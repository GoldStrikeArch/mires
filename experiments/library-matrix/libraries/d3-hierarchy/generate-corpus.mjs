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
for(const op of ['treemap','pack'])add('bench-'+op,op,{data:generated(10000,6),sort:true,padding:.15})
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'d3-hierarchy',cases}, (_key,value) => Object.is(value,-0) ? JSON.rawJSON('-0') : value)+'\n')
