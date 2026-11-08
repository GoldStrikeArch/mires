import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createDriver, browserIdentity } from '../../../experiments-pro/shared/webdriver.mjs';
import { startServer } from '../../../experiments-pro/shared/static-server.mjs';
const work=pathToFileURL(resolve(process.argv[3])+'/'), built=pathToFileURL(resolve(process.argv[2])+'/');
const entries=['off','report','on'].flatMap(mode=>['baseline','jir'].map(lane=>({name:`${lane}-${mode}/program.mjs`,kind:mode})));
const control=process.argv[4]==='control';
const hash=data=>createHash('sha256').update(data).digest('hex');
const routes={'/':{body:'<!doctype html><title>Task exception cleanup</title>',type:'.html'}};
const inputs={};
for(const name of ['shim.mjs','topcoat-dom.mjs',...entries.map(entry=>entry.name)]) {
 const bytes=await readFile(new URL(name,built));inputs[name]=hash(bytes);routes['/'+name]={body:bytes,type:'.js'};
}
const checker=await readFile(new URL('./task-cleanup-check.mjs',import.meta.url));inputs.checker=hash(checker);routes['/check.mjs']={body:checker,type:'.js'};
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
       const {checkTaskCleanup}=await import('/check.mjs');
       return {...await checkTaskCleanup(await import('/'+name),await import('/topcoat-dom.mjs')),errors};
     })().then(value=>done({value}),error=>done({error:String(error)+'\n'+String(error.stack||'')}));
    },name,kind);
    assert.equal(actual.error,undefined);assert.equal(actual.value.passed,!control||kind==='on');assert.equal(actual.value.rows.length,14);assert.deepEqual(actual.value.errors,[]);
    result.cells.push({browser:identity,name,kind,actual:actual.value});console.log(browser,name,actual.value.rows.length,actual.value.passed?'pass':'expected rejection');
    await writeFile(new URL('browser.json',work),JSON.stringify(result,null,2)+'\n');
   }
  } finally {await driver.quit();}
 }
 if(!control) for(const cell of result.cells) assert.deepEqual(cell.actual,result.cells[0].actual);
 result.status='pass';
} catch(error){result.status='fail';result.error=String(error)+'\n'+String(error.stack||'');process.exitCode=1;}
finally {await server.close();await writeFile(new URL('browser.json',work),JSON.stringify(result,null,2)+'\n');}
