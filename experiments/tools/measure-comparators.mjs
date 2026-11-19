#!/usr/bin/env node
// Deterministic transfer-size inventory, not a timing or full-parity benchmark.
import {readFile,readdir,writeFile,mkdir} from 'node:fs/promises';
import {existsSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {gzipSync,brotliCompressSync,constants} from 'node:zlib';
import {resolve,dirname,relative} from 'node:path';
import {fileURLToPath} from 'node:url';
import {parse} from 'acorn';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const languages=['rescript','melange','jsoo','scalajs','dart','kotlin','elm','gopherjs'];
const production={rescript:'default',melange:'default',jsoo:'default',scalajs:'full',dart:'o2',kotlin:'production',elm:'production',gopherjs:'default'};
const rows=[];
async function resource(file){const b=await readFile(file);return {path:relative(root,file),bytes:b.length,gzip9:gzipSync(b,{level:9,mtime:0}).length,brotli11:brotliCompressSync(b,{params:{[constants.BROTLI_PARAM_QUALITY]:11}}).length,sha256:createHash('sha256').update(b).digest('hex')};}
for(const suite of ['oss','pro'])for(const language of languages){
 const build=JSON.parse(await readFile(resolve(root,`build/comparators/${suite}-${language}.build.json`),'utf8'));
 if(!build.built)throw new Error(`not built: ${suite}-${language}`);
 const port=resolve(root,suite==='oss'?'experiments':'experiments-pro',`${suite==='oss'?'fingerprintjs':'fp-pro'}-${language}`);
 const dist=resolve(port,'dist');
 const variants=[['default',dist],...(await readdir(dist,{withFileTypes:true})).filter(x=>x.isDirectory()&&existsSync(resolve(dist,x.name,'fp.esm.min.js'))).map(x=>[x.name,resolve(dist,x.name)])];
 for(const [variant,dir] of variants){
  const main=await resource(resolve(dir,'fp.esm.min.js'));
  if(!build.artifacts.some(a=>a.path===main.path&&a.sha256===main.sha256))throw new Error(`stale artifact: ${main.path}`);
  const resources=[main];let worker=null;
  if(suite==='pro'){
   const file=resolve(dir,'fp.worker.min.js');worker=await resource(file);
   if(!build.artifacts.some(a=>a.path===worker.path&&a.sha256===worker.sha256))throw new Error(`stale worker: ${worker.path}`);
   const ast=parse(await readFile(file,'utf8'),{sourceType:'module',ecmaVersion:'latest'});
   worker.imports=ast.body.filter(x=>x.type==='ImportDeclaration').map(x=>x.source.value);
   if(worker.imports.some(x=>x!=='./fp.esm.min.js'))throw new Error(`unaccounted worker import: ${worker.imports}`);
   resources.push(worker);
  }
  rows.push({suite,language,variant,productionSelection:suite==='oss'?production[language]===variant:null,implementation:language==='elm'&&suite==='oss'?'Elm + substantial JavaScript host':'historical language port; full ownership not requalified',main,worker,deployment:{resources:resources.map(x=>x.path),bytes:resources.reduce((s,x)=>s+x.bytes,0),gzip9:resources.reduce((s,x)=>s+x.gzip9,0),brotli11:resources.reduce((s,x)=>s+x.brotli11,0)},scope:'static package sizes; worker imports counted once; no browser/time qualification'});
 }
}
const output=resolve(root,process.argv[2]||'build/comparators/sizes.json');
const receipt={schema:1,generatedAt:new Date().toISOString(),node:process.version,zlib:process.versions.zlib,brotli:process.versions.brotli,compression:'each resource compressed separately with Node zlib; not interchangeable with historical system-gzip measurements',rows,qualifiedLanguageRanking:false};
await mkdir(dirname(output),{recursive:true});await writeFile(output,JSON.stringify(receipt,null,2)+'\n');
console.log(`${rows.length} artifact/profile rows -> ${relative(root,output)}`);
