#!/usr/bin/env bash
# Differentially checks 64-bit `(lo, hi)` pair lowering and `BigInt -> f32` casts against native
# Rust across 10,000+ cases in both `js-opt=off` and `js-opt=on` builds, on both Node V8 and
# standalone `jsc` (when available).
#
# Set SKIP_BUILD=1 to reuse an already-built compiler.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/native-tests/302_i64_pair_differential.rs"
prev_sysroot_link=""
if [ -L "$root/build/sysroot" ]; then
	prev_sysroot_link=$(readlink "$root/build/sysroot")
fi
work=$(mktemp -d)
cleanup() {
	rm -rf -- "$work"
	if [ -n "$prev_sysroot_link" ]; then
		local tmp="$root/build/.sysroot-restore.$$"
		mkdir -p "$root/build"
		ln -s "$prev_sysroot_link" "$tmp"
		mv -Tf "$tmp" "$root/build/sysroot" 2>/dev/null || {
			rm -f "$root/build/sysroot" "$tmp"
			ln -s "$prev_sysroot_link" "$root/build/sysroot"
		}
	fi
}
trap cleanup EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
	"$root/rust-toolchain.toml" | head -n 1)
: "${channel:=nightly}"

oracle_src="$work/oracle.rs"
cat >"$oracle_src" <<RUST
#[path = "$fixture"]
mod dut;

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn push_f32_midpoint_candidates(out: &mut Vec<u64>) {
    out.push(0);
    out.push(1);
    for exp in 24u32..64 {
        let base = 1u64 << exp;
        // Exact powers, +-1, and exact f32 midpoint (+ 2^(exp-25)) with +-2 around it
        let mid = 1u64 << (exp - 25);
        for d in [-2i64, -1, 0, 1, 2] {
            let v1 = base.wrapping_add_signed(d);
            out.push(v1);
            out.push(base.wrapping_add(mid).wrapping_add_signed(d));
            let odd_ulp = base.wrapping_add(1u64 << (exp - 24));
            out.push(odd_ulp.wrapping_add(mid).wrapping_add_signed(d));
        }
        // Also probe values near the f64 double-rounding boundary (exp >= 53)
        if exp >= 53 {
            let f64_half = 1u64 << (exp - 54);
            let odd_mid = base.wrapping_add(1u64 << (exp - 24)).wrapping_add(mid);
            out.push(odd_mid.wrapping_sub(1));
            out.push(odd_mid.wrapping_sub(f64_half.max(1)));
            out.push(odd_mid.wrapping_add(1));
            out.push(odd_mid.wrapping_add(f64_half.max(1)));
        }
    }
    // Classic 1,193-style sweep across powers of 2
    for shift in 24u32..64 {
        let pow = 1u64 << shift;
        let step = 1u64 << (shift - 24);
        for k in [0u64, 1, 2, 3, 7, 15] {
            let m = pow.wrapping_add(k.wrapping_mul(step)).wrapping_add(step >> 1);
            for d in [-2i64, -1, 0, 1, 2] {
                out.push(m.wrapping_add_signed(d));
            }
        }
    }
    out.push(i64::MAX as u64);
    out.push(i64::MIN as u64);
    out.push(u64::MAX);
    out.sort_unstable();
    out.dedup();
}

