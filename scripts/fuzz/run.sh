#!/usr/bin/env bash
#
# The wasm32-LLVM-vs-JS differential fuzzer.
#
#   scripts/fuzz/run.sh 200                 # seeds 0..199
#   scripts/fuzz/run.sh 20 --seed 1000      # seeds 1000..1019
#   scripts/fuzz/run.sh 10 --family c       # only the destructor family
#   scripts/fuzz/run.sh 5 --keep            # keep the working directory of every seed
#   scripts/fuzz/run.sh 50 --lane jir --opt on   # the JIR lane with the optimizer on
#
# For each seed, scripts/fuzz/gen.mjs writes one defined-behaviour `#![no_std]` program, and that
# ONE source is compiled twice:
#
#   * by the ordinary LLVM backend for `wasm32-unknown-unknown` and run under Node's
#     `WebAssembly.instantiate` with scripts/fuzz/shim.mjs supplying the print imports;
#   * by rustc_codegen_js through scripts/compile_core.sh, and run the way scripts/test.sh runs a
#     fixture — runtime/shim.js, the program, scripts/entry.js, concatenated into node.
#
# The two runs must print the same stdout and end the same way. That is the only claim being
# checked, and it is the strongest one available: LLVM is the reference implementation of what the
# program means, and the generator only emits programs that HAVE one meaning (see gen.mjs).
#
# An exit STATUS is compared as a class rather than a number. A program that panics ends in
# `js_abort`, which the JS shim raises as an exception (node exits 1) and the wasm shim reports as
# exit 101; both are "aborted", and which number arrives says nothing about the compilers.
#
# A mismatch keeps `build/fuzz/<seed>/` — the source, both compilations, both outputs and the diff
# — and then SHRINKS it: one statement line is dropped at a time, and a drop is kept when the
# program still compiles on both sides and still disagrees. What is left is the smallest line-wise
# repro of the divergence, written to `build/fuzz/<seed>/shrunk.rs`.
#
# `--lane baseline|jir` and `--opt off|report|on` choose which lane the link step prints from and
# whether the optimizer runs, spelled the way `scripts/test.sh` spells them (`js-lane=`, `js-opt=`).
# `js-opt` is emit-affecting, so each `--opt` value has its own keyed sysroot: the flag selects it,
# builds it if the cache does not hold it yet (cached thereafter), and the run banner prints the
# key so which sysroot answered is part of the output rather than a guess. An invalid value is
# refused outright; there is no falling back to the default sysroot. The flags compose onto
# $JS_EXTRA_ARGS (last value wins), which is also still honored on its own — but unlike the
# pre-Phase-5 script, whatever emit-affecting args are in effect are the ones the sysroot is
# keyed by, never silently the default one.
#
# The JS side needs the matching sysroot (`scripts/build_sysroot.sh`) and the LLVM side needs the
# `wasm32-unknown-unknown` target for the pinned nightly; both are checked, and the target is
# installed if it is missing.

set -uo pipefail

fuzz_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
script_dir=$(cd -- "$fuzz_dir/.." && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=../sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"

count=${1:-}
if [ -z "$count" ] || ! [ "$count" -ge 1 ] 2>/dev/null; then
	echo "usage: run.sh <count> [--seed <first>] [--family a|b|c] [--keep] [--lane baseline|jir] [--opt off|report|on]" >&2
	exit 2
fi
shift

first_seed=0
family_args=()
keep=0
lane=""
opt=""
while [ "$#" -gt 0 ]; do
	case "$1" in
	--seed)
		first_seed=${2:-0}
		shift 2
		;;
	--family)
		family_args=(--family "${2:-}")
		shift 2
		;;
	--keep)
		keep=1
		shift
		;;
	--lane)
		lane=${2:-}
		case "$lane" in
		baseline | jir) ;;
		*)
			echo "run.sh: --lane must be baseline or jir, got: $lane" >&2
			exit 2
			;;
		esac
		shift 2
		;;
	--opt)
		opt=${2:-}
		case "$opt" in
		off | report | on) ;;
		*)
			echo "run.sh: --opt must be off, report or on, got: $opt" >&2
			exit 2
			;;
		esac
		shift 2
		;;
	*)
		echo "run.sh: unknown option: $1" >&2
		exit 2
		;;
	esac
