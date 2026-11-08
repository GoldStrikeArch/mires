from pathlib import Path
import hashlib,json,os,subprocess,sys
root=Path(__file__).resolve().parent.parent;w=Path(sys.argv[1]).resolve();w.mkdir();src=w/'source';src.mkdir();provider=src/'provider';provider.mkdir();cache=w/'cache'
opts='js-opt=on js-minify=on js-ecma-target=2018';env={**os.environ,'JS_EXTRA_ARGS':opts,'SKIP_BUILD':'1','JSC_REMAP_ROOT':str(src)};rows=[]
def run(name,cmd,extra=None):
 with (w/(name+'.log')).open('w') as log:p=subprocess.run(cmd,cwd=root,env={**env,**(extra or {})},stdout=log,stderr=subprocess.STDOUT)
 rows.append(dict(name=name,command=cmd,exitCode=p.returncode));(w/'commands.json').write_text(json.dumps(rows,indent=2)+'\n');assert p.returncode==0,name
run('sysroot',['bash',str(root/'scripts/build_sysroot.sh')])
run('shim',['node',str(root/'scripts/make-esm-shim.mjs'),str(w/'shim.mjs')])
sysroot=subprocess.check_output(['bash','-c','source "$1/scripts/sysroot-cache.sh"; jsc_resolve_sysroot "$1" wasm32-unknown-unknown "$1/target/release/librustc_codegen_mires.dylib"','--',str(root)],env=env,text=True).strip()
(src/'main.rs').write_text('#![no_std]\n#![no_main]\n#[panic_handler]\nfn panic(_: &core::panic::PanicInfo)->! {loop{}}\n#[unsafe(no_mangle)]\npub extern "C" fn calculate(x:u32)->u32 { provider::run(x) }\n')
(provider/'lib.rs').write_text('#![no_std]\nmod a;\nmod b;\n#[inline(never)]\npub fn run(x:u32)->u32 { a::first(x).wrapping_add(b::second(x)) }\n')
(provider/'a.rs').write_text('#[inline(never)]\npub fn first(x:u32)->u32 { x.wrapping_add(3) }\n')
(provider/'b.rs').write_text('#[inline(never)]\npub fn second(x:u32)->u32 { x.wrapping_add(7) }\n')
checks=[]
generic='#![no_std]\n#[inline(never)]\npub fn run<T: Copy + core::ops::Add<Output=T>>(x:T)->T { x + x }\n'
(src/'generic.rs').write_text(generic)
run('generic-provider',['bash',str(root/'scripts/compile_core.sh'),str(src/'generic.rs'),str(w/'libgeneric.rlib'),'--crate-name','provider','--crate-type','rlib'])
(src/'generic.rs').rename(w/'removed-generic.rs')
run('generic-consumer',['bash',str(root/'scripts/compile_core.sh'),str(src/'main.rs'),str(w/'generic.mjs'),'--crate-name','source_generic','--extern',f'provider={w/"libgeneric.rlib"}'],{'JS_EXTRA_ARGS':opts+' js-lane=jir js-modules=esm js-source-map=on js-shim-module=./shim.mjs'})
generic_map=json.loads((w/'generic.mjs.map').read_text());assert dict(zip(generic_map['sources'],generic_map['sourcesContent'])).get('generic.rs')==generic
run('generic-execute',['node','--input-type=module','-e',f'import {{calculate}} from {json.dumps((w/"generic.mjs").as_uri())};import assert from "node:assert/strict";for(const x of [0,1,17,4294967295]) assert.equal(calculate(x),(2*x)>>>0);'])
checks.append(dict(stage='generic-only',passed=True,nodeChecks=4))
for stage in ['cold','warm','edit','comment']:
 if stage=='comment':(provider/'b.rs').write_text((provider/'b.rs').read_text()+'// source-only change\n')
 if stage=='edit':(provider/'b.rs').write_text((provider/'b.rs').read_text().replace('add(7)','add(9)'))
 expected={f'provider/{p.name}':p.read_text() for p in provider.glob('*.rs')};(w/f'{stage}-expected.json').write_text(json.dumps(expected,indent=2)+'\n')
 out=w/f'libprovider-{stage}.rlib';cmd=['rustc','+nightly-2026-07-27','-Zcodegen-backend='+str(root/'target/release/librustc_codegen_mires.dylib'),'--edition=2021','--target=wasm32-unknown-unknown','--sysroot',sysroot,'--crate-type=rlib','--crate-name=provider','-Ccodegen-units=2','-Cincremental='+str(cache),'-Cpanic=abort','-Coverflow-checks=off','-Cdebug-assertions=off','-Cdebuginfo=0','--remap-path-prefix',str(root)+'/=','--remap-path-prefix',str(src)+'/=','-Cllvm-args='+opts,'-o',str(out),str(provider/'lib.rs')]
 cmd += ['-Cllvm-args=js-producer=' + hashlib.sha256((root/'target/release/librustc_codegen_mires.dylib').read_bytes()).hexdigest()]
 run(stage+'-provider',cmd)
 parked=w/f'{stage}-removed';provider.rename(parked)
 try:
  program=w/f'{stage}.mjs';run(stage+'-consumer',['bash',str(root/'scripts/compile_core.sh'),str(src/'main.rs'),str(program),'--crate-name','source_incremental','--extern',f'provider={out}'],{'JS_EXTRA_ARGS':opts+' js-lane=jir js-modules=esm js-source-map=on js-shim-module=./shim.mjs'})
  m=json.loads(program.with_suffix('.mjs.map').read_text());actual=dict(zip(m['sources'],m['sourcesContent']));assert all(actual.get(k)==v for k,v in expected.items()),(stage,{k:actual.get(k) for k in expected})
  run(stage+'-execute',['node','--input-type=module','-e',f'import {{calculate}} from {json.dumps(program.as_uri())};import assert from "node:assert/strict";for (const x of [0,1,17,4294967295]) assert.equal(calculate(x), (2*x+{12 if stage in ["edit","comment"] else 10})>>>0);'])
  checks.append(dict(stage=stage,passed=True,sources=len(m['sources']),authoredSources=len(expected),nodeChecks=4))
 finally:parked.rename(provider)
(w/'receipt.json').write_text(json.dumps(dict(passed=True,checks=checks),indent=2)+'\n');print('generic, cold, warm, edit and comment-only removed-source maps pass',flush=True)