fn main() {
    let mut f32_vals = Vec::new();
    push_f32_midpoint_candidates(&mut f32_vals);
    for &x in &f32_vals {
        let sx = x as i64;
        println!("u64_to_f32_bits\t{x}\t{}", dut::u64_to_f32_bits(x));
        println!("i64_to_f32_bits\t{sx}\t{}", dut::i64_to_f32_bits(sx));
        println!("u64_to_f64_bits\t{x}\t{}", dut::u64_to_f64_bits(x));
        println!("i64_to_f64_bits\t{sx}\t{}", dut::i64_to_f64_bits(sx));
    }

    // 128-bit BigInt -> f32 midpoint and extreme boundaries
    let mut w128_vals: Vec<u128> = vec![
        0, 1, (1u128 << 53) - 1, 1u128 << 53, (1u128 << 53) + 1, (1u128 << 53) + 3,
        u64::MAX as u128, (u64::MAX as u128) + 1, i128::MAX as u128, (i128::MAX as u128) + 1,
        u128::MAX, u128::MAX - 1,
    ];
    for exp in 54u32..128 {
        let base = 1u128 << exp;
        let ulp = 1u128 << (exp - 24);
        let mid = 1u128 << (exp - 25);
        for d in [-1i128, 0, 1] {
            w128_vals.push(base.wrapping_add(mid).wrapping_add_signed(d));
            w128_vals.push(base.wrapping_add(ulp).wrapping_add(mid).wrapping_add_signed(d));
        }
    }
    w128_vals.sort_unstable();
    w128_vals.dedup();
    for &x in &w128_vals {
        let sx = x as i128;
        println!("u128_to_f32_bits\t{x}\t{}", dut::u128_to_f32_bits(x));
        println!("i128_to_f32_bits\t{sx}\t{}", dut::i128_to_f32_bits(sx));
    }

    // Saturating f64 / f32 -> i64 / u64 boundaries
    let f64_cases: Vec<f64> = vec![
        f64::NAN,
        f64::from_bits(0x7ff8_0000_0000_0001),
        f64::from_bits(0xfff8_0000_0000_0001),
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.0,
        -0.0,
        0.5,
        -0.5,
        0.999999,
        -0.999999,
        1.0,
        -1.0,
        12345.678,
        -12345.678,
        ((1u64 << 53) - 1) as f64,
        (1u64 << 53) as f64,
        ((1u64 << 53) + 2) as f64,
        -((1u64 << 53) as f64),
        9223372036854774784.0_f64,   // 2^63 - 1024 (largest f64 < i64::MAX)
        9223372036854775808.0_f64,   // 2^63
        -9223372036854775808.0_f64,  // -2^63
        -9223372036854777856.0_f64,  // -2^63 - 2048
        18446744073709549568.0_f64,  // 2^64 - 2048 (largest f64 < u64::MAX)
        18446744073709551616.0_f64,  // 2^64
        1.0e20,
        -1.0e20,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
    ];
    for f in f64_cases {
        let b = f.to_bits();
        println!("f64_bits_to_i64\t{b}\t{}", dut::f64_bits_to_i64(b));
        println!("f64_bits_to_u64\t{b}\t{}", dut::f64_bits_to_u64(b));
    }

    let f32_cases: Vec<f32> = vec![
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        0.0,
        -0.0,
        0.5,
        -0.5,
        1.0,
        -1.0,
        12345.678_f32,
        -12345.678_f32,
        9223371487098961920.0_f32,  // largest f32 < 2^63
        9223372036854775808.0_f32,  // 2^63
        -9223372036854775808.0_f32, // -2^63
        -9223373136366403584.0_f32, // next f32 < -2^63
        18446742974197923840.0_f32, // largest f32 < 2^64
        18446744073709551616.0_f32, // 2^64
        1.0e20_f32,
        -1.0e20_f32,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
    ];
    for f in f32_cases {
        let b = f.to_bits();
        println!("f32_bits_to_i64\t{b}\t{}", dut::f32_bits_to_i64(b));
        println!("f32_bits_to_u64\t{b}\t{}", dut::f32_bits_to_u64(b));
    }

    // 64-bit arithmetic, division/remainder, shifts/rotates, checked, bit intrinsics, cmp, bytes/struct
    let edge_vals: [u64; 18] = [
        0,
        1,
        2,
        3,
        7,
        0x7fff_ffff,
        0x8000_0000,
        0xffff_ffff,
        0x1_0000_0000,
        0x0123_4567_89ab_cdef,
        0x7fff_ffff_ffff_ffff,
        0x8000_0000_0000_0000,
        0x8000_0000_0000_0001,
        0xffff_ffff_0000_0000,
        0xffff_ffff_ffff_fffe,
        0xffff_ffff_ffff_ffff,
        0x9e37_79b9_7f4a_7c15,
        0xbf58_476d_1ce4_e5b9,
    ];
    let mut pairs: Vec<(u64, u64, u32)> = Vec::new();
    let shift_list = [0u32, 1, 7, 31, 32, 33, 63, 64, 65, 95, 127];
    for (i, &a) in edge_vals.iter().enumerate() {
        for (j, &b) in edge_vals.iter().enumerate() {
            let s = shift_list[(i + j) % shift_list.len()];
            pairs.push((a, b, s));
        }
    }
    let mut rng = 0x1234_5678_9abc_def0u64;
    for i in 0..320usize {
        let a = splitmix64(&mut rng);
        let b = if i % 5 == 0 {
            splitmix64(&mut rng) & 0xffff_ffff // 32-bit divisor/operand fast path
        } else {
            splitmix64(&mut rng)
        };
        let s = shift_list[i % shift_list.len()];
        pairs.push((a, b, s));
    }

    for (idx, &(a, b, s)) in pairs.iter().enumerate() {
        let sa = a as i64;
        let sb = b as i64;
        println!("u64_add\t{a},{b}\t{}", dut::u64_add(a, b));
        println!("u64_sub\t{a},{b}\t{}", dut::u64_sub(a, b));
        println!("u64_mul\t{a},{b}\t{}", dut::u64_mul(a, b));
        println!("u64_div\t{a},{b}\t{}", dut::u64_div(a, b));
        println!("u64_rem\t{a},{b}\t{}", dut::u64_rem(a, b));
        println!("i64_div\t{sa},{sb}\t{}", dut::i64_div(sa, sb));
        println!("i64_rem\t{sa},{sb}\t{}", dut::i64_rem(sa, sb));
        println!("u64_shl\t{a},{s}\t{}", dut::u64_shl(a, s));
        println!("u64_shr\t{a},{s}\t{}", dut::u64_shr(a, s));
        println!("i64_shr\t{sa},{s}\t{}", dut::i64_shr(sa, s));
        println!("u64_rol\t{a},{s}\t{}", dut::u64_rol(a, s));
        println!("u64_ror\t{a},{s}\t{}", dut::u64_ror(a, s));
        println!("checked_flags\t{a},{b}\t{}", dut::checked_flags(a, b));
        println!("checked_vals\t{a},{b}\t{}", dut::checked_vals(a, b));
        println!("cmp_mask\t{a},{b}\t{}", dut::cmp_mask(a, b));
        println!("bytes_struct_roundtrip\t{a},{sb}\t{}", dut::bytes_struct_roundtrip(a, sb));
        if idx < 200 {
            println!("u64_mul_consts\t{a}\t{}", dut::u64_mul_consts(a));
            println!("u64_const_shifts\t{a}\t{}", dut::u64_const_shifts(a));
            println!("i64_const_shr\t{sa}\t{}", dut::i64_const_shr(sa));
            println!("u64_bit_counts\t{a}\t{}", dut::u64_bit_counts(a));
            println!("u64_swap_bytes\t{a}\t{}", dut::u64_swap_bytes(a));
            println!("u64_reverse_bits\t{a}\t{}", dut::u64_reverse_bits(a));
        }
    }
}
RUST

