#!/usr/bin/env bash
# Verifies whole-program sound `RTerm::Invoke` pruning (`prune_invoke::run_checked`):
#   1. Name-perturbation equivalence (`ordinary_callee` vs `user_rust_begin_unwind` vs
#      `#[export_name = "rust_begin_unwind_alias"]`) calling `extern "C-unwind" { fn host_throw(); }`
#      across `js-opt=off` and `js-opt=on` with `RCGJS_WITNESS_CHECK=1`.
#   2. Foreign/raw-ptr/spoofed-crate collisions (`clone`, `copy(*const u8, *mut u8, usize)`,
#      and a `--crate-name core` dependency rlib) across `js-opt=off` and `js-opt=on`.
#   3. Negative witness / rollback gate (`RCGJS_WITNESS_CHECK=1 RCGJS_WITNESS_FAULT=prune-invoke`):
#      asserts `.witnesses.failed > 0` in `.lane.json`, unpruned `try` block preserved, and
#      runtime `[9, 1]` + thrown JS object identity still passes.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
work=$(mktemp -d)
orig_sysroot=""
if [ -L "$root/build/sysroot" ]; then
	orig_sysroot=$(readlink "$root/build/sysroot")
fi

cleanup() {
	rm -rf -- "$work"
	if [ -n "$orig_sysroot" ] && [ -d "$orig_sysroot" ]; then
		ln -sfn "$orig_sysroot" "$root/build/sysroot"
	fi
}
trap cleanup EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

cat >"$work/ordinary.rs" <<'EOF'
#![no_std]
#![no_main]

unsafe extern "C-unwind" {
    fn host_note(v: u32);
    fn host_throw();
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(1) }
    }
}

#[inline(never)]
#[no_mangle]
pub fn ordinary_callee() {
    unsafe { host_throw() }
}

#[no_mangle]
pub fn audit_entry() -> u32 {
    let _g = Guard;
    ordinary_callee();
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
EOF

cat >"$work/user_rust_begin_unwind.rs" <<'EOF'
#![no_std]
#![no_main]

unsafe extern "C-unwind" {
    fn host_note(v: u32);
    fn host_throw();
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(1) }
    }
}

#[inline(never)]
#[no_mangle]
pub fn user_rust_begin_unwind() {
    unsafe { host_throw() }
}

#[no_mangle]
pub fn audit_entry() -> u32 {
    let _g = Guard;
    user_rust_begin_unwind();
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
EOF

cat >"$work/export_name_alias.rs" <<'EOF'
#![no_std]
#![no_main]

unsafe extern "C-unwind" {
    fn host_note(v: u32);
    fn host_throw();
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(1) }
    }
}

#[inline(never)]
#[export_name = "rust_begin_unwind_alias"]
pub fn alias_callee() {
    unsafe { host_throw() }
}

#[no_mangle]
pub fn audit_entry() -> u32 {
    let _g = Guard;
    alias_callee();
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
EOF

cat >"$work/spoof_core_dep.rs" <<'EOF'
#![no_std]

unsafe extern "C-unwind" {
    fn host_throw();
}

#[inline(never)]
pub fn spoofed_core_throw() {
    unsafe { host_throw() }
}
EOF

cat >"$work/collisions.rs" <<'EOF'
#![no_std]
#![no_main]

extern crate spoof_core;

unsafe extern "C-unwind" {
    fn host_note(v: u32);
    fn clone();
    fn copy(src: *const u8, dst: *mut u8, n: usize);
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(1) }
    }
}

#[inline(never)]
unsafe fn call_clone() {
    unsafe { clone() }
}

#[inline(never)]
unsafe fn call_copy(src: *const u8, dst: *mut u8, n: usize) {
    unsafe { copy(src, dst, n) }
}

#[no_mangle]
pub fn test_clone() -> u32 {
    let _g = Guard;
    unsafe { call_clone() }
    3
}

#[no_mangle]
pub fn test_copy() -> u32 {
    let _g = Guard;
    let src = [1u8, 2, 3, 4];
    let mut dst = [0u8; 4];
    unsafe { call_copy(src.as_ptr(), dst.as_mut_ptr(), dst.len()) }
    3
}

#[no_mangle]
pub fn test_spoofed_core() -> u32 {
    let _g = Guard;
    spoof_core::spoofed_core_throw();
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
EOF

cat >"$work/rollback_probe.rs" <<'EOF'
#![no_std]
#![no_main]

unsafe extern "C-unwind" {
    fn host_note(v: u32);
    fn host_throw();
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(1) }
    }
}

