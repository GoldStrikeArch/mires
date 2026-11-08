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
 vm.runInContext(`globalThis.failure=new Error('injected');globalThis.trace=[];
 globalThis.coercible=function(label,result,kind){
  const object={};
  Object.defineProperty(object,Symbol.toPrimitive,{get(){trace.push(['get-primitive',label]);if(kind==='getter-throw')throw failure;if(kind==='bad-method')return 7;return function(hint){trace.push(['primitive',label,hint,this===object]);if(kind==='throw')throw failure;return kind==='object'?{}:result}}});return object
 };
 globalThis.ordinary=function(label){const object={};Object.defineProperty(object,'valueOf',{get(){trace.push(['get-valueOf',label]);return function(){trace.push(['valueOf',label,this===object]);return {}}}});object.toString=function(){trace.push(['toString',label,this===object]);return label};return object};
 globalThis.proxied=function(label){return new Proxy({valueOf(){trace.push(['proxy-valueOf',label]);return 5}},{get(target,key,receiver){trace.push(['proxy-get',label,String(key)]);return Reflect.get(target,key,receiver)}})};
 `,context)
 const normalize=value=>{
  if(value===undefined)return{type:'undefined'};if(value===null)return{type:'null'}
  if(typeof value==='number')return{type:'number',value:Number.isNaN(value)?'NaN':Object.is(value,-0)?'-0':String(value)}
  if(typeof value==='bigint')return{type:'bigint',value:String(value)}
  if(typeof value==='symbol')return{type:'symbol',value:String(value)}
  if(typeof value==='object')return{type:'error',name:value.name??null,message:value.message??String(value),identity:value===context.failure}
  return{type:typeof value,value}
 }
 const evaluate=(left,right,expression)=>{
  vm.runInContext('trace=[];globalThis.left=('+left+');globalThis.right=('+right+');',context)
  let result;try{result={value:normalize(vm.runInContext(expression,context))}}catch(error){result={error:normalize(error)}}
  return{...result,trace:structuredClone(context.trace)}
 }
 const compare=(name,left,right,actualExpression,expectedExpression)=>{
  const expected=evaluate(left,right,expectedExpression),actual=evaluate(left,right,actualExpression);rows.push({stage,name,passed:JSON.stringify(actual)===JSON.stringify(expected),actual,expected})
 }
 const values=['undefined','null','false','true','0','-0','NaN','Infinity','-Infinity','1','-1','1.5','""','"2"','"text"','BigInt(1)','BigInt(-1)','Symbol.for("key")','[]','[1]','({})','new Number(3)','new String("boxed")','new Date(0)']
 for(const left of values)for(const right of values)compare('value '+left+' + '+right,left,right,'candidate.add(left,right)','left+right')
 const effects=[
  ['numbers','coercible("left",2)','coercible("right",3)'],
  ['string','coercible("left","prefix")','coercible("right",3)'],
  ['left getter throws','coercible("left",2,"getter-throw")','coercible("right",3)'],
  ['left conversion throws','coercible("left",2,"throw")','coercible("right",3)'],
  ['right getter throws','coercible("left",2)','coercible("right",3,"getter-throw")'],
  ['right conversion throws','coercible("left",2)','coercible("right",3,"throw")'],
  ['nonprimitive result','coercible("left",2,"object")','coercible("right",3)'],
  ['noncallable conversion','coercible("left",2,"bad-method")','coercible("right",3)'],
  ['ordinary conversion','ordinary("left")','ordinary("right")'],
  ['proxy conversion','proxied("left")','proxied("right")'],
 ]
 for(const [name,left,right]of effects){
  compare(name,left,right,'candidate.add(left,right)','left+right')
  compare('first class '+name,left,right,'candidate.first_class(left,right)','left+right')
  compare('unused '+name,left,right,'candidate.unused(left,right)','(()=>{left+right})()')
  compare('caught '+name,left,right,'candidate.catch_add(left,right)','(()=>{try{return left+right}catch(error){return error}})()')
 }
 for(const mode of ['normal','left-operand-throw','right-operand-throw','conversion-throw']){
  const left='function(){trace.push("left-operand");'+(mode==='left-operand-throw'?'throw failure;':'return coercible("left",2,'+(mode==='conversion-throw'?'"throw"':'undefined')+');')+'}',right='function(){trace.push("right-operand");'+(mode==='right-operand-throw'?'throw failure;':'return coercible("right",3);')+'}'
  compare('evaluation '+mode,left,right,'candidate.evaluated(left,right)','left()+right()')
 }
}
const sha=b=>createHash('sha256').update(b).digest('hex'),result={schema:1,status:rows.every(row=>row.passed)?'pass':'fail',checks:rows.length,observations:rows,program:resolve(program),programSha256:sha(readFileSync(program)),harnessSha256:sha(readFileSync(import.meta.filename)),scope:'Native addition across 24 host-value forms, ordered primitive conversion/getters/proxies, exact operand evaluation, first-class dispatch, unused results and original exceptions, before and after Terser; ES2018 syntax.'}
writeFileSync(output,JSON.stringify(result,null,2)+'\n');const failures=rows.filter(row=>!row.passed);console.log(JSON.stringify({status:result.status,checks:rows.length,failed:failures.length,failures:failures.slice(0,20).map(row=>[row.stage,row.name])}));if(result.status!=='pass')process.exitCode=1