done

# The flags compose onto whatever $JS_EXTRA_ARGS already says, last value winning, which is the
# backend's own parse rule. compile_core.sh reads the composed value from the environment.
if [ -n "$lane" ]; then
	JS_EXTRA_ARGS="${JS_EXTRA_ARGS:+$JS_EXTRA_ARGS }js-lane=$lane"
fi
if [ -n "$opt" ]; then
	JS_EXTRA_ARGS="${JS_EXTRA_ARGS:+$JS_EXTRA_ARGS }js-opt=$opt"
fi
export JS_EXTRA_ARGS

work_root="$root/build/fuzz"
mkdir -p "$work_root"
shim="$root/runtime/shim.js"
entry="$script_dir/entry.js"
channel=$(jsc_pinned_channel "$root")

case "$(uname -s)" in
Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
esac
if [ ! -f "$dylib" ]; then
	echo "run.sh: backend not built: $dylib (run scripts/build.sh)" >&2
	exit 1
fi

# The sysroot the composed args key — never the default one pinned regardless of them, which is
# what this script did before Phase 5 and what made emit-affecting extra args a trap.
sysroot_args=$(jsc_sysroot_args)
sysroot=$(jsc_sysroot_path "$root" wasm32-unknown-unknown "$dylib") || exit 1
if [ ! -d "$sysroot/lib/rustlib/wasm32-unknown-unknown/lib" ]; then
	echo "==> building the keyed sysroot for '${sysroot_args:-<default>}' (about a minute, then cached)"
	if ! SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work_root/build_sysroot.log" 2>&1; then
		echo "run.sh: could not build the sysroot for '${sysroot_args:-<default>}'" >&2
		echo "run.sh: see $work_root/build_sysroot.log" >&2
		exit 1
	fi
fi
if [ ! -d "$sysroot/lib/rustlib/wasm32-unknown-unknown/lib" ]; then
	echo "run.sh: no sysroot at $sysroot" >&2
	exit 1
fi
# The banner: which args the JS side compiles with, and the key of the sysroot that answers
# them. `js-lane` is link-time and so absent from the key's args on purpose.
echo "==> js side: args='${JS_EXTRA_ARGS:-<none>}' (emit-affecting: '${sysroot_args:-<default>}')"
echo "==> sysroot: $(basename -- "$sysroot")"

if ! rustup "+$channel" target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then
	echo "==> installing the wasm32-unknown-unknown target for $channel"
	if ! rustup "+$channel" target add wasm32-unknown-unknown >/dev/null 2>&1; then
		echo "run.sh: could not install the wasm32-unknown-unknown target" >&2
		exit 1
	fi
fi

# compile_llvm <src.rs> <out.wasm> <log>
#
# `--import-undefined` is what turns the `extern "C"` print declarations into wasm imports instead
# of link errors; the JS backend needs no equivalent, because a foreign call there is a call on the
# runtime shim. `--remap-path-prefix` is spelled exactly as scripts/compile_core.sh spells it, so a
# `#[track_caller]` location printed by the panic handler reads the same on both sides.
compile_llvm() {
	rustc "+$channel" \
		--edition 2021 \
		--target wasm32-unknown-unknown \
		--crate-type cdylib \
		-Ccodegen-units=1 -Cpanic=abort -Coverflow-checks=off -Cdebug-assertions=off \
		-Cdebuginfo=0 -O \
		-Clink-arg=--import-undefined \
		--remap-path-prefix "$root/=" \
		-o "$2" "$1" >"$3" 2>&1
}

# compare <src.rs> <dir>: compiles and runs both sides.
#
#   0  the two disagree             (a finding, or the shrinker's "still reproduces")
#   1  the two agree
#   2  one of the two compilers rejected the program
compare() {
	local src=$1 dir=$2
	mkdir -p "$dir"
	if ! compile_llvm "$src" "$dir/prog.wasm" "$dir/llvm.compile.log"; then
		return 2
	fi
	if ! "$script_dir/compile_core.sh" "$src" "$dir/prog.js" \
		>"$dir/js.compile.log" 2>&1; then
		return 2
	fi

	node "$fuzz_dir/shim.mjs" "$dir/prog.wasm" >"$dir/llvm.out" 2>"$dir/llvm.err"
	local llvm_status=$?
	cat "$shim" "$dir/prog.js" "$entry" | node - >"$dir/js.out" 2>"$dir/js.err"
	local js_status=$?

	llvm_class=ok
	js_class=ok
	[ "$llvm_status" -ne 0 ] && llvm_class=aborted
	[ "$js_status" -ne 0 ] && js_class=aborted

	if [ "$llvm_class" = "$js_class" ] && cmp -s "$dir/llvm.out" "$dir/js.out"; then
		return 1
	fi
	diff -u "$dir/llvm.out" "$dir/js.out" >"$dir/stdout.diff"
	return 0
}

