#!/usr/bin/env bash
#
# The wasm32-LLVM-vs-JS differential fuzzer.
#
# Usage:
#   scripts/fuzz/run.sh <count> [options]
#   scripts/fuzz/run.sh --ci-smoke [options]
#
# Examples:
#   scripts/fuzz/run.sh 200                             # seeds 0..199, all 6 families
#   scripts/fuzz/run.sh 24 --all-profiles -j 4          # 4 seeds/family across all release profiles
#   scripts/fuzz/run.sh 24 --all-profiles --mir-inline -j 4
#   scripts/fuzz/run.sh --ci-smoke -j 4                 # 24 seeds across dev, release-balanced, release-speed
#   scripts/fuzz/run.sh 20 --seed 1000 --family d       # heap/alloc family starting at seed 1000
#   scripts/fuzz/run.sh 10 --profile release-speed      # single profile mode
#   scripts/fuzz/run.sh 50 --lane jir --opt on          # legacy lane/opt spelling
#
# For each seed, scripts/fuzz/gen.mjs writes one defined-behaviour `#![no_std]` program, and that
# ONE source is compiled:
#
#   * by the ordinary LLVM backend for `wasm32-unknown-unknown` (with `--cfg fuzz_wasm_alloc` so
#     alloc families get a static bump allocator) and run under Node's `WebAssembly.instantiate`
#     with scripts/fuzz/shim.mjs supplying the print imports;
#   * by rustc_codegen_mires through scripts/compile_core.sh for every selected profile, and run
#     under Node with runtime/shim.js + prog.js + scripts/entry.js.
#
# Every JS build must print the exact same stdout and finish with the same exit class (normal vs.
# aborted) as LLVM wasm32. Any mismatch keeps `build/fuzz/<seed>/` and shrinks the failing repro
# line-by-line to `build/fuzz/<seed>/shrunk.rs`.

set -uo pipefail

fuzz_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
script_dir=$(cd -- "$fuzz_dir/.." && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=../sysroot-cache.sh disable=SC1091
source "$script_dir/sysroot-cache.sh"

# Always enable optimizer witness verification by default during fuzzing.
: "${RCGJS_WITNESS_CHECK:=1}"
export RCGJS_WITNESS_CHECK

SHRINK_BUDGET=${SHRINK_BUDGET:-400}

usage() {
	cat <<'EOF'
usage: run.sh [<count>] [options]

Options:
  --seed <first>           First seed index (default: 0)
  --family <a|b|c|d|e|f>   Restrict generation to one family (default: seed % 6)
  --profile <name>         Profile: dev | release-size | release-balanced | release-speed
  --all-profiles           Compare LLVM wasm32 vs dev, release-size, release-balanced, release-speed
  --mir-inline             Also compare release-speed with JSC_RUST_MIR_INLINE=on
  --ci-smoke               Preset: 24 seeds across dev, release-balanced, release-speed
  -j, --jobs <N>           Parallel worker count (default: 1)
  --keep                   Keep build/fuzz/<seed>/ for passing seeds too
  --include-large-enums    Also generate 10-variant enums in family b
  --lane <baseline|jir>    Legacy backend lane flag
  --opt <off|report|on>    Legacy optimizer mode flag
  -h, --help               Show this help text
EOF
}

item_in_list() {
	local needle=$1
	shift
	local item
	for item in "$@"; do
		if [ "$item" = "$needle" ]; then
			return 0
		fi
	done
	return 1
}

# Maps a named profile label to its JS_EXTRA_ARGS string.
profile_extra_args() {
	local base=${2:-}
	case "$1" in
	default)
		printf '%s' "$base"
		;;
	dev | release-size | release-balanced | release-speed)
		printf '%s' "${base:+$base }js-profile=$1"
		;;
	release-speed-mir-inline)
		printf '%s' "${base:+$base }js-profile=release-speed"
		;;
	*)
		return 1
		;;
	esac
}