rustc "+$channel" --edition=2024 -O --cfg oracle "$oracle_src" -o "$work/oracle"
"$work/oracle" >"$work/cases.tsv"

for mode in off on; do
	args="js-opt=$mode"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="$args js-lane=jir js-opt-warnings=off" \
		"$script_dir/compile_core.sh" "$fixture" "$work/$mode.js" >/dev/null
done

node_check="$work/check.mjs"
cat >"$node_check" <<'NODE'
import crypto from "node:crypto";
import fs from "node:fs";
import vm from "node:vm";

const [root, offPath, onPath, casesPath] = process.argv.slice(2);
const shim = fs.readFileSync(`${root}/runtime/shim.js`, "utf8");

function load(file) {
  const ctx = vm.createContext({ TextDecoder, TextEncoder });
  vm.runInContext(shim, ctx);
  vm.runInContext(fs.readFileSync(file, "utf8"), ctx);
  return ctx;
}

const offCtx = load(offPath);
const onCtx = load(onPath);

const BIG_OPS = new Set([
  "u64_to_f32_bits", "i64_to_f32_bits", "u64_to_f64_bits", "i64_to_f64_bits",
  "u128_to_f32_bits", "i128_to_f32_bits", "f64_bits_to_i64", "f64_bits_to_u64",
  "u64_add", "u64_sub", "u64_mul", "u64_mul_consts", "u64_div", "u64_rem",
  "i64_div", "i64_rem", "u64_shl", "u64_shr", "i64_shr", "u64_rol", "u64_ror",
  "u64_const_shifts", "i64_const_shr", "checked_flags", "checked_vals",
  "u64_bit_counts", "u64_swap_bytes", "u64_reverse_bits", "cmp_mask",
  "bytes_struct_roundtrip",
]);
const NUM_OPS = new Set(["f32_bits_to_i64", "f32_bits_to_u64"]);
const SHIFT_OPS = new Set(["u64_shl", "u64_shr", "i64_shr", "u64_rol", "u64_ror"]);

