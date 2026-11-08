import vm from 'node:vm'
import {readFileSync,writeFileSync} from 'node:fs'
import {resolve} from 'node:path'
import {createRequire} from 'node:module'
import {createHash} from 'node:crypto'
const w=resolve(process.argv[2]),stem=process.argv[3],c=resolve(process.argv[4]),require=createRequire(resolve(process.cwd(),'experiments/fingerprintjs/package.json')),{rollup}=require('rollup'),{minify}=require('terser'),acorn=require('acorn'),{artifactGraphPlugin}=await import(resolve(c,'scripts/artifact-manifest.mjs')),rows=[]
for(const lane of ['baseline','jir']){
 const dir=resolve(w,stem+'-'+lane+'-on'),program=resolve(dir,'program.js'),bundle=await rollup({input:program,plugins:[artifactGraphPlugin([program+'.artifacts.json'])]}),code=(await bundle.generate({format:'iife',name:'candidate'})).output[0].code;await bundle.close()
 const min=(await minify(code,{ecma:2018,compress:true,mangle:true})).code
 for(const [stage,source]of [['rollup',code],['terser',min]]){
  acorn.parse(source,{ecmaVersion:2018});writeFileSync(resolve(dir,stage+'.js'),source)
  for(const globalKind of ['property','lexical']){
   const context=vm.createContext({})
   vm.runInContext(`globalThis.opaque={marker:7};globalThis.argument={marker:9};globalThis.failure=new Error('injected');globalThis.trace=[];globalThis.fail=false;globalThis.hostFunction=function(value){trace.push(['call',value===argument]);return value};`,context)
   const globals=globalKind==='property'?`for(const [name,value]of [['fixture_global',opaque],['_0',opaque],['a',opaque],['$t0',hostFunction]])Object.defineProperty(globalThis,name,{get(){trace.push(['get',name]);if(fail)throw failure;return value},configurable:true})`:`let fixture_global=opaque;let _0=opaque;let a=opaque;let $t0=hostFunction;`
   vm.runInContext(globals,context);vm.runInContext(source,context)
   const evaluate=expr=>{try{return vm.runInContext(expr,context)}catch(error){return{error:error?.message??String(error),identity:error===context.failure}}}
   const checks=[['local','candidate.matching_local()===opaque',true],['parameter','candidate.matching_parameter(argument)',false],['matching parameter','candidate.matching_parameter(opaque)',true],['numbered','candidate.numbered()===opaque',true],['short','candidate.short()===opaque',true],['temporary','candidate.temporary(argument)===argument',true],['helper','candidate.through_helper(argument)===opaque',true],['unused','candidate.unused()',17],['caught value','candidate.caught()===opaque',true]]
   for(const [name,expression,expected]of checks){const actual=evaluate(expression);rows.push({lane,stage,globalKind,name,passed:actual===expected,actual,expected})}
   const actualTrace=JSON.parse(JSON.stringify(context.trace)),expectedTrace=globalKind==='property'?[['get','fixture_global'],['get','fixture_global'],['get','fixture_global'],['get','_0'],['get','a'],['get','$t0'],['call',true],['get','fixture_global'],['get','fixture_global'],['get','fixture_global']]:[['call',true]]
   rows.push({lane,stage,globalKind,name:'ordered exact-once effects',passed:JSON.stringify(actualTrace)===JSON.stringify(expectedTrace),actual:actualTrace,expected:expectedTrace})
   if(globalKind==='property'){
    vm.runInContext('fail=true',context);const actual=evaluate('candidate.caught()===failure');rows.push({lane,stage,globalKind,name:'original exception',passed:actual===true,actual,expected:true})
   }
  }
 }
}
const sha=b=>createHash('sha256').update(b).digest('hex'),result={schema:1,rows,status:rows.every(row=>row.passed)?'pass':'fail',checks:rows.length,fixtureSha256:sha(readFileSync(resolve(c,'examples/global_shadow.rs'))),harnessSha256:sha(readFileSync(import.meta.filename)),scope:'Declared globals with same-spelled Rust locals/parameters, MIR numbered locals, generated argument temporaries, short minified names, helper inlining, discarded reads and original exceptions; actual lexical globals and observable global getters.'};writeFileSync(resolve(w,stem+'-result.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify({status:result.status,checks:rows.length,failures:rows.filter(r=>!r.passed).map(r=>[r.lane,r.stage,r.globalKind,r.name])}));
if(result.status!=="pass")process.exitCode=1
