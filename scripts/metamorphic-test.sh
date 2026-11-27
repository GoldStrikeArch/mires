#!/usr/bin/env bash
#
# Cross-profile metamorphic test oracle for `examples/core-tests/`.
#
# Compiles and executes `examples/core-tests/` fixtures across all mires
# compiler profiles AND against the LLVM `wasm32-unknown-unknown` oracle,
# asserting that every profile agrees byte-for-byte on stdout and exit class
# (normal exit vs. abort) and matches each fixture's `.expected` golden file.
#
# Usage:
#   scripts/metamorphic-test.sh [--fast | --full] [--profiles <csv>] [-j <N>]
#                               [--skip-build] [--keep] [<fixture>...]
#
# Options:
#   --fast              Run curated ~47 high-signal fixtures across all profiles (default when no fixtures given)
#   --full              Run every fixture in examples/core-tests/ across all profiles
#   --profiles <csv>    Comma-separated profiles to check (default: all 6)
#                       Valid: dev,release-size,release-balanced,release-speed,
#                              release-speed-mir-inline,llvm-wasm32
#   -j, --jobs <N>      Run up to N fixture workers in parallel (default: 1)
#   --skip-build        Reuse existing target/release compiler binaries
#   --keep              Keep compiled .js/.wasm artifacts even for passing fixtures
#   -h, --help          Show this help text
#   <fixture>...        One or more fixture names, file names, or paths

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=sysroot-cache.sh disable=SC1091
source "$script_dir/sysroot-cache.sh"

# Always replay optimizer proof witnesses during metamorphic checks.
: "${RCGJS_WITNESS_CHECK:=1}"
export RCGJS_WITNESS_CHECK

ALL_PROFILES=(
	dev
	release-size
	release-balanced
	release-speed
	release-speed-mir-inline
	llvm-wasm32
)

# Curated high-signal subset for PR CI (`--fast`), covering every major language
# feature, runtime subsystem, and optimizer pass.
FAST_FIXTURES=(
	01_option
	02_result
	04_iter_range
	05_slice_index
	06_num
	07_mem
	08_dyn
	09_closure
	10_panic
	11_fmt
	12_slice_iter
	13_sort
	14_str
	15_ptr
	19_float_fmt
	20_alloc_box
	21_vec
	22_string
	23_format
	24_enum_repr
	26_async
	29_rc
	30_hashmap
	37_native_vec
	38_native_string
	52_sroa
	60_virtual_struct_merge
	65_flat_abi_spill
	77_pea_merge_materialize
	81_bounds_range
	97_slice_lanes
	98_product_lanes
	101_btree_map_nodes
	105_slice_iterator_cursor
	117_overflow_arith
	214_copy_writes_through_borrows
	260_static_cycles
	271_bigint_shift_rotate
	310_i64_pair_arith
	320_r3_loops_bounds
	330_r3_inline_cells
	366_byte_views_wider_scalars
	402_byte_views_analysis_soundness
	413_bstr_chars_bytes_cursor
	419_native_vec_typed_array
	guard_custom_allocator
	guard_heap_retype
)

# Fixtures that intentionally trigger mires-specific runtime safety refusals
# (use-after-free, non-scalar byte view / heap retype, as_chunks guard). In
# LLVM wasm32 linear memory these either exhibit UB or permit raw byte access
# rather than trapping, so they are cross-checked across all 5 mires profiles
# and skipped only for the llvm-wasm32 oracle.
LLVM_WASM32_RUNTIME_GUARD_SKIP=(
	367_byte_view_non_scalar_abort
	370_retyped_alloc_write_bytes_abort
	396_lazy_proxy_non_scalar_abort
	404_realloc_scaled_non_scalar_abort
	guard_as_chunks
	guard_heap_retype
	guard_use_after_free
	guard_zeroed_retype
)

# Explicit, documented skip list for genuine pre-existing `JSC_RUST_MIR_INLINE=on`
# failures, kept empty unless verified and documented with exact failure reasons.
KNOWN_MIR_INLINE_SKIP=()

