import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
import {gzipSync,brotliCompressSync,constants} from 'node:zlib';
import {rollup} from '../../../experiments/fingerprintjs/node_modules/rollup/dist/es/rollup.js';
import {minify} from '../../../experiments/fingerprintjs/node_modules/terser/main.js';
import {parse} from './es2018-parser/node_modules/acorn/dist/acorn.mjs';
import {minimalShim} from './minimal-shim.mjs';
import {checkReady} from './promise-ready-check.mjs';

const root = resolve(process.argv[2]);
const runtime = await readFile(new URL('../runtime/shim.js',import.meta.url),'utf8');
const sha = value => createHash('sha256').update(value).digest('hex');
const rows = [];
for (const arm of ['jir-control','jir-on','jir-retry']) {
  const file = join(root,arm,'program.mjs');
  const program = await readFile(file,'utf8');
  const shim = minimalShim([program],runtime);
  assert.ok(shim.names.includes(arm === 'jir-on' ? 'host_future_ready' : 'host_future_promise'));
  assert.ok(!shim.names.includes(arm === 'jir-on' ? 'host_future_promise' : 'host_future_ready'));
  const bundle = await rollup({input:file,onwarn(warning) { throw new Error(warning.message); },plugins:[{
    name:'exact-ready-runtime',
    resolveId(source) { if (source === '../shim.mjs') return '\0ready-runtime'; },
    load(id) { if (id === '\0ready-runtime') return shim.source + `\nexport const {${shim.names.join(',')}}=globalThis.__rt;`; },
  }]});
  const {output} = await bundle.generate({format:'es'});
  await bundle.close();
  assert.equal(output.length,1);
  const generated = await minify(output[0].code,{ecma:2018,module:true,compress:{passes:3},mangle:true,format:{comments:false}});
  assert.ok(generated.code);
  parse(generated.code,{ecmaVersion:2018,sourceType:'module'});
  const directory = join(root,'packages');
  await mkdir(directory,{recursive:true});
  const packaged = join(directory,arm+'.mjs');
  await writeFile(packaged,generated.code);
  const result = await checkReady(await import(pathToFileURL(packaged)));
  rows.push({arm,sourceSha256:sha(program),runtimeSha256:sha(runtime),members:shim.names,
    packageSha256:sha(generated.code),minified:Buffer.byteLength(generated.code),
    gzip:gzipSync(generated.code,{level:9}).length,
    brotli:brotliCompressSync(generated.code,{params:{[constants.BROTLI_PARAM_QUALITY]:11}}).length,result});
}
assert.deepEqual(rows[0].result,rows[1].result);
assert.deepEqual(rows[0].result,rows[2].result);
assert.equal(rows[0].packageSha256,rows[2].packageSha256,'rollback must reproduce the complete minified control package');
assert.ok(rows[1].gzip < rows[0].gzip && rows[1].brotli < rows[0].brotli,'complete compressed package must improve');
const versions = {};
for (const name of ['rollup','terser']) versions[name] = JSON.parse(await readFile(new URL(`../../../experiments/fingerprintjs/node_modules/${name}/package.json`,import.meta.url))).version;
const receipt = {passed:true,versions,rows,delta:{minified:rows[1].minified-rows[0].minified,gzip:rows[1].gzip-rows[0].gzip,brotli:rows[1].brotli-rows[0].brotli},productionQualified:false};
await writeFile(join(root,'package.json'),JSON.stringify(receipt,null,2)+'\n');
console.log(JSON.stringify(receipt,null,2));