profile_mir_inline() {
	case "$1" in
	release-speed-mir-inline) printf '%s' 'on' ;;
	*) printf '%s' "${JSC_RUST_MIR_INLINE:-off}" ;;
	esac
}

# compile_llvm <channel> <src.rs> <out.wasm> <log>
compile_llvm() {
	local ch=$1 src=$2 out=$3 log=$4
	rustc "+$ch" \
		--edition 2021 \
		--target wasm32-unknown-unknown \
		--crate-type cdylib \
		--cfg fuzz_wasm_alloc \
		-Ccodegen-units=1 -Cpanic=abort -Coverflow-checks=off -Cdebug-assertions=off \
		-Cdebuginfo=0 -O \
		-Clink-arg=--import-undefined \
		--remap-path-prefix "$root/=" \
		-o "$out" "$src" >"$log" 2>&1
}

# compare_one_profile <channel> <src.rs> <dir> <extra_args> <mir_inline>
#
#   0  LLVM and JS disagree         (finding / shrinker "still reproduces")
#   1  LLVM and JS agree
#   2  either compiler rejected src
compare_one_profile() {
	local ch=$1 src=$2 dir=$3 extra_args=$4 mir_inline=$5
	local shim="$root/runtime/shim.js"
	local entry="$script_dir/entry.js"
	mkdir -p "$dir"
	if ! compile_llvm "$ch" "$src" "$dir/prog.wasm" "$dir/llvm.compile.log"; then
		return 2
	fi
	if ! JS_EXTRA_ARGS="$extra_args" JSC_RUST_MIR_INLINE="$mir_inline" \
		"$script_dir/compile_core.sh" "$src" "$dir/prog.js" \
		>"$dir/js.compile.log" 2>&1; then
		return 2
	fi

	node "$fuzz_dir/shim.mjs" "$dir/prog.wasm" >"$dir/llvm.out" 2>"$dir/llvm.err"
	local llvm_status=$?
	cat "$shim" "$dir/prog.js" "$entry" | node - >"$dir/js.out" 2>"$dir/js.err"
	local js_status=$?

	local llvm_class=ok
	local js_class=ok
	[ "$llvm_status" -ne 0 ] && llvm_class=aborted
	[ "$js_status" -ne 0 ] && js_class=aborted

	if [ "$llvm_class" = "$js_class" ] && cmp -s "$dir/llvm.out" "$dir/js.out"; then
		return 1
	fi
	diff -u "$dir/llvm.out" "$dir/js.out" >"$dir/stdout.diff"
	return 0
}

# shrink_profile <channel> <seed_dir> <profile> <extra_args> <mir_inline>
shrink_profile() {
	local ch=$1 seed_dir=$2 prof=$3 extra_args=$4 mir_inline=$5
	local work="$seed_dir/shrink-$prof"
	rm -rf "$work"
	mkdir -p "$work/try"
	cp "$seed_dir/prog.rs" "$work/prog.rs"

	local attempts=0 changed=1 pass=0
	while [ "$changed" -eq 1 ] && [ "$attempts" -lt "$SHRINK_BUDGET" ]; do
		changed=0
		pass=$((pass + 1))
		local line
		line=$(wc -l <"$work/prog.rs" | tr -d ' ')
		while [ "$line" -ge 1 ] && [ "$attempts" -lt "$SHRINK_BUDGET" ]; do
			attempts=$((attempts + 1))
			sed "${line}d" "$work/prog.rs" >"$work/try/prog.rs"
			if compare_one_profile "$ch" "$work/try/prog.rs" "$work/try" "$extra_args" "$mir_inline"; then
				cp "$work/try/prog.rs" "$work/prog.rs"
				changed=1
			fi
			line=$((line - 1))
		done
	done
	local final_lines
	final_lines=$(wc -l <"$work/prog.rs" | tr -d ' ')
	cp "$work/prog.rs" "$seed_dir/shrunk.$prof.rs"
	if [ ! -f "$seed_dir/shrunk.rs" ]; then
		cp "$work/prog.rs" "$seed_dir/shrunk.rs"
	fi
	printf '      shrunk (%s): %s lines in %d passes (%d attempts) -> %s/shrunk.%s.rs\n' \
		"$prof" "$final_lines" "$pass" "$attempts" "$seed_dir" "$prof"
	rm -rf "$work"
}