usage() {
	sed -n '3,26p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
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

profile_extra_args() {
	case "$1" in
	dev) printf '%s' 'js-profile=dev' ;;
	release-size) printf '%s' 'js-profile=release-size' ;;
	release-balanced) printf '%s' 'js-profile=release-balanced' ;;
	release-speed) printf '%s' 'js-profile=release-speed' ;;
	release-speed-mir-inline) printf '%s' 'js-profile=release-speed' ;;
	*) return 1 ;;
	esac
}

profile_mir_inline() {
	case "$1" in
	release-speed-mir-inline) printf '%s' 'on' ;;
	*) printf '%s' 'off' ;;
	esac
}

# Appends a small static bump #[global_allocator] to the end of a scratch copy
# of a `#![no_std]` + `extern crate alloc` fixture so LLVM wasm32 can execute
# heap-allocating core-tests without shifting any source line numbers.
append_wasm_bump_allocator() {
	cat >>"$1" <<'EOF'

// --- llvm-wasm32 scratch allocator (appended at EOF to preserve line numbers) ---
mod __metamorphic_wasm_alloc {
    use core::alloc::{GlobalAlloc, Layout};
    use core::cell::UnsafeCell;
    use core::ptr;
    use core::sync::atomic::{AtomicUsize, Ordering};

    const HEAP_SIZE: usize = 8 * 1024 * 1024;

    #[repr(align(16))]
    struct Heap(UnsafeCell<[u8; HEAP_SIZE]>);
    unsafe impl Sync for Heap {}

    static HEAP: Heap = Heap(UnsafeCell::new([0; HEAP_SIZE]));
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Bump;

    unsafe impl GlobalAlloc for Bump {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let align = layout.align().max(1);
            let size = layout.size();
            let mut cur = NEXT.load(Ordering::Relaxed);
            loop {
                let aligned = (cur + align - 1) & !(align - 1);
                let end = match aligned.checked_add(size) {
                    Some(e) if e <= HEAP_SIZE => e,
                    _ => return ptr::null_mut(),
                };
                match NEXT.compare_exchange_weak(cur, end, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => return unsafe { (*HEAP.0.get()).as_mut_ptr().add(aligned) },
                    Err(v) => cur = v,
                }
            }
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
    }

    #[global_allocator]
    static ALLOC: Bump = Bump;
}
EOF
}

# Determines whether a fixture is eligible for the llvm-wasm32 oracle.
# Prints the skip reason and returns 1 if ineligible; returns 0 if eligible.
llvm_wasm32_skip_reason() {
	local core_dir=$1
	local name=$2
	local src="$core_dir/$name.rs"

	if [ -f "$core_dir/$name.expect_fail" ]; then
		printf 'expect-fail'
		return 1
	fi
	if item_in_list "$name" "${LLVM_WASM32_RUNTIME_GUARD_SKIP[@]}"; then
		printf 'mires-runtime-guard'
		return 1
	fi
	if grep -q 'mires_js' "$src"; then
		printf 'uses-mires-js'
		return 1
	fi
	if grep -qE 'JsStr|sb_new|sb_push|sb_take' "$src"; then
		printf 'uses-js-str-builder'
		return 1
	fi
	if grep -q 'host_throw' "$src"; then
		printf 'uses-host-throw'
		return 1
	fi
	return 0
}

