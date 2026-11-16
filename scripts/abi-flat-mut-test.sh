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
orig_sysroot=$(readlink "$root/build/sysroot" 2>/dev/null || true)
cleanup() {
	if [ -n "$orig_sysroot" ]; then
		ln -sfn "$orig_sysroot" "$root/build/sysroot"
	fi
	rm -rf -- "$work"
}
trap cleanup EXIT

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

# Independent e2e fixtures for non-unit return values with &mut Enum + &mut i32 (`-> i32` and `-> u64`)
# and `Pin<&mut T>` rejection (`transparent = Some`, non-reference unpeeled `TypeId`).
cat > "$work/ret_i32.rs" << 'EOF'
#![no_std]
#![no_main]

enum State {
    Idle,
    Loading { progress: i32, retries: i32 },
    Ready { value: i32, flags: i32, checksum: i32 },
}

#[inline(never)]
fn step_state_ret(state: &mut State, i: i32, acc: &mut i32) -> i32 {
    match *state {
        State::Idle => *state = State::Loading { progress: i, retries: 1 },
        State::Loading { progress, retries } => {
            if (i & 3) == 3 {
                *state = State::Ready { value: progress.wrapping_add(i), flags: retries, checksum: i.wrapping_mul(31) };
            } else {
                *state = State::Loading { progress: progress.wrapping_add(i), retries: retries.wrapping_add(1) };
            }
        }
        State::Ready { value, flags, checksum } => {
            *acc = acc.wrapping_add(value).wrapping_add(flags).wrapping_add(checksum);
            *state = State::Idle;
        }
    }
    *acc ^ i
}