# run_seed_worker <work_root> <channel> <profiles_csv> <base_args> <family_opt> <large_enums> <keep> <seed>
run_seed_worker() {
	local work_root=$1
	local ch=$2
	local profiles_csv=$3
	local base_args=$4
	local family_opt=$5
	local large_enums=$6
	local keep=$7
	local seed=$8

	local shim="$root/runtime/shim.js"
	local entry="$script_dir/entry.js"
	local dir="$work_root/$seed"
	local res_file="$work_root/.results/$seed.tsv"

	rm -rf "$dir"
	mkdir -p "$dir"

	local gen_args=()
	if [ "$family_opt" != "-" ]; then
		gen_args+=(--family "$family_opt")
	fi
	if [ "$large_enums" -eq 1 ]; then
		gen_args+=(--include-large-enums)
	fi

	if ! node "$fuzz_dir/gen.mjs" "$seed" ${gen_args[@]+"${gen_args[@]}"} >"$dir/prog.rs" 2>"$dir/gen.err"; then
		printf '%s\t?\tREJECTED\tgenerator-error\n' "$seed" >"$res_file"
		printf '%6d  ?  GENERATOR ERROR (kept in %s)\n' "$seed" "$dir"
		return 0
	fi

	local family
	family=$(sed -n '1s/.*family \([a-f]\).*/\1/p' "$dir/prog.rs")
	: "${family:=?}"

	# Compile & run LLVM wasm32 ONCE per seed (shared oracle across all JS profiles).
	if ! compile_llvm "$ch" "$dir/prog.rs" "$dir/prog.wasm" "$dir/llvm.compile.log"; then
		printf '%s\t%s\tREJECTED\tllvm-compile-error\n' "$seed" "$family" >"$res_file"
		printf '%6d  %s  COMPILE ERROR [llvm] (kept in %s)\n' "$seed" "$family" "$dir"
		return 0
	fi
	node "$fuzz_dir/shim.mjs" "$dir/prog.wasm" >"$dir/llvm.out" 2>"$dir/llvm.err"
	local llvm_status=$?
	local llvm_class=ok
	[ "$llvm_status" -ne 0 ] && llvm_class=aborted

	local raw_profiles=()
	IFS=, read -r -a raw_profiles <<<"$profiles_csv"
	local profiles=()
	local p
	for p in ${raw_profiles[@]+"${raw_profiles[@]}"}; do
		[ -n "$p" ] && profiles+=("$p")
	done

	local seed_status=AGREED
	local detail_notes=""
	local first_mismatch_prof=""
	local first_mismatch_extra=""
	local first_mismatch_mir=""

	local prof
	for prof in "${profiles[@]}"; do
		local pdir="$dir/$prof"
		mkdir -p "$pdir"
		local extra_args=""
		extra_args=$(profile_extra_args "$prof" "$base_args")
		local mir_inline=""
		mir_inline=$(profile_mir_inline "$prof")

		if ! JS_EXTRA_ARGS="$extra_args" JSC_RUST_MIR_INLINE="$mir_inline" \
			"$script_dir/compile_core.sh" "$dir/prog.rs" "$pdir/prog.js" \
			>"$pdir/js.compile.log" 2>&1; then
			if [ "$seed_status" != "MISMATCH" ]; then
				seed_status=REJECTED
			fi
			detail_notes="${detail_notes:+$detail_notes,}$prof:compile-error"
			continue
		fi

		cat "$shim" "$pdir/prog.js" "$entry" | node - >"$pdir/js.out" 2>"$pdir/js.err"
		local js_status=$?
		local js_class=ok
		[ "$js_status" -ne 0 ] && js_class=aborted

		if [ "$llvm_class" = "$js_class" ] && cmp -s "$dir/llvm.out" "$pdir/js.out"; then
			continue
		fi

		diff -u "$dir/llvm.out" "$pdir/js.out" >"$pdir/stdout.diff"
		seed_status=MISMATCH
		detail_notes="${detail_notes:+$detail_notes,}$prof:mismatch(llvm=$llvm_class,js=$js_class)"
		if [ -z "$first_mismatch_prof" ]; then
			first_mismatch_prof=$prof
			first_mismatch_extra=$extra_args
			first_mismatch_mir=$mir_inline
		fi
	done

	case "$seed_status" in
	AGREED)
		local plural="s"
		if [ "${#profiles[@]}" -eq 1 ]; then
			plural=""
		fi
		printf '%s\t%s\tAGREED\t%s\n' "$seed" "$family" "$llvm_class" >"$res_file"
		printf '%6d  %s  agree (%s, %d profile%s)\n' \
			"$seed" "$family" "$llvm_class" "${#profiles[@]}" "$plural"
		if [ "$keep" -eq 0 ]; then
			rm -rf "$dir"
		fi
		;;
	REJECTED)
		printf '%s\t%s\tREJECTED\t%s\n' "$seed" "$family" "$detail_notes" >"$res_file"
		printf '%6d  %s  COMPILE ERROR (%s) — %s\n' "$seed" "$family" "$detail_notes" "$dir"
		;;
	MISMATCH)
		printf '%s\t%s\tMISMATCH\t%s\n' "$seed" "$family" "$detail_notes" >"$res_file"
		printf '%6d  %s  MISMATCH (%s) — %s\n' "$seed" "$family" "$detail_notes" "$dir"
		if [ -f "$dir/$first_mismatch_prof/stdout.diff" ]; then
			sed 's/^/      /' "$dir/$first_mismatch_prof/stdout.diff" | head -n 20
		fi
		shrink_profile "$ch" "$dir" "$first_mismatch_prof" "$first_mismatch_extra" "$first_mismatch_mir"
		;;
	esac
}