# Worker entrypoint: runs all selected profiles for a single fixture, writes
# per-profile results to `$work_dir/results/<fixture>.tsv`, and prints one
# concise progress line.
run_fixture_worker() {
	local work_dir=$1
	local profiles_csv=$2
	local channel=$3
	local keep=$4
	local name=$5

	local core_dir="$root/examples/core-tests"
	local shim="$root/runtime/shim.js"
	local entry="$script_dir/entry.js"
	local src="$core_dir/$name.rs"
	local expected="$core_dir/$name.expected"
	local expect_fail="$core_dir/$name.expect_fail"
	local expect_abort="$core_dir/$name.expect_abort"
	local result_file="$work_dir/results/$name.tsv"
	local fixture_dir="$work_dir/runs/$name"
	local diffs_dir="$work_dir/diffs"
	local proc_macro_dir="$work_dir/proc-macro"

	mkdir -p "$fixture_dir" "$diffs_dir"
	: >"$result_file"

	local raw_profiles=()
	IFS=, read -r -a raw_profiles <<<"$profiles_csv"
	local profiles=()
	local p
	for p in ${raw_profiles[@]+"${raw_profiles[@]}"}; do
		[ -n "$p" ] && profiles+=("$p")
	done

	local uses_mires_js=0
	if grep -q 'mires_js' "$src"; then
		uses_mires_js=1
	fi

	local expected_class=ok
	if [ -f "$expect_abort" ]; then
		expected_class=aborted
	fi

	local fixture_failed=0
	local ran_count=0
	local pass_count=0
	local skip_count=0
	local fail_notes=""
	local comparable_profiles=()

	local prof
	for prof in "${profiles[@]}"; do
		local prof_dir="$fixture_dir/$prof"
		mkdir -p "$prof_dir"

		if [ "$prof" = "release-speed-mir-inline" ] && item_in_list "$name" ${KNOWN_MIR_INLINE_SKIP[@]+"${KNOWN_MIR_INLINE_SKIP[@]}"}; then
			printf '%s\t%s\tSKIP\tknown-mir-inline-skip\n' "$name" "$prof" >>"$result_file"
			skip_count=$((skip_count + 1))
			continue
		fi

		if [ "$prof" = "llvm-wasm32" ]; then
			local skip_reason=""
			if ! skip_reason=$(llvm_wasm32_skip_reason "$core_dir" "$name"); then
				printf '%s\t%s\tSKIP\t%s\n' "$name" "$prof" "$skip_reason" >>"$result_file"
				skip_count=$((skip_count + 1))
				continue
			fi
			ran_count=$((ran_count + 1))

			local llvm_src="$work_dir/llvm-src/examples/core-tests/$name.rs"
			cp "$src" "$llvm_src"
			if grep -q 'extern crate alloc' "$llvm_src" && ! grep -q 'global_allocator' "$llvm_src"; then
				append_wasm_bump_allocator "$llvm_src"
			fi

			if ! rustc "+$channel" \
				--edition 2021 \
				--target wasm32-unknown-unknown \
				--crate-type cdylib \
				-Ccodegen-units=1 -Cpanic=abort -Coverflow-checks=off \
				-Cdebug-assertions=off -Cdebuginfo=0 -O \
				-Clink-arg=--import-undefined \
				--remap-path-prefix "$work_dir/llvm-src/=" \
				-o "$prof_dir/prog.wasm" "$llvm_src" \
				>"$prof_dir/compile.log" 2>&1; then
				printf '%s\t%s\tFAIL\tcompile-error\n' "$name" "$prof" >>"$result_file"
				fixture_failed=1
				fail_notes="${fail_notes:+$fail_notes, }$prof:compile-error"
				continue
			fi

			node "$script_dir/fuzz/shim.mjs" "$prof_dir/prog.wasm" \
				>"$prof_dir/stdout" 2>"$prof_dir/stderr"
			local run_status=$?
			local exit_class=ok
			[ "$run_status" -ne 0 ] && exit_class=aborted
			printf '%s' "$exit_class" >"$prof_dir/exit_class"
			comparable_profiles+=("$prof")

			if [ "$exit_class" != "$expected_class" ]; then
				printf '%s\t%s\tFAIL\texit-class(%s,expected=%s)\n' \
					"$name" "$prof" "$exit_class" "$expected_class" >>"$result_file"
				fixture_failed=1
				fail_notes="${fail_notes:+$fail_notes, }$prof:exit-class($exit_class)"
				continue
			fi

			if ! diff -u "$expected" "$prof_dir/stdout" >"$diffs_dir/$name.$prof.expected.diff"; then
				printf '%s\t%s\tFAIL\tgolden-mismatch\n' "$name" "$prof" >>"$result_file"
				fixture_failed=1
				fail_notes="${fail_notes:+$fail_notes, }$prof:golden-mismatch"
				continue
			fi
			rm -f "$diffs_dir/$name.$prof.expected.diff"
			printf '%s\t%s\tPENDING_XCHECK\tok\n' "$name" "$prof" >>"$result_file"
			continue
		fi

		# Mires compiler profile
		ran_count=$((ran_count + 1))
		local extra_args=""
		extra_args=$(profile_extra_args "$prof")
		local mir_inline=""
		mir_inline=$(profile_mir_inline "$prof")

		local extern_args=()
		if [ "$uses_mires_js" -eq 1 ]; then
			local mires_js_rlib=""
			IFS= read -r mires_js_rlib <"$work_dir/meta/mires_js.$prof"
			extern_args=(--extern "mires_js=$mires_js_rlib" --search "$proc_macro_dir")
		fi

		if [ -f "$expect_fail" ]; then
			if JS_EXTRA_ARGS="$extra_args" JSC_RUST_MIR_INLINE="$mir_inline" \
				"$script_dir/compile_core.sh" "$src" "$prof_dir/prog.js" \
				${extern_args[@]+"${extern_args[@]}"} \
				>"$prof_dir/compile.log" 2>&1; then
				printf '%s\t%s\tFAIL\tunexpected-compile-success\n' "$name" "$prof" >>"$result_file"
				fixture_failed=1
				fail_notes="${fail_notes:+$fail_notes, }$prof:unexpected-compile-success"
				continue
			fi

			local missing=0
			local pattern
			while IFS= read -r pattern; do
				[ -z "$pattern" ] && continue
				if ! grep -qF -- "$pattern" "$prof_dir/compile.log"; then
					missing=1
					break
				fi
			done <"$expect_fail"

			if [ "$missing" -ne 0 ]; then
				printf '%s\t%s\tFAIL\texpect-fail-wrong-message\n' "$name" "$prof" >>"$result_file"
				fixture_failed=1
				fail_notes="${fail_notes:+$fail_notes, }$prof:wrong-error-message"
			else
				printf '%s\t%s\tPASS\texpected-failure\n' "$name" "$prof" >>"$result_file"
				pass_count=$((pass_count + 1))
			fi
			continue
		fi

		if ! JS_EXTRA_ARGS="$extra_args" JSC_RUST_MIR_INLINE="$mir_inline" \
			"$script_dir/compile_core.sh" "$src" "$prof_dir/prog.js" \
			${extern_args[@]+"${extern_args[@]}"} \
			>"$prof_dir/compile.log" 2>&1; then
			printf '%s\t%s\tFAIL\tcompile-error\n' "$name" "$prof" >>"$result_file"
			fixture_failed=1
			fail_notes="${fail_notes:+$fail_notes, }$prof:compile-error"
			continue
		fi

		cat "$shim" "$prof_dir/prog.js" "$entry" | node - >"$prof_dir/stdout" 2>"$prof_dir/stderr"
		local js_status=$?
		local js_class=ok
		[ "$js_status" -ne 0 ] && js_class=aborted
		printf '%s' "$js_class" >"$prof_dir/exit_class"
		comparable_profiles+=("$prof")

		if [ "$js_class" != "$expected_class" ]; then
			printf '%s\t%s\tFAIL\texit-class(%s,expected=%s)\n' \
				"$name" "$prof" "$js_class" "$expected_class" >>"$result_file"
			fixture_failed=1
			fail_notes="${fail_notes:+$fail_notes, }$prof:exit-class($js_class)"
			continue
		fi

		if ! diff -u "$expected" "$prof_dir/stdout" >"$diffs_dir/$name.$prof.expected.diff"; then
			printf '%s\t%s\tFAIL\tgolden-mismatch\n' "$name" "$prof" >>"$result_file"
			fixture_failed=1
			fail_notes="${fail_notes:+$fail_notes, }$prof:golden-mismatch"
			continue
		fi
		rm -f "$diffs_dir/$name.$prof.expected.diff"
		printf '%s\t%s\tPENDING_XCHECK\tok\n' "$name" "$prof" >>"$result_file"
	done

	# Cross-profile oracle check across every profile that compiled & executed.
	# Prefer llvm-wasm32 as the canonical reference when present, else dev, else first.
	if [ "${#comparable_profiles[@]}" -gt 0 ]; then
		local ref_prof="${comparable_profiles[0]}"
		for prof in "${comparable_profiles[@]}"; do
			if [ "$prof" = "dev" ]; then
				ref_prof="dev"
			fi
		done
		for prof in "${comparable_profiles[@]}"; do
			if [ "$prof" = "llvm-wasm32" ]; then
				ref_prof="llvm-wasm32"
			fi
		done

		local ref_class=""
		IFS= read -r ref_class <"$fixture_dir/$ref_prof/exit_class"
		local ref_out="$fixture_dir/$ref_prof/stdout"

		local finalized_file="$result_file.final"
		: >"$finalized_file"
		local r_name r_prof r_status r_detail
		while IFS=$'\t' read -r r_name r_prof r_status r_detail; do
			if [ "$r_status" = "PENDING_XCHECK" ]; then
				local p_class=""
				IFS= read -r p_class <"$fixture_dir/$r_prof/exit_class"
				local p_out="$fixture_dir/$r_prof/stdout"
				if [ "$p_class" != "$ref_class" ]; then
					printf '%s\t%s\tFAIL\texit-diverge-vs-%s(%s!=%s)\n' \
						"$r_name" "$r_prof" "$ref_prof" "$p_class" "$ref_class" >>"$finalized_file"
					fixture_failed=1
					fail_notes="${fail_notes:+$fail_notes, }$r_prof:diverge-vs-$ref_prof"
				elif ! cmp -s "$ref_out" "$p_out"; then
					diff -u "$ref_out" "$p_out" >"$diffs_dir/$name.$r_prof.vs-$ref_prof.diff"
					printf '%s\t%s\tFAIL\tstdout-diverge-vs-%s\n' \
						"$r_name" "$r_prof" "$ref_prof" >>"$finalized_file"
					fixture_failed=1
					fail_notes="${fail_notes:+$fail_notes, }$r_prof:diverge-vs-$ref_prof"
				else
					printf '%s\t%s\tPASS\t%s\n' "$r_name" "$r_prof" "$r_detail" >>"$finalized_file"
					pass_count=$((pass_count + 1))
				fi
			else
				printf '%s\t%s\t%s\t%s\n' "$r_name" "$r_prof" "$r_status" "$r_detail" >>"$finalized_file"
			fi
		done <"$result_file"
		mv "$finalized_file" "$result_file"
	fi

	if [ "$fixture_failed" -eq 0 ]; then
		local skip_note=""
		if [ "$skip_count" -gt 0 ]; then
			skip_note=", $skip_count skipped"
		fi
		printf '  PASS  %-42s (%d/%d passed%s)\n' \
			"$name" "$pass_count" "$ran_count" "$skip_note"
		if [ "$keep" -eq 0 ]; then
			rm -rf "$fixture_dir"
			rm -f "$work_dir/llvm-src/examples/core-tests/$name.rs"
		fi
	else
		printf '  FAIL  %-42s (%s)\n' "$name" "$fail_notes"
	fi
}