#[unsafe(no_mangle)]
pub fn run_ret_i32(n: i32) -> i32 {
    let mut state = State::Idle;
    let mut acc = 0;
    let mut last = 0;
    for i in 0..n {
        last = step_state_ret(&mut state, i, &mut acc);
    }
    acc.wrapping_add(last)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
EOF

cat > "$work/ret_u64.rs" << 'EOF'
#![no_std]
#![no_main]

enum State {
    Idle,
    Loading { progress: i32, retries: i32 },
    Ready { value: i32, flags: i32, checksum: i32 },
}

#[inline(never)]
fn step_state_ret_u64(state: &mut State, i: i32, acc: &mut i32) -> u64 {
    match *state {
        State::Idle => *state = State::Loading { progress: i, retries: 1 },
        State::Loading { progress, retries } => {
            if (i & 3) == 3 {
                *state = State::Ready { value: progress.wrapping_add(i), flags: retries, checksum: i.wrapping_mul(31) };
            } else {
                *state = State::Loading { progress: progress.wrapping_add(i), retries: retries.wrapping_add(1) };
            }
        }
        State::Ready { value, flags, checksum } => {
            *acc = acc.wrapping_add(value).wrapping_add(flags).wrapping_add(checksum);
            *state = State::Idle;
        }
    }
    ((*acc as u32 as u64) << 16) | (i as u32 as u64)
}

#[unsafe(no_mangle)]
pub fn run_ret_u64(n: i32) -> u32 {
    let mut state = State::Idle;
    let mut acc = 0;
    let mut mixed = 0u64;
    for i in 0..n {
        mixed ^= step_state_ret_u64(&mut state, i, &mut acc);
    }
    (mixed ^ (mixed >> 32)) as u32
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
EOF

cat > "$work/pin_guard.rs" << 'EOF'
#![no_std]
#![no_main]
use core::pin::Pin;

enum State {
    Idle,
    Active { v: i32 },
}

#[inline(never)]
fn step_pin(state: Pin<&mut State>, acc: Pin<&mut i32>, x: i32) -> i32 {
    let s = unsafe { state.get_unchecked_mut() };
    let a = unsafe { acc.get_unchecked_mut() };
    match *s {
        State::Idle => *s = State::Active { v: x },
        State::Active { v } => { *a += v; *s = State::Idle; }
    }
    *a
}

#[unsafe(no_mangle)]
pub fn run_pin() -> i32 {
    let mut s = State::Idle;
    let mut a = 5;
    let p_s = unsafe { Pin::new_unchecked(&mut s) };
    let _ = step_pin(p_s, unsafe { Pin::new_unchecked(&mut a) }, 10);
    let p_s2 = unsafe { Pin::new_unchecked(&mut s) };
    step_pin(p_s2, unsafe { Pin::new_unchecked(&mut a) }, 20)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
EOF

for extra in ret_i32 ret_u64 pin_guard; do
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
		"$work/$extra.rs" "$work/$extra.js" --crate-type cdylib >/dev/null
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
	"$work/317_abi_flat_mut_escape_and_alias.js.lane.json" \
	"$work/ret_i32.js" "$work/ret_i32.js.lane.json" \
	"$work/ret_u64.js" "$work/ret_u64.js.lane.json" \
	"$work/pin_guard.js" "$work/pin_guard.js.lane.json" <<'NODE'
const fs = require('node:fs');
const vm = require('node:vm');

const [
  shimPath,
  f314, f315, f316, f317,
  l314, l315, l316, l317,
  fRetI32, lRetI32,
  fRetU64, lRetU64,
  fPin, lPin,
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
// Independent JS reference for run_ret_i32(4096) and run_ret_u64(4096).
let refTag = 0, refP = 0, refR = 0, refV = 0, refF = 0, refC = 0, refAcc = 0;
let refLast = 0, refMixed = 0n;
for (let i = 0; i < 4096; i++) {
  if (refTag === 0) {
    refTag = 1; refP = i; refR = 1;
  } else if (refTag === 1) {
    if ((i & 3) === 3) {
      refTag = 2; refV = (refP + i) | 0; refF = refR; refC = Math.imul(i, 31) | 0;
    } else {
      refP = (refP + i) | 0; refR = (refR + 1) | 0;
    }
  } else {
    refAcc = (refAcc + refV + refF + refC) | 0;
    refTag = 0;
  }
  refLast = refAcc ^ i;
  const word = (BigInt(refAcc >>> 0) << 16n) | BigInt(i >>> 0);
  refMixed ^= word;
}
const expectedRet = (refAcc + refLast) | 0;
const expectedU64Mix = Number((refMixed ^ (refMixed >> 32n)) & 0xffff_ffffn) >>> 0;

const laneRetI32 = JSON.parse(fs.readFileSync(lRetI32, 'utf8'));
const decRetI32 = laneRetI32.physical.decisions.find((d) => d.item.includes('step_state_ret'));
if (!decRetI32 || decRetI32.shape !== 'mut-enum(6)+scalar-ref(1)' || !decRetI32.selected) {
  throw new Error(`ret_i32 expected step_state_ret selected, got ${JSON.stringify(decRetI32)}`);
}
const ctxRetI32 = vm.createContext({ TextDecoder, TextEncoder });
vm.runInContext(shim + '\n' + fs.readFileSync(fRetI32, 'utf8'), ctxRetI32);
const gotRet = ctxRetI32.run_ret_i32(4096);
if (gotRet !== expectedRet) {
  throw new Error(`ret_i32 mismatch: got ${gotRet}, expected ${expectedRet}`);
}

const laneRetU64 = JSON.parse(fs.readFileSync(lRetU64, 'utf8'));
const decRetU64 = laneRetU64.physical.decisions.find((d) => d.item.includes('step_state_ret_u64'));
if (!decRetU64 || !decRetU64.shape?.includes('mut-enum(6)+scalar-ref(1)') || !decRetU64.selected) {
  throw new Error(`ret_u64 expected step_state_ret_u64 selected, got ${JSON.stringify(decRetU64)}`);
}
const ctxRetU64 = vm.createContext({ TextDecoder, TextEncoder });
vm.runInContext(shim + '\n' + fs.readFileSync(fRetU64, 'utf8'), ctxRetU64);
const gotU64 = ctxRetU64.run_ret_u64(4096) >>> 0;
if (gotU64 !== expectedU64Mix) {
  throw new Error(`ret_u64 mismatch: got ${gotU64}, expected ${expectedU64Mix}`);
}

const lanePin = JSON.parse(fs.readFileSync(lPin, 'utf8'));
const decPin = lanePin.physical.decisions.find((d) => d.item.includes('step_pin'));
if (!decPin || decPin.shape !== null || decPin.selected !== false) {
  throw new Error(`pin_guard expected step_pin shape=null (not Pin<&mut T> flattened), got ${JSON.stringify(decPin)}`);
}
const ctxPin = vm.createContext({ TextDecoder, TextEncoder });
vm.runInContext(shim + '\n' + fs.readFileSync(fPin, 'utf8'), ctxPin);
if (ctxPin.run_pin() !== 15) {
  throw new Error(`pin_guard mismatch: got ${ctxPin.run_pin()}, expected 15`);
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