# Internal worker entrypoint for parallel execution.
if [ "${1:-}" = "--_worker" ]; then
	shift
	run_seed_worker "$@"
	exit 0
fi

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

count=""
first_seed=0
family_opt="-"
keep=0
lane=""
opt=""
profile=""
all_profiles=0
mir_inline_flag=0
ci_smoke=0
large_enums=0
jobs=1

if [ "$#" -gt 0 ] && [ "$1" -ge 1 ] 2>/dev/null; then
	count=$1
	shift
fi

while [ "$#" -gt 0 ]; do
	case "$1" in
	--seed)
		if [ "$#" -lt 2 ]; then
			echo "run.sh: --seed requires an integer" >&2
			exit 2
		fi
		first_seed=$2
		shift 2
		;;
	--family)
		if [ "$#" -lt 2 ]; then
			echo "run.sh: --family requires a family letter (a|b|c|d|e|f)" >&2
			exit 2
		fi
		case "$2" in
		a | b | c | d | e | f) family_opt=$2 ;;
		*)
			echo "run.sh: --family must be a, b, c, d, e or f, got: $2" >&2
			exit 2
			;;
		esac
		shift 2
		;;
	--keep)
		keep=1
		shift
		;;
	--include-large-enums)
		large_enums=1
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
	--profile)
		profile=${2:-}
		case "$profile" in
		dev | release-size | release-balanced | release-speed) ;;
		*)
			echo "run.sh: --profile must be dev, release-size, release-balanced or release-speed, got: $profile" >&2
			exit 2
			;;
		esac
		shift 2
		;;
	--all-profiles)
		all_profiles=1
		shift
		;;
	--mir-inline)
		mir_inline_flag=1
		shift
		;;
	--ci-smoke)
		ci_smoke=1
		shift
		;;
	-j | --jobs)
		if [ "$#" -lt 2 ]; then
			echo "run.sh: $1 requires an integer" >&2
			exit 2
		fi
		jobs=$2
		shift 2
		;;
	-j[0-9]*)
		jobs=${1#-j}
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	[0-9]*)
		if [ -z "$count" ] && [ "$1" -ge 1 ] 2>/dev/null; then
			count=$1
			shift
		else
			echo "run.sh: unexpected argument: $1" >&2
			usage >&2
			exit 2
		fi
		;;
	*)
		echo "run.sh: unknown option: $1" >&2
		usage >&2
		exit 2
		;;
	esac
