import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createDriver, browserIdentity } from '../../../experiments-pro/shared/webdriver.mjs';
import { startServer } from '../../../experiments-pro/shared/static-server.mjs';
const work=pathToFileURL(resolve(process.argv[3])+'/'), built=pathToFileURL(resolve(process.argv[2])+'/');
const ready=[...['off','report','on'].flatMap(mode=>['baseline','jir'].map(lane=>`${lane}-${mode}/program.mjs`)), 'jir-control/program.mjs','jir-retry/program.mjs',...['jir-control','jir-on','jir-retry'].map(arm=>`packages/${arm}.mjs`)];
const entries=[...ready.map(name=>({name,kind:'ready'})),...['custom','suspending','local_drop','captured_drop','helper_drop','borrowed','mixed','inlined_helper'].map(kind=>({name:`${kind}/program.mjs`,kind}))];
const hash=data=>createHash('sha256').update(data).digest('hex');
const routes={'/':{body:'<!doctype html><title>Ready Promise boundary</title>',type:'.html'}};
const inputs={};
for(const name of ['shim.mjs',...entries.map(entry=>entry.name)]) {
 const bytes=await readFile(new URL(name,built));inputs[name]=hash(bytes);routes['/'+name]={body:bytes,type:'.js'};
}
const checker=await readFile(new URL('./promise-ready-check.mjs',import.meta.url));inputs.checker=hash(checker);routes['/check.mjs']={body:checker,type:'.js'};
const server=await startServer(routes), result={status:'running',inputs,cells:[]};
try {
 for(const browser of ['chrome','firefox','safari']) {
  const driver=await createDriver(browser,fileURLToPath(new URL('../../../experiments/fingerprintjs/',import.meta.url)));
  try {
   const identity=await browserIdentity(driver);
   for(const {name,kind} of entries) {await driver.get(server.origin);
    const actual=await driver.executeAsyncScript(function(name,kind,done){
     const errors=[];
     addEventListener('error',event=>errors.push(String(event.error)));
     addEventListener('unhandledrejection',event=>errors.push(String(event.reason)));
     (async()=>{
       const {checkReady}=await import('/check.mjs');
       return {...await checkReady(await import('/'+name),kind),errors};
     })().then(value=>done({value}),error=>done({error:String(error)+'\n'+String(error.stack||'')}));
    },name,kind);
    assert.equal(actual.error,undefined);assert.equal(actual.value.passed,true);assert.equal(actual.value.cases,kind==='ready'?12:3);assert.deepEqual(actual.value.errors,[]);
    result.cells.push({browser:identity,name,kind,actual:actual.value});console.log(browser,name,actual.value.cases,'pass');
    await writeFile(new URL('browser.json',work),JSON.stringify(result,null,2)+'\n');
   }
  } finally {await driver.quit();}
 }
 for(const cell of result.cells) assert.deepEqual(cell.actual,result.cells.find(first=>first.kind===cell.kind).actual);
 result.status='pass';
} catch(error){result.status='fail';result.error=String(error)+'\n'+String(error.stack||'');process.exitCode=1;}
finally {await server.close();await writeFile(new URL('browser.json',work),JSON.stringify(result,null,2)+'\n');}
