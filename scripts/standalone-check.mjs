import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const root=resolve(dirname(fileURLToPath(import.meta.url)), '..');
const out=resolve(root,'build/standalone/hello.mjs');
const run=spawnSync(process.execPath,['scripts/mires.mjs','build','examples/standalone/hello.rs','--out',out,'--skip-build'],{cwd:root,stdio:'inherit'});
assert.equal(run.status,0);
const program=await import(pathToFileURL(out).href+'?verification='+Date.now());
assert.equal(program.sum_squares(0),0);
assert.equal(program.sum_squares(5),30);
assert.equal(program.sum_squares(100),328350);
assert.equal(program.greeting(),'Hello from Rust');
const receipt=JSON.parse(readFileSync(out+'.build.json'));
// JSC_PANIC=unwind: the receipt records the mode and `cleanup: 'off'` (rustc's unwind edges carry the
// cleanup there), so the default-cleanup assertion is skipped, with its reason printed.
if((process.env.JSC_PANIC||'abort')==='unwind'){assert.equal(receipt.panic,'unwind');console.log("standalone module: SKIP receipt.cleanup === 'on' (JSC_PANIC=unwind records cleanup 'off')");}
else assert.equal(receipt.cleanup,'on');
assert.equal(receipt.profile,'release-balanced');
const map=JSON.parse(readFileSync(out+'.map'));
assert.ok(map.sources.some(s=>s.endsWith('hello.rs')));
assert.ok(map.sourcesContent.some(s=>s?.includes('sum_squares')));
console.log('standalone module: values, default cleanup, manifest publication and authored source map passed');