done

if [ "$ci_smoke" -eq 1 ] && [ -z "$count" ]; then
	count=24
fi

if [ -z "$count" ] || ! [ "$count" -ge 1 ] 2>/dev/null; then
	usage >&2
	exit 2
fi

if ! [ "$first_seed" -ge 0 ] 2>/dev/null; then
	echo "run.sh: --seed must be a non-negative integer, got: $first_seed" >&2
	exit 2
fi

if ! [ "$jobs" -ge 1 ] 2>/dev/null; then
	echo "run.sh: --jobs must be a positive integer, got: $jobs" >&2
	exit 2
fi

# Compose base JS_EXTRA_ARGS (honoring caller's $JS_EXTRA_ARGS plus legacy --lane/--opt).
base_extra_args=${JS_EXTRA_ARGS:-}
if [ -n "$lane" ]; then
	base_extra_args="${base_extra_args:+$base_extra_args }js-lane=$lane"
fi
if [ -n "$opt" ]; then
	base_extra_args="${base_extra_args:+$base_extra_args }js-opt=$opt"
fi

# Resolve active profile list.
selected_profiles=()
if [ "$all_profiles" -eq 1 ]; then
	selected_profiles=(dev release-size release-balanced release-speed)
elif [ "$ci_smoke" -eq 1 ]; then
	selected_profiles=(dev release-balanced release-speed)
elif [ -n "$profile" ]; then
	selected_profiles=("$profile")
elif [ "$mir_inline_flag" -eq 1 ]; then
	selected_profiles=()
else
	selected_profiles=(default)
fi

if [ "$mir_inline_flag" -eq 1 ]; then
	selected_profiles+=(release-speed-mir-inline)
fi

profiles_csv=""
for p in "${selected_profiles[@]}"; do
	profiles_csv="${profiles_csv:+$profiles_csv,}$p"
done

# ---------------------------------------------------------------------------
# Setup: lock, backend check, sequential sysroot pre-build, wasm target check
# ---------------------------------------------------------------------------

# Hold integration-lock.sh once in the parent so sequential sysroot builds never
# race external scripts while parallel compile_core.sh workers run without contention.
# shellcheck source=../integration-lock.sh disable=SC1091
source "$script_dir/integration-lock.sh"
jsc_target_name=""
jsc_panic_mode=""
jsc_select_mode "$root" || exit 2

work_root="$root/build/fuzz"
mkdir -p "$work_root"
rm -rf "$work_root/.results"
mkdir -p "$work_root/.results"

channel=$(jsc_pinned_channel "$root")

case "$(uname -s)" in
Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
esac
if [ ! -f "$dylib" ]; then
	echo "run.sh: backend not built: $dylib (run scripts/build.sh)" >&2
	exit 1
fi

# Pre-build every required keyed sysroot sequentially BEFORE spawning parallel workers.
built_keys=()
for p in "${selected_profiles[@]}"; do
	extra_args=$(profile_extra_args "$p" "$base_extra_args")
	sysroot_args=$(JS_EXTRA_ARGS="$extra_args" jsc_sysroot_args) || exit 1
	sysroot_key=$(JS_EXTRA_ARGS="$extra_args" jsc_sysroot_key "$root" "$jsc_target_name" "$dylib") || exit 1
	sysroot=$(JS_EXTRA_ARGS="$extra_args" jsc_sysroot_path "$root" "$jsc_target_name" "$dylib") || exit 1

	if ! item_in_list "$sysroot_key" ${built_keys[@]+"${built_keys[@]}"}; then
		if [ ! -d "$sysroot/lib/rustlib/$jsc_target_name/lib" ]; then
			echo "==> building keyed sysroot for '$p' ('${sysroot_args:-<default>}', key=${sysroot_key:0:12})"
			if ! SKIP_BUILD=1 JS_EXTRA_ARGS="$extra_args" "$script_dir/build_sysroot.sh" \
				>"$work_root/build_sysroot.$sysroot_key.log" 2>&1; then
				echo "run.sh: could not build sysroot for '$p' ('${sysroot_args:-<default>}')" >&2
				echo "run.sh: see $work_root/build_sysroot.$sysroot_key.log" >&2
				exit 1
			fi
		fi
		if [ ! -d "$sysroot/lib/rustlib/$jsc_target_name/lib" ]; then
			echo "run.sh: no sysroot at $sysroot" >&2
			exit 1
		fi
		built_keys+=("$sysroot_key")
	fi
	echo "==> profile '$p': args='${extra_args:-<default>}' mir_inline=$(profile_mir_inline "$p") sysroot=${sysroot_key:0:12}"