#[inline(never)]
fn prunable_rust_leaf(x: u32) -> u32 {
    if x == 0xffff_fffe {
        panic!("impossible guard");
    }
    x.wrapping_add(7)
}

#[no_mangle]
pub fn audit_entry(x: u32) -> u32 {
    let _g = Guard;
    let v = prunable_rust_leaf(x);
    unsafe { host_throw() }
    v
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
EOF

cat >"$work/alloc_zero_try.rs" <<'EOF'
#![no_std]
#![no_main]
extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

unsafe extern "C-unwind" {
    fn host_note(v: u32);
}

struct Guard(u32);
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(self.0) }
    }
}

#[no_mangle]
pub fn build_boxes(n: u32) -> u32 {
    let _g = Guard(1);
    let mut v: Vec<Box<u32>> = Vec::new();
    let mut i = 0;
    while i < n {
        v.push(Box::new(i.wrapping_mul(3).wrapping_add(1)));
        i += 1;
    }
    let mut sum = 0u32;
    for b in v {
        sum = sum.wrapping_add(*b);
    }
    sum
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
EOF

export RCGJS_WITNESS_CHECK=1

for opt in off on; do
	export JS_EXTRA_ARGS="js-profile=release-balanced js-opt=$opt js-lane=jir"
	SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$opt.log" 2>&1

	for case_name in ordinary user_rust_begin_unwind export_name_alias; do
		out_js="$work/$case_name-$opt.js"
		"$script_dir/compile_core.sh" "$work/$case_name.rs" "$out_js" >"$out_js.log" 2>&1
	done

	mkdir -p "$work/deps-$opt"
	"$script_dir/compile_core.sh" "$work/spoof_core_dep.rs" "$work/deps-$opt/libcore.rlib" \
		--crate-type rlib --crate-name core >"$work/deps-$opt/dep.log" 2>&1
	"$script_dir/compile_core.sh" "$work/collisions.rs" "$work/collisions-$opt.js" \
		--extern "spoof_core=$work/deps-$opt/libcore.rlib" >"$work/collisions-$opt.log" 2>&1
done

# Negative witness / rollback gate under js-opt=on
export JS_EXTRA_ARGS="js-profile=release-balanced js-opt=on js-lane=jir"
SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-rollback.log" 2>&1
"$script_dir/compile_core.sh" "$work/rollback_probe.rs" "$work/rollback-clean.js" >"$work/rollback-clean.log" 2>&1
RCGJS_WITNESS_FAULT=prune-invoke \
	"$script_dir/compile_core.sh" "$work/rollback_probe.rs" "$work/rollback-fault.js" >"$work/rollback-fault.log" 2>&1
"$script_dir/compile_core.sh" "$work/alloc_zero_try.rs" "$work/alloc-zero-try.js" >"$work/alloc-zero-try.log" 2>&1

node --input-type=module - "$work" "$root/runtime/shim.js" <<'NODE'
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import vm from 'node:vm';

const [work, shimPath] = process.argv.slice(2);
const shimCode = readFileSync(shimPath, 'utf8');

function runEntry(jsPath, entryName, extraArgs = [], hookNames = ['host_throw']) {
  const progCode = readFileSync(jsPath, 'utf8');
  const results = {};
  for (const throwing of [false, true]) {
    const trace = [];
    const token = { pruneInvokeTest: `${entryName}-${throwing}` };
    const ctx = vm.createContext({ TextEncoder, TextDecoder, console });
    vm.runInContext(shimCode, ctx);
    ctx.__rt.host_note = (v) => trace.push(Number(v));
    for (const hook of hookNames) {
      ctx.__rt[hook] = () => {
        trace.push(9);
        if (throwing) throw token;
      };
    }
    vm.runInContext(progCode, ctx);
    let ret;
    let caught;
    let threw = false;
    try {
      ret = ctx[entryName](...extraArgs);
    } catch (e) {
      threw = true;
      caught = e;
    }
    if (throwing) {
      assert.equal(threw, true, `${jsPath} ${entryName} should throw`);
      assert.equal(caught === token, true, `${jsPath} ${entryName} thrown object identity mismatch`);
      assert.deepEqual(trace, [9, 1], `${jsPath} ${entryName} throwing trace`);
    } else {
      assert.equal(threw, false, `${jsPath} ${entryName} should not throw`);
      assert.deepEqual(trace, [9, 1], `${jsPath} ${entryName} non-throwing trace`);
    }
    results[throwing ? 'throw' : 'normal'] = { trace, ret, same: throwing ? caught === token : null };
  }
  return results;
}

function readLane(jsPath) {
  return JSON.parse(readFileSync(`${jsPath}.lane.json`, 'utf8'));
}

// 1. Name-perturbation equivalence across js-opt=off and js-opt=on
for (const opt of ['off', 'on']) {
  const outcomes = [];
  for (const name of ['ordinary', 'user_rust_begin_unwind', 'export_name_alias']) {
    const jsPath = join(work, `${name}-${opt}.js`);
    const lane = readLane(jsPath);
    assert.equal(lane.witnesses?.failed ?? 0, 0, `${name}-${opt} witnesses.failed must be 0`);
    const res = runEntry(jsPath, 'audit_entry');
    assert.equal(res.normal.ret, 3);
    assert.equal(res.throw.same, true);
    outcomes.push(res);
  }
  assert.deepEqual(outcomes[1], outcomes[0], `user_rust_begin_unwind != ordinary (opt=${opt})`);
  assert.deepEqual(outcomes[2], outcomes[0], `export_name_alias != ordinary (opt=${opt})`);
}

// 2. Foreign / raw-ptr / spoofed-crate collisions across js-opt=off and js-opt=on
for (const opt of ['off', 'on']) {
  const jsPath = join(work, `collisions-${opt}.js`);
  const lane = readLane(jsPath);
  assert.equal(lane.witnesses?.failed ?? 0, 0, `collisions-${opt} witnesses.failed must be 0`);
  for (const entry of ['test_clone', 'test_copy', 'test_spoofed_core']) {
    const res = runEntry(jsPath, entry, [], ['clone', 'copy', 'host_throw']);
    assert.equal(res.normal.ret, 3);
    assert.equal(res.throw.same, true);
  }
}

// 3. Negative witness / rollback gate
const cleanJs = join(work, 'rollback-clean.js');
const faultJs = join(work, 'rollback-fault.js');
const cleanLane = readLane(cleanJs);
const faultLane = readLane(faultJs);
assert.equal(cleanLane.witnesses?.failed ?? 0, 0, 'clean rollback_probe witnesses.failed must be 0');
assert.ok((faultLane.witnesses?.failed ?? 0) > 0, `faulted rollback_probe witnesses.failed must be > 0, got ${JSON.stringify(faultLane.witnesses)}`);

const countTry = (p) => (readFileSync(p, 'utf8').match(/\btry\s*\{/g) || []).length;
const cleanTries = countTry(cleanJs);
const faultTries = countTry(faultJs);
assert.ok(faultTries > cleanTries, `expected faulted try count (${faultTries}) > clean try count (${cleanTries})`);

const cleanRes = runEntry(cleanJs, 'audit_entry', [5]);
const faultRes = runEntry(faultJs, 'audit_entry', [5]);
assert.equal(cleanRes.normal.ret, 12);
assert.equal(faultRes.normal.ret, 12);
assert.equal(cleanRes.throw.same, true);
assert.equal(faultRes.throw.same, true);

// 4. Default-allocator Vec<Box<u32>> + Guard under js-opt=on emits zero `try {` blocks
const allocJs = join(work, 'alloc-zero-try.js');
const allocLane = readLane(allocJs);
assert.equal(allocLane.witnesses?.failed ?? 0, 0, 'alloc-zero-try witnesses.failed must be 0');
const allocTries = countTry(allocJs);
assert.equal(allocTries, 0, `expected 0 try blocks in default-allocator Vec<Box<u32>> + Guard, got ${allocTries}`);
{
  const trace = [];
  const ctx = vm.createContext({ TextEncoder, TextDecoder, console });
  vm.runInContext(shimCode, ctx);
  ctx.__rt.host_note = (v) => trace.push(Number(v));
  vm.runInContext(readFileSync(allocJs, 'utf8'), ctx);
  const sum = ctx.build_boxes(5);
  assert.equal(sum, 1 + 4 + 7 + 10 + 13);
  assert.deepEqual(trace, [1]);
}

console.log(`prune-invoke-test: PASS (clean tries=${cleanTries}, fault tries=${faultTries}, fault witnesses.failed=${faultLane.witnesses.failed}, allocTries=${allocTries})`);
NODE
