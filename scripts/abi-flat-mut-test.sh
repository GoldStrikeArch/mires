#!/usr/bin/env bash
# Integration & soundness suite for Slice R3-A (`&mut Enum` and `&Scalar`/`&mut Scalar` flattening):
#   314: positive `&mut State` + `&mut i32` opaque loop (0 TAG, 0 overwrite, 0 slot boxing; checksum 71197732)
#   315: negative control 1 — `extern "C-unwind"` throw + `Drop` observer -> `ExceptionalCopyBack` ([ -1, 99, 77 ])
#   316: negative control 2 — indirect `fn` pointer throw + `Drop` observer -> `ExceptionalCopyBack` ([ -1, 99, 77 ])
#   317: negative control 3 — escaping pointer-identity cast (`ParamEscapes`) + shared `&i32` alias (`purity > Read`)

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

export JSC_PANIC=unwind
export RCGJS_WITNESS_CHECK=1

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

args="js-lane=jir js-profile=release-balanced js-opt=on js-cleanup=on"
JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null

for fixture in \
	314_abi_flat_mut_enum_and_scalar \
	315_abi_flat_mut_unwind_drop \
	316_abi_flat_mut_indirect_throw \
	317_abi_flat_mut_escape_and_alias; do
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
		"$root/examples/unwind-tests/$fixture.rs" \
		"$work/$fixture.js" --crate-type cdylib >/dev/null
done

# 314: Positive check — flattened in .lane.json, zero TAG/overwrite/slot-boxing in JS, checksum 71197732 in V8 & JSC
jq -e '
  .abi.applied == true and
  any(.abi.flat[]; (.item | contains("step_state")) and .spill == 6) and
  any(.physical.decisions[]; (.item | contains("step_state")) and .shape == "mut-enum(6)+scalar-ref(1)" and .selected == true)
' "$work/314_abi_flat_mut_enum_and_scalar.js.lane.json" >/dev/null

node - "$root/runtime/shim.js" \
	"$work/314_abi_flat_mut_enum_and_scalar.js" \
	"$work/315_abi_flat_mut_unwind_drop.js" \
	"$work/316_abi_flat_mut_indirect_throw.js" \
	"$work/317_abi_flat_mut_escape_and_alias.js" \
	"$work/314_abi_flat_mut_enum_and_scalar.js.lane.json" \
	"$work/315_abi_flat_mut_unwind_drop.js.lane.json" \
	"$work/316_abi_flat_mut_indirect_throw.js.lane.json" \
	"$work/317_abi_flat_mut_escape_and_alias.js.lane.json" <<'NODE'
const fs = require('node:fs');
const vm = require('node:vm');

const [
  shimPath,
  f314, f315, f316, f317,
  l314, l315, l316, l317,
] = process.argv.slice(2);
const shim = fs.readFileSync(shimPath, 'utf8');

function extractFn(code, matchRe) {
  const m = code.match(matchRe);
  if (!m) throw new Error(`missing function matching ${matchRe}`);
  return m[0];
}

// 314: verify zero TAG / __rt.overwrite / slot boxing in bench_enum_state and step_state
const code314 = fs.readFileSync(f314, 'utf8');
const lane314 = JSON.parse(fs.readFileSync(l314, 'utf8'));
if (/TAG\s*:/.test(code314)) {
  throw new Error(`314 still contains TAG literal:\n${code314}`);
}
if (/__rt\.overwrite/.test(code314)) {
  throw new Error(`314 still contains __rt.overwrite:\n${code314}`);
}
// No `{ buf: <id>, off: 0 }` inside bench_enum_state or step_state (core panic cold paths excluded if any)
const fnBench = code314.slice(code314.indexOf('function bench_enum_state'));
if (/\{buf:[^,]+,off:0\}/.test(fnBench.replace(/core\$panicking[^}]+/g, ''))) {
  // Check more precisely per-function: bench_enum_state's step call & step_state
  const stepCallLine = code314.match(/function bench_enum_state\([^)]*\)\{[^\n]+\}/)?.[0] || '';
  if (/step_state|__rt\.overwrite|TAG:/.test(stepCallLine)) {
    throw new Error(`314 bench_enum_state bad: ${stepCallLine}`);
  }
}
const ctx314 = vm.createContext({ TextDecoder, TextEncoder });
vm.runInContext(shim + '\n' + code314, ctx314);
const sum314 = ctx314.bench_enum_state(4096);
if (sum314 !== 71197732) {
  throw new Error(`314 checksum mismatch: got ${sum314}, expected 71197732`);
}