function parseArgs(op, raw) {
  const parts = raw.split(",");
  if (SHIFT_OPS.has(op)) return [BigInt(parts[0]), Number(parts[1])];
  if (BIG_OPS.has(op)) return parts.map((p) => BigInt(p));
  if (NUM_OPS.has(op)) return parts.map((p) => Number(p));
  throw new Error(`unknown op ${op}`);
}

const rawCases = fs.readFileSync(casesPath, "utf8").trimEnd().split("\n");
const hash = crypto.createHash("sha256");
const started = process.hrtime.bigint();
let count = 0;

for (const line of rawCases) {
  const [op, rawArgs, expected] = line.split("\t");
  const args = parseArgs(op, rawArgs);
  for (const [label, ctx] of [["js-opt=off", offCtx], ["js-opt=on", onCtx]]) {
    const actual = String(ctx[op](...args));
    if (actual !== expected) {
      throw new Error(
        `Node ${label} mismatch at case ${count + 1} (${op}(${rawArgs})): Rust ${expected}, JS ${actual}`,
      );
    }
  }
  count += 1;
  hash.update(line);
  hash.update("\n");
}

if (count < 10000) {
  throw new Error(`expected >= 10,000 differential cases, got ${count}`);
}
const elapsed = Number(process.hrtime.bigint() - started) / 1e9;
console.log(
  `native i64-pair differential (node): PASS (${count} cases, off+on, ${elapsed.toFixed(3)}s, sha256 ${hash.digest("hex")})`,
);
NODE

node "$node_check" "$root" "$work/off.js" "$work/on.js" "$work/cases.tsv"

jsc_bin="/System/Library/Frameworks/JavaScriptCore.framework/Versions/Current/Helpers/jsc"
if [ -x "$jsc_bin" ]; then
	for mode in off on; do
		jsc_check="$work/check-jsc-$mode.js"
		cat >"$jsc_check" <<JSC
const SHIFT_OPS = { u64_shl: 1, u64_shr: 1, i64_shr: 1, u64_rol: 1, u64_ror: 1 };
const NUM_OPS = { f32_bits_to_i64: 1, f32_bits_to_u64: 1 };
const lines = readFile("$work/cases.tsv").trimEnd().split("\n");
let count = 0;
for (let i = 0; i < lines.length; i += 1) {
  const parts = lines[i].split("\t");
  const op = parts[0], rawArgs = parts[1], expected = parts[2];
  const argParts = rawArgs.split(",");
  let a0, a1, actual;
  if (SHIFT_OPS[op]) {
    actual = String(globalThis[op](BigInt(argParts[0]), Number(argParts[1])));
  } else if (NUM_OPS[op]) {
    actual = String(globalThis[op](Number(argParts[0])));
  } else if (argParts.length === 1) {
    actual = String(globalThis[op](BigInt(argParts[0])));
  } else {
    actual = String(globalThis[op](BigInt(argParts[0]), BigInt(argParts[1])));
  }
  if (actual !== expected) {
    throw new Error("jsc js-opt=$mode mismatch at case " + (i + 1) + " (" + op + "(" + rawArgs + ")): Rust " + expected + ", JS " + actual);
  }
  count += 1;
}
print("native i64-pair differential (jsc js-opt=$mode): PASS (" + count + " cases)");
JSC
		"$jsc_bin" "$root/runtime/shim.js" "$work/$mode.js" "$jsc_check"
	done
fi
