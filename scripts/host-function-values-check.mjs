import assert from 'node:assert/strict'
import vm from 'node:vm'
import {readFileSync,writeFileSync} from 'node:fs'
import {resolve} from 'node:path'
import {createRequire} from 'node:module'
import {createHash} from 'node:crypto'
const root=process.cwd(),require=createRequire(resolve(root,'experiments/fingerprintjs/package.json')),{rollup}=require('rollup'),{minify}=require('terser'),acorn=require('acorn')
const [program,output,compiler]=process.argv.slice(2),{artifactGraphPlugin}=await import(resolve(compiler,'scripts/artifact-manifest.mjs'))
const bundle=await rollup({input:resolve(program),plugins:[artifactGraphPlugin([resolve(program)+'.artifacts.json'])]}),code=(await bundle.generate({format:'iife',name:'candidate'})).output[0].code;await bundle.close()
const min=(await minify(code,{ecma:2018,compress:true,mangle:true})).code,rows=[]
for(const [stage,source]of [['rollup',code],['terser',min]]){
 acorn.parse(source,{ecmaVersion:2018});writeFileSync(output+'.'+stage+'.js',source)
 const context=vm.createContext({});vm.runInContext(source,context)
 vm.runInContext(`globalThis.trace=[];globalThis.opaque={};globalThis.failure=new Error('injected');globalThis.fail=false;globalThis.stored=opaque;
 Object.defineProperty(globalThis,'fixtureValue',{get(){trace.push('global');if(fail)throw failure;return stored}});
 globalThis.receiver={get value(){return stored},set value(value){trace.push(['set',value===opaque]);if(fail)throw failure;stored=value},get read(){trace.push('method-get');if(fail)throw failure;return function(arg){trace.push(['method-call',this===receiver,arg===opaque]);if(arg===failure)throw failure;return arg}},get item(){trace.push('index-get');if(fail)throw failure;return opaque}};
 globalThis.FixtureConstructor=function(value){trace.push(['construct',new.target===FixtureConstructor,value===opaque]);if(fail)throw failure;this.value=value};`,context)
 const run=expression=>vm.runInContext(expression,context),check=(name,expr,expected)=>{const actual=run(expr);assert.deepEqual(JSON.parse(JSON.stringify(actual)),expected,name);rows.push({stage,name,passed:true})}
 check('global value','candidate.get_value()===opaque',true)
 check('property setter','candidate.put_value(receiver,opaque)===opaque && receiver.value===opaque',true)
 check('method receiver','candidate.method(receiver,opaque)===opaque',true)
 check('computed getter','candidate.element(receiver,"item")===opaque',true)
 check('constructor','candidate.construct(opaque).value===opaque',true)
 check('ordered effects','trace',['global',['set',true],'method-get',['method-call',true,true],'index-get',['construct',true,true]])
 run('fail=true;trace=[]')
 for(const [name,expr]of [['global failure','candidate.get_value()'],['setter failure','candidate.put_value(receiver,opaque)'],['method lookup failure','candidate.method(receiver,opaque)'],['index failure','candidate.element(receiver,"item")'],['constructor failure','candidate.construct(opaque)']])check(name,expr+'===failure',true)
 run('fail=false')
 check('method call failure','candidate.method(receiver,failure)===failure',true)
}
const sha=value=>createHash('sha256').update(value).digest('hex');writeFileSync(output,JSON.stringify({schema:1,status:'pass',checks:rows.length,rows,program:resolve(program),programSha256:sha(readFileSync(program)),harnessSha256:sha(readFileSync(import.meta.filename)),scope:'Native getter, setter, method, index and constructor host bindings used as Rust function values, including ordered effects, original receiver/value/error identity, and host exceptions through try_host.'},null,2)+'\n');console.log('host function values',rows.length,'passed')