# Internal worker dispatch (used by xargs -P for parallel runs).
if [ "${1:-}" = "--_worker" ]; then
	shift
	run_fixture_worker "$@"
	exit 0
fi

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

mode=""
profiles_csv=""
jobs=1
skip_build=0
keep=0
filters=()

while [ "$#" -gt 0 ]; do
	case "$1" in
	--fast)
		mode=fast
		shift
		;;
	--full)
		mode=full
		shift
		;;
	--profiles)
		if [ "$#" -lt 2 ]; then
			echo "metamorphic-test.sh: --profiles requires a comma-separated list" >&2
			exit 2
		fi
		profiles_csv=$2
		shift 2
		;;
	-j | --jobs)
		if [ "$#" -lt 2 ]; then
			echo "metamorphic-test.sh: $1 requires an integer argument" >&2
			exit 2
		fi
		jobs=$2
		shift 2
		;;
	-j[0-9]*)
		jobs=${1#-j}
		shift
		;;
	--skip-build)
		skip_build=1
		shift
		;;
	--keep)
		keep=1
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	-*)
		echo "metamorphic-test.sh: unknown option: $1" >&2
		usage >&2
		exit 2
		;;
	*)
		filters+=("$1")
		shift
		;;
	esac
done

if ! [ "$jobs" -ge 1 ] 2>/dev/null; then
	echo "metamorphic-test.sh: --jobs must be a positive integer, got: $jobs" >&2
	exit 2
