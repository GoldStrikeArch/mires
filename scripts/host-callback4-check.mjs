import assert from 'node:assert/strict'
import vm from 'node:vm'
import {readFileSync,writeFileSync} from 'node:fs'
import {resolve} from 'node:path'
import {createRequire} from 'node:module'
import {createHash} from 'node:crypto'

const root=process.cwd(),require=createRequire(resolve(root,'experiments/fingerprintjs/package.json'))
const {rollup}=require('rollup'),{minify}=require('terser'),acorn=require('acorn')
const [program,output,compiler,fault]=process.argv.slice(2)
assert.ok(!fault||fault==='drop-fourth')
const {artifactGraphPlugin}=await import(resolve(compiler,'scripts/artifact-manifest.mjs'))
let graph
const bundle=await rollup({input:resolve(program),plugins:[artifactGraphPlugin([resolve(program)+'.artifacts.json'],{onReceipt:value=>{graph=value}})]})
const code=(await bundle.generate({format:'iife',name:'candidate'})).output[0].code
await bundle.close()
const min=(await minify(code,{ecma:2018,compress:true,mangle:true})).code,observations=[]
const cases=[]
for(let arity=0;arity<=6;arity++)cases.push({name:'arity '+arity,arity})
for(const kind of ['object','symbol','function','null','undefined','negative-zero','bigint','proxy'])cases.push({name:'opaque '+kind,kind})
for(let index=0;index<4;index++)cases.push({name:'independent capture '+index,index})
for(const kind of ['error','object','symbol','undefined'])cases.push({name:'throw '+kind,throws:kind})
for(const [stage,source]of [['rollup',code],['terser',min]]){
 acorn.parse(source,{ecmaVersion:2018});writeFileSync(output+'.'+stage+'.js',source)
 for(const spec of cases){
  const context=vm.createContext({spec})
  vm.runInContext(source+(fault?';const original=candidate.make;candidate.make=seed=>{const callback=original(seed);return (a,b,c,d)=>callback(a,b,c,undefined)};':''),context)
  const actual=vm.runInContext(`(()=>{
   const seed={},opaque={},symbol=Symbol('opaque'),fn=()=>{},trace=[];
   const values={object:opaque,symbol,function:fn,null:null,undefined:undefined,'negative-zero':-0,bigint:BigInt('18446744073709551615'),proxy:new Proxy(opaque,{get(){trace.push('unexpected get');throw opaque}})};
   if(spec.throws){
    const thrown={error:new Error('injected'),object:opaque,symbol,undefined:undefined}[spec.throws],callback=candidate.make_throwing(thrown),rows=[];
    for(let i=0;i<3;i++){let caught=false;try{callback(1,2,3,4)}catch(error){caught=Object.is(error,thrown)}rows.push(caught)}
    return {length:callback.length,rows};
   }
   if(spec.index!==undefined){
    const callbacks=candidate.make_many(),callback=callbacks[spec.index],rows=[];
    for(let i=0;i<3;i++){const result=callback(opaque,symbol,fn,i);rows.push(result.length===5&&result[0]===spec.index&&result[1]===opaque&&result[2]===symbol&&result[3]===fn&&result[4]===i)}
    return {length:callback.length,count:callbacks.length,rows};
   }
   const callback=candidate.make(seed),args=spec.kind?[values[spec.kind],opaque,symbol,values[spec.kind]]:Array.from({length:spec.arity},(_,i)=>({index:i})),rows=[];
   for(let i=0;i<3;i++){
    const result=callback.apply({ignoredReceiver:true},args);
    rows.push(Array.isArray(result)&&result.length===5&&result[0]===seed&&[0,1,2,3].every(index=>Object.is(result[index+1],args[index])));
   }
   return {length:callback.length,rows,trace};
  })()`,context)
  const expected=spec.throws?{length:4,rows:[true,true,true]}:spec.index!==undefined?{length:4,count:4,rows:[true,true,true]}:{length:4,rows:[true,true,true],trace:[]}
  observations.push({stage,name:spec.name,passed:JSON.stringify(actual)===JSON.stringify(expected),actual,expected})
 }
}
const hash=value=>createHash('sha256').update(value).digest('hex')
const inputs=Object.fromEntries([program,import.meta.filename,resolve(compiler,'examples/host_callback4.rs'),require.resolve('rollup'),require.resolve('terser'),require.resolve('acorn')].map(path=>[resolve(path),hash(readFileSync(path))]))
for(const row of graph.modules)assert.equal(hash(readFileSync(row.path)),row.sha256)
const receipt={schema:1,status:observations.every(row=>row.passed)?'pass':'fail',checks:observations.length,observations,inputs,graph,fault:fault||null,timingAuthority:false,promotionAuthority:false}
writeFileSync(output,JSON.stringify(receipt,null,2)+'\n')
console.log(JSON.stringify({status:receipt.status,checks:receipt.checks,failed:observations.filter(row=>!row.passed).length}))
if(receipt.status!=='pass')process.exitCode=1
