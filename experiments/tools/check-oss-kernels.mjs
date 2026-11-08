#!/usr/bin/env node
// Deterministic, offline comparisons of the exact packaged reference and compiled Rust module.
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const input=process.argv[2] || 'experiments/fingerprintjs-rust-compact/dist/mires-v2/fp.esm.min.js';
const output=resolve(root, process.argv[3] || 'build/assembly/oss-kernels.json');
async function load(path) {
  const bytes=await readFile(path);
  return {module:await import(`data:text/javascript;base64,${bytes.toString('base64')}`),path,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};
}
const reference=await load(resolve(root,'experiments/fingerprintjs/dist/fp.esm.js'));
const candidate=await load(resolve(root,input));
const results=[];
function compare(name,fn) {try{fn();results.push({name,passed:true});}catch(e){results.push({name,passed:false,message:e.message});}}
const texts=['','a','hello','\0','αβγ','日本語','😀🍀','\ufeffheader','a\r\nb','a'.repeat(16),'x'.repeat(17),'abc'.repeat(341)];
for(const [i,text] of texts.entries())for(const seed of [0,1,42,0xffffffff])
 compare(`murmur/text-${i}/seed-${seed}`,()=>assert.equal(candidate.module.murmurX64Hash128(text,seed),reference.module.murmurX64Hash128(text,seed)));
const components=[{}, {a:{value:'hello',duration:1}}, {z:{value:[1,'two',null],duration:2},a:{value:false,duration:9}}, {'a:b|c\\d':{value:{x:3,y:'α😀'},duration:0}}, {failure:{error:new Error('fixture failure'),duration:4}}];
for(const [i,value] of components.entries()) {
 compare(`component-hash/${i}`,()=>assert.equal(candidate.module.hashComponents(value),reference.module.hashComponents(value)));
 compare(`component-debug/${i}`,()=>assert.equal(candidate.module.componentsToDebugString(value),reference.module.componentsToDebugString(value)));
}
const identity=({module,...identity})=>identity;
const receipt={schema:1,generatedAt:new Date().toISOString(),scope:'offline deterministic package kernels; no browser-source collection, network, lifecycle or performance qualification',reference:identity(reference),candidate:identity(candidate),passed:results.every(r=>r.passed),checks:results};
await mkdir(dirname(output),{recursive:true});await writeFile(output,JSON.stringify(receipt,null,2)+'\n');
console.log(`${results.filter(r=>r.passed).length}/${results.length} offline package kernel comparisons pass`);
for(const r of results.filter(r=>!r.passed))console.error(r.name,r.message);
if(!receipt.passed)process.exitCode=1;
