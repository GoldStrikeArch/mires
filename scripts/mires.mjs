#!/usr/bin/env node
// Standalone entry point: compile real-core Rust and publish its declared runtime closure.
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, basename, resolve, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { gzipSync, brotliCompressSync, constants } from 'node:zlib';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const help = `Usage: node scripts/mires.mjs build source.rs --out build/app.mjs
  --profile dev|report|release-size|release-balanced|release-speed
  --cleanup on|off       Default: off (in default unwind mode, where rustc's unwind
                         edges keep cleanup and --cleanup on is refused). Under
                         JSC_PANIC=abort, defaults to on (off omits cleanup).
  --extern name=path     Repeat for dependencies compiled with matching settings.
  --search directory    Repeat for transitive dependencies.
  --skip-build          Reuse the existing backend/linker; still resolve the sysroot.

JSC_PANIC defaults to unwind (targets/mires32-unwind.json, scripts/sysroot-cache.sh);
set JSC_PANIC=abort for -Cpanic=abort. JSC_RUST_MIR_INLINE defaults to off for client
crates (set JSC_RUST_MIR_INLINE=on to enable). The receipt records the panic mode.

Input is a no_std library with exported functions and a panic handler. Output is
an ES module, a reached-only runtime module, source map and artifact manifest.
The assembled compiler is a recovery baseline, not the complete October release.
`;
function run(cmd, args, env) {
  const r=spawnSync(cmd,args,{cwd:root,env,stdio:'inherit'});
  if (r.error) throw r.error;
  if (r.status !== 0) throw new Error(`${cmd} failed (${r.signal || r.status})`);
}
function hash(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
try {
  const args=process.argv.slice(2);
  if (!args.length || args.includes('--help') || args[0]==='help') {console.log(help);process.exit(0);}
  if (args.shift() !== 'build') throw new Error('expected build; use --help');
  const src=args.shift(); if (!src || src.startsWith('-')) throw new Error('missing Rust source');
  const panic=process.env.JSC_PANIC||'unwind';
  if(!['abort','unwind'].includes(panic))throw new Error('JSC_PANIC must be unset, abort or unwind');
  const mirInline=process.env.JSC_RUST_MIR_INLINE||'off';
  if(!['off','on'].includes(mirInline))throw new Error('JSC_RUST_MIR_INLINE must be unset, off or on');
  let output,profile='release-balanced',cleanup=panic==='unwind'?'off':'on',skip=false;const extra=[];
  while(args.length) {
    const k=args.shift();
    if(k==='--skip-build'){skip=true;continue;}
    if(!['--out','--profile','--cleanup','--extern','--search'].includes(k))throw new Error(`unknown option ${k}`);
    const v=args.shift();if(!v || v.startsWith('--'))throw new Error(`${k} needs a value`);
    if(k==='--out')output=v;else if(k==='--profile')profile=v;else if(k==='--cleanup')cleanup=v;
    else if(k==='--extern') {const pos=v.indexOf('=');if(pos<1)throw new Error('--extern expects name=path');extra.push(k,v.slice(0,pos+1)+resolve(v.slice(pos+1)));}
    else extra.push(k,resolve(v));
  }
  if(!['dev','report','release-size','release-balanced','release-speed'].includes(profile))throw new Error('invalid profile');
  if(!['on','off'].includes(cleanup))throw new Error('invalid cleanup setting');
  if(panic==='unwind'&&cleanup==='on')throw new Error('JSC_PANIC=unwind keeps cleanup through rustc unwind edges; --cleanup on is the abort-mode lowering');
  const input=resolve(src),out=resolve(output||`build/${basename(src,extname(src))}.mjs`);
  if(!existsSync(input))throw new Error(`source missing: ${input}`);
  if(!out.endsWith('.mjs') || !/^[A-Za-z0-9._-]+$/.test(basename(out)))throw new Error('output must have a simple .mjs filename');
  mkdirSync(dirname(out),{recursive:true});
  const runtimeName=basename(out,'.mjs')+'.runtime.mjs';
  const emit=`js-profile=${profile} js-cleanup=${cleanup}`;
  const env={...process.env,JS_EXTRA_ARGS:emit,JSC_SYSROOT_ARGS:emit,JSC_REMAP_ROOT:dirname(input),SKIP_BUILD:'1',JSC_PANIC:panic,JSC_RUST_MIR_INLINE:mirInline};
  delete env.JSC_RUST_OPT_LEVEL;
  if(!skip)run('bash',['scripts/build.sh','--locked'],env);
  run('bash',['scripts/build_sysroot.sh'],env);
  run('bash',['scripts/compile_core.sh',input,out,...extra],{...env,JS_EXTRA_ARGS:`${emit} js-lane=jir js-modules=esm js-shim-module=./${runtimeName} js-source-map=on js-opt-warnings=off`});
  const manifest=out+'.artifacts.json';
  run(process.execPath,['scripts/minimal-shim.mjs','--module','--manifest',manifest,'runtime/shim.js',resolve(dirname(out),runtimeName)],env);
  run(resolve(root,'target/release/jsc-link'),['--verify-artifacts',manifest],env);
  const binaryName=process.platform==='darwin'?'librustc_codegen_mires.dylib':'librustc_codegen_mires.so';
  const files=[out,resolve(dirname(out),runtimeName)];
  const receipt={schema:1,kind:'mires-standalone-build',generatedAt:new Date().toISOString(),source:{path:input,sha256:hash(readFileSync(input))},profile,cleanup,panic,backend:hash(readFileSync(resolve(root,'target/release',binaryName))),linker:hash(readFileSync(resolve(root,'target/release/jsc-link'))),node:process.version,artifacts:files.map(path=>{const b=readFileSync(path);return{path:basename(path),bytes:b.length,sha256:hash(b),gzip:gzipSync(b,{level:9}).length,brotli:brotliCompressSync(b,{params:{[constants.BROTLI_PARAM_QUALITY]:11}}).length};}),manifest:basename(manifest),qualification:'build-and-manifest-only'};
  writeFileSync(out+'.build.json',JSON.stringify(receipt,null,2)+'\n');
  console.log(`Built ${out}`);
} catch(e) { console.error(`mires: ${e.message}`);process.exitCode=1; }
