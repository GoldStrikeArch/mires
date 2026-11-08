#!/usr/bin/env node
// Regression for JavaScript's 32-bit hash-seed coercion across language ports.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {resolve,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const [candidatePath,output]=process.argv.slice(2);
if(!candidatePath||!output)throw new Error('usage: check-seed-boundaries.mjs <candidate-esm> <receipt>');
async function load(p){const b=await readFile(resolve(root,p));return {namespace:await import(`data:text/javascript;base64,${b.toString('base64')}`),path:p,sha256:createHash('sha256').update(b).digest('hex')};}
const reference=await load('experiments/fingerprintjs/dist/fp.esm.js'),candidate=await load(candidatePath);
const seeds=[undefined,NaN,Infinity,-Infinity,-4294967297,-4294967296,-2147483649,-2147483648,-1,-0,0,1,1.5,-1.5,2147483647,2147483648,2147483649,4294967295,4294967296,4294967297,Number.MAX_SAFE_INTEGER];
let x=0x12345678;for(let i=0;i<32;i++){x^=x<<13;x^=x>>>17;x^=x<<5;seeds.push(x>>>0);}
const texts=['','a','1234567','12345678','a'.repeat(15),'b'.repeat(16),'c'.repeat(17),'α😀日本語','abc'.repeat(341)];
const checks=[];
for(const [textIndex,text] of texts.entries())for(const [seedIndex,seed] of seeds.entries()){
 const expected=reference.namespace.murmurX64Hash128(text,seed);let actual,error;
 try{actual=candidate.namespace.murmurX64Hash128(text,seed);}catch(e){error=String(e);}
 checks.push({textIndex,seedIndex,seed:String(seed),passed:actual===expected,...(actual!==expected?{expected,actual,error}:{})});
}
const identity=({namespace,...rest})=>rest;
const receipt={schema:1,generatedAt:new Date().toISOString(),scope:'hash-seed numerical boundary and deterministic random regression; not browser parity',reference:identity(reference),candidate:identity(candidate),passed:checks.every(x=>x.passed),checks};
await mkdir(dirname(resolve(root,output)),{recursive:true});await writeFile(resolve(root,output),JSON.stringify(receipt,null,2)+'\n');
console.log(`${checks.filter(x=>x.passed).length}/${checks.length} seed-boundary comparisons passed`);
for(const c of checks.filter(x=>!x.passed).slice(0,12))console.error(JSON.stringify(c));
if(!receipt.passed)process.exitCode=1;