fi

if [ -z "$profiles_csv" ]; then
	for p in "${ALL_PROFILES[@]}"; do
		profiles_csv="${profiles_csv:+$profiles_csv,}$p"
	done
fi

# Validate selected profiles and deduplicate in order.
raw_profiles=()
IFS=, read -r -a raw_profiles <<<"$profiles_csv"
selected_profiles=()
for p in ${raw_profiles[@]+"${raw_profiles[@]}"}; do
	[ -z "$p" ] && continue
	if ! item_in_list "$p" "${ALL_PROFILES[@]}"; then
		echo "metamorphic-test.sh: unknown profile: $p" >&2
		echo "metamorphic-test.sh: valid profiles: ${ALL_PROFILES[*]}" >&2
		exit 2
	fi
	if ! item_in_list "$p" ${selected_profiles[@]+"${selected_profiles[@]}"}; then
		selected_profiles+=("$p")
	fi
done

if [ "${#selected_profiles[@]}" -eq 0 ]; then
	echo "metamorphic-test.sh: no profiles selected" >&2
	exit 2
fi

normalized_profiles_csv=""
for p in "${selected_profiles[@]}"; do
	normalized_profiles_csv="${normalized_profiles_csv:+$normalized_profiles_csv,}$p"
done

# Collect target fixtures.
core_dir="$root/examples/core-tests"
fixtures=()
if [ "${#filters[@]}" -gt 0 ]; then
	for raw in "${filters[@]}"; do
		name=$(basename -- "$raw")
		name=${name%.rs}
		if [ ! -f "$core_dir/$name.rs" ] || [ "$name" = "prelude" ]; then
			echo "metamorphic-test.sh: no such core-test fixture: $raw" >&2
			exit 2
		fi
		fixtures+=("$name")
	done