# shrink <dir>: the smallest line-wise program that still diverges.
#
# One line at a time, from the bottom up: a drop is kept when both compilers still accept the
# program and the two still disagree, and reverted otherwise. Repeated until a pass changes
# nothing, which is what removes a statement whose only reason to stay was a line dropped later.
SHRINK_BUDGET=${SHRINK_BUDGET:-400}
shrink() {
	local dir=$1
	local work="$dir/shrink"
	rm -rf "$work"
	mkdir -p "$work/try"
	cp "$dir/prog.rs" "$work/prog.rs"

	local attempts=0 changed=1 pass=0
	while [ "$changed" -eq 1 ] && [ "$attempts" -lt "$SHRINK_BUDGET" ]; do
		changed=0
		pass=$((pass + 1))
		local line
		line=$(wc -l <"$work/prog.rs" | tr -d ' ')
		while [ "$line" -ge 1 ] && [ "$attempts" -lt "$SHRINK_BUDGET" ]; do
			attempts=$((attempts + 1))
			sed "${line}d" "$work/prog.rs" >"$work/try/prog.rs"
			if compare "$work/try/prog.rs" "$work/try" ; then
				cp "$work/try/prog.rs" "$work/prog.rs"
				changed=1
			fi
			line=$((line - 1))
		done
		echo "    shrink pass $pass: $(wc -l <"$work/prog.rs" | tr -d ' ') lines, $attempts attempts"
	done
	cp "$work/prog.rs" "$dir/shrunk.rs"
	echo "    shrunk repro: $dir/shrunk.rs"
}

agreed=0
mismatched=0
rejected=0
mismatch_seeds=""
reject_seeds=""

seed=$first_seed
last=$((first_seed + count - 1))
while [ "$seed" -le "$last" ]; do
	dir="$work_root/$seed"
	rm -rf "$dir"
	mkdir -p "$dir"
	node "$fuzz_dir/gen.mjs" "$seed" ${family_args[@]+"${family_args[@]}"} >"$dir/prog.rs"
	family=$(sed -n '1s/.*family \([abc]\).*/\1/p' "$dir/prog.rs")

	compare "$dir/prog.rs" "$dir"
	case $? in
	1)
		agreed=$((agreed + 1))
		printf '%6d  %s  agree (%s)\n' "$seed" "$family" "$llvm_class"
		[ "$keep" -eq 1 ] || rm -rf "$dir"
		;;
	2)
		rejected=$((rejected + 1))
		reject_seeds="$reject_seeds $seed"
		printf '%6d  %s  COMPILE ERROR (kept in %s)\n' "$seed" "$family" "$dir"
		;;
	*)
		mismatched=$((mismatched + 1))
		mismatch_seeds="$mismatch_seeds $seed"
		printf '%6d  %s  MISMATCH (llvm %s, js %s) — %s\n' \
			"$seed" "$family" "$llvm_class" "$js_class" "$dir"
		sed 's/^/      /' "$dir/stdout.diff" | head -20
		shrink "$dir"
		;;
	esac
	seed=$((seed + 1))
done

echo
echo "================================================================"
printf 'seeds %d..%d: %d agreed, %d mismatched, %d rejected\n' \
	"$first_seed" "$last" "$agreed" "$mismatched" "$rejected"
[ -n "$mismatch_seeds" ] && echo "mismatched seeds:$mismatch_seeds"
[ -n "$reject_seeds" ] && echo "rejected seeds:$reject_seeds"
echo "================================================================"

[ "$mismatched" -eq 0 ] && [ "$rejected" -eq 0 ]
