#!/usr/bin/env node
// Offline packaging checks. Never starts collection, networking, or a worker.
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'acorn';
const root=resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const [suite, input, output]=process.argv.slice(2);
if(!['oss','pro'].includes(suite)||!input||!output) throw new Error('usage: check-comparator-package.mjs oss|pro <dist> <receipt>');
const dist=resolve(root,input), checks=[];
async function record(name, action){try{await action();checks.push({name,passed:true});}catch(e){checks.push({name,passed:false,message:String(e.stack||e)});}}
const hash=b=>createHash('sha256').update(b).digest('hex');
async function load(path){const b=await readFile(path); return {namespace:await import(`data:text/javascript;base64,${b.toString('base64')}`),sha256:hash(b),bytes:b.length};}
const refPath=suite==='oss'?resolve(root,'experiments/fingerprintjs/dist/fp.esm.js'):resolve(root,'experiments-pro/fp-pro/packages/agent-static/dist/fp.with_dependencies.esm.min.js');
const reference=await load(refPath), identities=[];
for(const file of ['fp.esm.js','fp.esm.min.js']) {
 await record(file+'/syntax-and-import',async()=>{
  const bytes=await readFile(resolve(dist,file));
  const ast=parse(bytes.toString(),{ecmaVersion:'latest',sourceType:'module'});
  const imports=ast.body.filter(x=>x.type==='ImportDeclaration'||(x.source&&x.type.startsWith('Export')));
  assert.equal(imports.length,0,'packaged main entry must have no unresolved runtime imports');
  const candidate=await load(resolve(dist,file)); identities.push({file,sha256:candidate.sha256,bytes:candidate.bytes});
  assert.deepEqual(Object.keys(candidate.namespace).sort(),Object.keys(reference.namespace).sort(),'public export surface');
 });
}
await record('declaration-contract',async()=>{
 const ref=resolve(root,suite==='oss'?'experiments/fingerprintjs/dist/fp.d.ts':'experiments-pro/fp-pro/packages/agent-static/dist/fp.d.ts');
 assert.equal(hash(await readFile(resolve(dist,'fp.d.ts'))),hash(await readFile(ref)));
});
if(suite==='pro')for(const [file,expected] of [['fp.worker.js','./fp.esm.js'],['fp.worker.min.js','./fp.esm.min.js']]) {
 await record(file+'/dependency',async()=>{
  const tree=parse(await readFile(resolve(dist,file),'utf8'),{ecmaVersion:'latest',sourceType:'module'});
  const imports=tree.body.filter(x=>x.type==='ImportDeclaration').map(x=>x.source.value);
  // ReScript has an independently packaged worker instead of importing the main entry.
  assert.ok(imports.length===0 || JSON.stringify(imports)===JSON.stringify([expected]),`unexpected worker graph: ${imports}`);
  for(const spec of imports)await readFile(resolve(dist,spec));
 });
}
const result={schema:1,generatedAt:new Date().toISOString(),suite,dist:input,passed:checks.every(x=>x.passed),checks,artifacts:identities,reference:{path:refPath,sha256:reference.sha256},scope:'syntax, import without browser globals, public export names, declaration checksum, and worker dependency closure; declarations are copied contracts, not inferred API proofs; no collection, network, browser lifecycle, or timing'};
await mkdir(dirname(resolve(root,output)),{recursive:true});await writeFile(resolve(root,output),JSON.stringify(result,null,2)+'\n');
console.log(`${checks.filter(x=>x.passed).length}/${checks.length} package checks passed: ${input}`);
for(const x of checks.filter(x=>!x.passed))console.error(x.name,x.message);
if(!result.passed)process.exitCode=1;