elif [ "$mode" = "full" ]; then
	for src in "$core_dir"/*.rs; do
		[ -f "$src" ] || continue
		name=$(basename -- "${src%.rs}")
		[ "$name" = "prelude" ] && continue
		fixtures+=("$name")
	done
else
	for name in "${FAST_FIXTURES[@]}"; do
		if [ ! -f "$core_dir/$name.rs" ]; then
			echo "metamorphic-test.sh: curated fast fixture missing: $name" >&2
			exit 1
		fi
		fixtures+=("$name")
	done
fi

if [ "${#fixtures[@]}" -eq 0 ]; then
	echo "metamorphic-test.sh: no fixtures selected" >&2
	exit 1
fi

# ---------------------------------------------------------------------------
# Setup: lock, compiler build, sequential sysroots, mires-js rlibs, wasm target
# ---------------------------------------------------------------------------

# Acquire the shared integration lock once in the parent so sequential sysroot
# preparation is serialized against other scripts while parallel compile_core.sh
# workers inherit RCGJS_INTEGRATION_LOCK_HELD and compile concurrently.
# shellcheck source=integration-lock.sh disable=SC1091
source "$script_dir/integration-lock.sh"
jsc_target_name=""
jsc_panic_mode=""
jsc_select_mode "$root" || exit 2

work_dir="$root/build/metamorphic"
rm -rf "$work_dir"
mkdir -p "$work_dir/results" "$work_dir/runs" "$work_dir/diffs" "$work_dir/meta"

start_ts=$(date +%s)

if [ "$skip_build" -eq 0 ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" --locked; then
		echo "metamorphic-test.sh: backend build failed" >&2
		exit 1
	fi
fi

case "$(uname -s)" in
Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
esac

if [ ! -f "$dylib" ]; then
	echo "metamorphic-test.sh: backend not built: $dylib (run scripts/build.sh)" >&2
	exit 1
fi

channel=$(jsc_pinned_channel "$root")

# Check whether any selected fixture needs mires_js or llvm-wasm32.
need_mires_js=0
need_llvm=0
for p in "${selected_profiles[@]}"; do
	if [ "$p" = "llvm-wasm32" ]; then
		need_llvm=1
	fi
done

for name in "${fixtures[@]}"; do
	if grep -q 'mires_js' "$core_dir/$name.rs"; then
		need_mires_js=1
		break
	fi
done

if [ "$need_llvm" -eq 1 ]; then
	if ! rustup "+$channel" target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then
		echo "==> installing wasm32-unknown-unknown target for $channel"
		if ! rustup "+$channel" target add wasm32-unknown-unknown >/dev/null 2>&1; then
			echo "metamorphic-test.sh: could not install wasm32-unknown-unknown target" >&2
			exit 1
		fi
	fi
	mkdir -p "$work_dir/llvm-src/examples/core-tests"
	cp "$core_dir/prelude.rs" "$work_dir/llvm-src/examples/core-tests/prelude.rs"
fi

# Pre-build js-extern-macro once if any selected fixture imports mires_js.
proc_macro=""
proc_macro_dir="$work_dir/proc-macro"
has_mires_profile=0
for p in "${selected_profiles[@]}"; do
	if [ "$p" != "llvm-wasm32" ]; then
		has_mires_profile=1
		break
	fi
done

if [ "$need_mires_js" -eq 1 ] && [ "$has_mires_profile" -eq 1 ]; then
	echo "==> building js-extern-macro for mires-js"
	if ! (cd -- "$root" && cargo build -p js-extern-macro) >"$work_dir/js-extern-macro.log" 2>&1; then
		echo "metamorphic-test.sh: failed to build js-extern-macro (see $work_dir/js-extern-macro.log)" >&2
		cat "$work_dir/js-extern-macro.log" >&2
		exit 1
	fi
	mkdir -p "$proc_macro_dir"
	case "$(uname -s)" in
	Darwin) proc_macro="$root/target/debug/libjs_extern_macro.dylib" ;;
	*) proc_macro="$root/target/debug/libjs_extern_macro.so" ;;
	esac
	ln -sf "$proc_macro" "$proc_macro_dir/$(basename -- "$proc_macro")"
fi

# CRITICAL: pre-build all required keyed sysroots (and per-sysroot mires_js
# rlibs) SEQUENTIALLY before spawning parallel fixture workers so build_sysroot.sh
# locks never collide.
built_keys=()
for p in "${selected_profiles[@]}"; do
	[ "$p" = "llvm-wasm32" ] && continue
	extra_args=$(profile_extra_args "$p")
	sysroot_key=$(JS_EXTRA_ARGS="$extra_args" jsc_sysroot_key "$root" "$jsc_target_name" "$dylib") || exit 1
	sysroot=$(JS_EXTRA_ARGS="$extra_args" jsc_sysroot_path "$root" "$jsc_target_name" "$dylib") || exit 1

	if ! item_in_list "$sysroot_key" ${built_keys[@]+"${built_keys[@]}"}; then
		if [ ! -d "$sysroot/lib/rustlib/$jsc_target_name/lib" ]; then
			echo "==> building keyed sysroot for profile '$p' (key=${sysroot_key:0:12})"
			if ! SKIP_BUILD=1 JS_EXTRA_ARGS="$extra_args" "$script_dir/build_sysroot.sh" \
				>"$work_dir/build_sysroot.$sysroot_key.log" 2>&1; then
				echo "metamorphic-test.sh: failed building sysroot for '$p'" >&2
				echo "metamorphic-test.sh: see $work_dir/build_sysroot.$sysroot_key.log" >&2
				exit 1
			fi
		fi
		if [ "$need_mires_js" -eq 1 ]; then
			mires_js_dir="$work_dir/mires-js/$sysroot_key"
			mkdir -p "$mires_js_dir"
			mires_js_rlib="$mires_js_dir/libmires_js.rlib"
			emit_args=$(jsc_normalize_emit_args "$extra_args")
			echo "==> building mires-js rlib for sysroot ${sysroot_key:0:12} ($p)"
			if ! JS_EXTRA_ARGS="$emit_args" "$script_dir/compile_core.sh" \
				"$root/mires-js/src/lib.rs" "$mires_js_rlib" \
				--crate-type rlib --crate-name mires_js \
				--extern "js_extern_macro=$proc_macro" \
				>"$mires_js_dir/build.log" 2>&1; then
				echo "metamorphic-test.sh: failed building mires-js for '$p'" >&2
				cat "$mires_js_dir/build.log" >&2
				exit 1
			fi
		fi
		built_keys+=("$sysroot_key")
	fi

	if [ "$need_mires_js" -eq 1 ]; then
		printf '%s\n' "$work_dir/mires-js/$sysroot_key/libmires_js.rlib" >"$work_dir/meta/mires_js.$p"
	fi
done

echo "==> metamorphic oracle: ${#fixtures[@]} fixture(s), profiles=[$normalized_profiles_csv], jobs=$jobs, panic=$jsc_panic_mode, witness=$RCGJS_WITNESS_CHECK"

# ---------------------------------------------------------------------------
# Execute fixtures (sequential or parallel via xargs -P)
# ---------------------------------------------------------------------------

if [ "$jobs" -le 1 ]; then
	for name in "${fixtures[@]}"; do
		run_fixture_worker "$work_dir" "$normalized_profiles_csv" "$channel" "$keep" "$name"
	done
else
	printf '%s\0' "${fixtures[@]}" |
		xargs -0 -P "$jobs" -n 1 \
			"$script_dir/metamorphic-test.sh" --_worker \
			"$work_dir" "$normalized_profiles_csv" "$channel" "$keep"
fi

# ---------------------------------------------------------------------------
# Aggregate results and print summary matrix
# ---------------------------------------------------------------------------

elapsed=$(($(date +%s) - start_ts))
total_fixtures=${#fixtures[@]}
failed_fixtures=0
total_failures=0
failure_report="$work_dir/failures.tsv"
: >"$failure_report"

for name in "${fixtures[@]}"; do
	rfile="$work_dir/results/$name.tsv"
	if [ ! -f "$rfile" ]; then
		failed_fixtures=$((failed_fixtures + 1))
		total_failures=$((total_failures + 1))
		printf '%s\tALL\tFAIL\tmissing-worker-result\n' "$name" >>"$failure_report"
		continue
	fi
	f_fail=0
	while IFS=$'\t' read -r r_name r_prof r_status r_detail; do
		if [ "$r_status" = "FAIL" ]; then
			f_fail=1
			total_failures=$((total_failures + 1))
			printf '%s\t%s\t%s\t%s\n' "$r_name" "$r_prof" "$r_status" "$r_detail" >>"$failure_report"
		fi
	done <"$rfile"
	if [ "$f_fail" -ne 0 ]; then
		failed_fixtures=$((failed_fixtures + 1))
	fi
done

echo
echo "========================================================================"
printf '%-26s %8s %8s %8s\n' "PROFILE" "PASSED" "FAILED" "SKIPPED"
echo "------------------------------------------------------------------------"
for p in "${selected_profiles[@]}"; do
	p_pass=0
	p_fail=0
	p_skip=0
	for name in "${fixtures[@]}"; do
		rfile="$work_dir/results/$name.tsv"
		[ -f "$rfile" ] || continue
		while IFS=$'\t' read -r _ r_prof r_status _; do
			[ "$r_prof" = "$p" ] || continue
			case "$r_status" in
			PASS) p_pass=$((p_pass + 1)) ;;
			FAIL) p_fail=$((p_fail + 1)) ;;
			SKIP) p_skip=$((p_skip + 1)) ;;
			esac
		done <"$rfile"
	done
	printf '%-26s %8d %8d %8d\n' "$p" "$p_pass" "$p_fail" "$p_skip"
done
echo "------------------------------------------------------------------------"
printf 'fixtures: %d total, %d passed, %d failed (elapsed: %ds)\n' \
	"$total_fixtures" "$((total_fixtures - failed_fixtures))" "$failed_fixtures" "$elapsed"
echo "========================================================================"

if [ "$total_failures" -ne 0 ]; then
	echo
	echo "FAILURES ($total_failures profile failure(s) across $failed_fixtures fixture(s)):"
	while IFS=$'\t' read -r f_name f_prof _ f_detail; do
		printf '  - %-40s %-25s %s\n' "$f_name" "$f_prof" "$f_detail"
		for dfile in "$work_dir/diffs/$f_name.$f_prof."*.diff; do
			[ -f "$dfile" ] || continue
			echo "      diff: $dfile"
			sed 's/^/        /' "$dfile" | head -n 20
		done
		if [ "$f_detail" = "compile-error" ] && [ -f "$work_dir/runs/$f_name/$f_prof/compile.log" ]; then
			echo "      compile log: $work_dir/runs/$f_name/$f_prof/compile.log"
			sed 's/^/        /' "$work_dir/runs/$f_name/$f_prof/compile.log" | tail -n 15
		fi
		if [ -s "$work_dir/runs/$f_name/$f_prof/stderr" ]; then
			echo "      stderr: $work_dir/runs/$f_name/$f_prof/stderr"
			sed 's/^/        /' "$work_dir/runs/$f_name/$f_prof/stderr" | head -n 10
		fi
	done <"$failure_report"
	echo
	echo "Logs and diffs preserved in: $work_dir"
	exit 1
fi