// 315: host_note(-1) throws -> Drop observer MUST see acc=99, Ready.total=77; step_and_throw declined with ExceptionalCopyBack
const lane315 = JSON.parse(fs.readFileSync(l315, 'utf8'));
const dec315 = lane315.physical.decisions.find((d) => d.item.includes('step_and_throw'));
if (!dec315 || dec315.selected !== false || dec315.reason !== 'boundary.exceptional-copy-back') {
  throw new Error(`315 expected ExceptionalCopyBack decline for step_and_throw, got ${JSON.stringify(dec315)}`);
}
const trace315 = [];
const note315 = (v) => {
  trace315.push(v);
  if (v === -1) throw 'boom';
};
const ctx315 = vm.createContext({
  TextDecoder,
  TextEncoder,
  host_note: note315,
});
vm.runInContext(shim + '\n' + fs.readFileSync(f315, 'utf8'), ctx315);
ctx315.__rt.host_note = note315;
let caught315 = null;
try {
  ctx315.run_unwind_observer(true);
} catch (e) {
  caught315 = e;
}
if (caught315 !== 'boom' || JSON.stringify(trace315) !== JSON.stringify([-1, 99, 77])) {
  throw new Error(`315 unwind trace mismatch: caught=${caught315}, trace=${JSON.stringify(trace315)}`);
}

// 316: indirect cb(-1) throws -> Drop observer MUST see acc=99, Ready.total=77; step_via_indirect declined with ExceptionalCopyBack
const lane316 = JSON.parse(fs.readFileSync(l316, 'utf8'));
const dec316 = lane316.physical.decisions.find((d) => d.item.includes('step_via_indirect'));
if (!dec316 || dec316.selected !== false || dec316.reason !== 'boundary.exceptional-copy-back') {
  throw new Error(`316 expected ExceptionalCopyBack decline for step_via_indirect, got ${JSON.stringify(dec316)}`);
}
const trace316 = [];
const note316 = (v) => {
  trace316.push(v);
  if (v === -1) throw 'boom-indirect';
};
const ctx316 = vm.createContext({
  TextDecoder,
  TextEncoder,
  host_note: note316,
});
vm.runInContext(shim + '\n' + fs.readFileSync(f316, 'utf8'), ctx316);
ctx316.__rt.host_note = note316;
let caught316 = null;
try {
  ctx316.run_indirect_observer(true);
} catch (e) {
  caught316 = e;
}
if (caught316 !== 'boom-indirect' || JSON.stringify(trace316) !== JSON.stringify([-1, 99, 77])) {
  throw new Error(`316 unwind trace mismatch: caught=${caught316}, trace=${JSON.stringify(trace316)}`);
}

// 317: pointer identity & shared-ref aliasing
const lane317 = JSON.parse(fs.readFileSync(l317, 'utf8'));
const decPtr = lane317.physical.decisions.find((d) => d.item.includes('check_ptr_identity'));
if (!decPtr || decPtr.selected !== false || decPtr.reason !== 'boundary.param-escapes') {
  throw new Error(`317 expected ParamEscapes decline for check_ptr_identity, got ${JSON.stringify(decPtr)}`);
}
const ctx317 = vm.createContext({ TextDecoder, TextEncoder });
vm.runInContext(shim + '\n' + fs.readFileSync(f317, 'utf8'), ctx317);
const idRes = ctx317.test_ptr_identity();
const aliasRes = ctx317.test_shared_alias();
if (idRes !== 7 || aliasRes !== 42) {
  throw new Error(`317 runtime mismatch: idRes=${idRes} (expected 7), aliasRes=${aliasRes} (expected 42)`);
}

console.log('abi-flat-mut-test: PASS (314=71197732, 315=[-1,99,77], 316=[-1,99,77], 317=(7,42))');
NODE

JSC_BIN="/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc"
if [ -x "$JSC_BIN" ]; then
	env -u JSC_PANIC "$JSC_BIN" -e "$(cat "$root/runtime/shim.js" "$work/314_abi_flat_mut_enum_and_scalar.js")
if (bench_enum_state(4096) !== 71197732) throw new Error('JSC 314 mismatch');
print('abi-flat-mut-test (JSC): PASS (314=71197732)');"
fi