done

if ! rustup "+$channel" target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then
	echo "==> installing the wasm32-unknown-unknown target for $channel"
	if ! rustup "+$channel" target add wasm32-unknown-unknown >/dev/null 2>&1; then
		echo "run.sh: could not install the wasm32-unknown-unknown target" >&2
		exit 1
	fi
fi

last=$((first_seed + count - 1))
echo "==> fuzzing seeds $first_seed..$last ($count seeds), profiles=[$profiles_csv], jobs=$jobs, panic=$jsc_panic_mode, witness=$RCGJS_WITNESS_CHECK"

# ---------------------------------------------------------------------------
# Run seeds (sequential or parallel via xargs -P)
# ---------------------------------------------------------------------------

start_ts=$(date +%s)

if [ "$jobs" -le 1 ]; then
	seed=$first_seed
	while [ "$seed" -le "$last" ]; do
		run_seed_worker "$work_root" "$channel" "$profiles_csv" "$base_extra_args" "$family_opt" "$large_enums" "$keep" "$seed"
		seed=$((seed + 1))
	done
else
	seeds=()
	seed=$first_seed
	while [ "$seed" -le "$last" ]; do
		seeds+=("$seed")
		seed=$((seed + 1))
	done
	printf '%s\0' "${seeds[@]}" |
		xargs -0 -P "$jobs" -n 1 \
			"$fuzz_dir/run.sh" --_worker \
			"$work_root" "$channel" "$profiles_csv" "$base_extra_args" "$family_opt" "$large_enums" "$keep"
fi

# ---------------------------------------------------------------------------
# Aggregate and report
# ---------------------------------------------------------------------------

elapsed=$(($(date +%s) - start_ts))
agreed=0
mismatched=0
rejected=0
mismatch_seeds=""
reject_seeds=""

seed=$first_seed
while [ "$seed" -le "$last" ]; do
	rfile="$work_root/.results/$seed.tsv"
	if [ ! -f "$rfile" ]; then
		rejected=$((rejected + 1))
		reject_seeds="$reject_seeds $seed"
	else
		IFS=$'\t' read -r _ _ r_status _ <"$rfile"
		case "$r_status" in
		AGREED) agreed=$((agreed + 1)) ;;
		MISMATCH)
			mismatched=$((mismatched + 1))
			mismatch_seeds="$mismatch_seeds $seed"
			;;
		*)
			rejected=$((rejected + 1))
			reject_seeds="$reject_seeds $seed"
			;;
		esac
	fi
	seed=$((seed + 1))
done
rm -rf "$work_root/.results"

echo
echo "================================================================"
printf 'seeds %d..%d (%ds): %d agreed, %d mismatched, %d rejected\n' \
	"$first_seed" "$last" "$elapsed" "$agreed" "$mismatched" "$rejected"
[ -n "$mismatch_seeds" ] && echo "mismatched seeds:$mismatch_seeds"
[ -n "$reject_seeds" ] && echo "rejected seeds:$reject_seeds"
echo "================================================================"

[ "$mismatched" -eq 0 ] && [ "$rejected" -eq 0 ]
