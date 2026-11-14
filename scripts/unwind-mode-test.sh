#!/usr/bin/env bash
# The unwind build mode's own fixtures (examples/unwind-tests/), in the mode JSC_PANIC selects:
# run it once unset (`unwind` default) and once with `JSC_PANIC=abort`. Each fixture compiles on the JIR lane under
# js-profile=release-balanced in optimizer modes off and on, in two settings (rustc's defaults, and
# JSC_RUST_MIR_INLINE=on JSC_RUST_OPT_LEVEL=z), then runs normally and with the host throwing
# (scripts/unwind-mode-check.mjs): the guard must drop and the thrown value must leave unchanged.
#
#   290  a foreign import declared extern "C".
#   291  the same import declared extern "C-unwind": both modes keep the drop.
#   292  an exported `extern "C" fn` holding a guard, whose cleanup path ends in
#        UnwindTerminate(Abi) under -Cpanic=unwind: both modes keep the drop.
#   293  a Rust-defined `extern "C" fn` called from a guard-holding Rust frame.
#   294  the same `extern "C" fn` marked #[inline(always)], which rustc inlines in the second
#        setting, leaving its UnwindTerminate(Abi) edge in the caller.
#   295  an `extern "C" fn` reached through an `extern "C" fn` pointer and as a `dyn` method.
#   296  a foreign import declared extern "cdecl".
#   297  an extern "C" import called only through a function pointer.
#   298  an extern "C" import declared in a dependency compiled as `--crate-name compiler_builtins`.
#   299  a dependency's non-generic `extern "C" fn`.
#
# 290 and 293-299 lose a destructor on a host exception if they build in unwind mode, so there each
# must be refused, with messages that contain every fragment `refusal` lists (each names the fix,
# extern "C-unwind"), and nothing may be linked. In abort mode every fixture compiles and
# js-cleanup keeps the drop. A fixture that `dependency` names is compiled against that crate,
# built from examples/unwind-tests/deps/ first.
#
# Every result is retained. SKIP_BUILD=1 reuses the backend.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"
panic=$(jsc_panic) || exit 2
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi

# The fragments the unwind-mode refusal of fixture $1 must contain in setting $2, one per line;
# nothing for a fixture unwind mode builds.
refusal() {
	local fix='Declare it `extern "C-unwind"`'
	case "$1" in
	290_extern_c_import_rejected)
		printf '%s\n' 'is a foreign import with the "C" ABI, which cannot unwind' "$fix"
		;;
	293_extern_c_fn_called_from_rust | 299_extern_c_fn_of_dependency)
		local path=middle
		[ "$1" = 299_extern_c_fn_of_dependency ] && path=unwind_dep::middle
		printf '%s\n' "\`$path\` has the \"C\" ABI, which cannot unwind" "$fix"
		;;
	294_extern_c_fn_inlined)
		printf '%s\n' '`middle` has the "C" ABI, which cannot unwind' "$fix"
		if [ "$2" = inline-z ]; then printf '%s\n' 'rustc inlined it here'; fi
		;;
	295_extern_c_fn_pointer_and_dyn)
		printf '%s\n' 'this call goes through a `extern "C" fn(bool)`, whose "C" ABI cannot unwind' \
			'Use an `extern "C-unwind"` function pointer type' \
			'this call of the `dyn` method `Act::act` has the "C" ABI, which cannot unwind' \
			'Declare the method `extern "C-unwind"`'
		;;
	296_cdecl_import_rejected)
		printf '%s\n' 'is a foreign import with the "cdecl" ABI, which cannot unwind' "$fix"
		;;
	297_extern_c_import_through_pointer)
		printf '%s\n' 'this call goes through a `unsafe extern "C" fn(f64)`, whose "C" ABI cannot unwind' \
			'Use an `extern "C-unwind"` function pointer type'
		;;
	298_import_from_sysroot_named_crate)
		printf '%s\n' '`compiler_builtins::host_note` is a foreign import with the "C" ABI, which cannot unwind' "$fix"
		;;
	esac
}

# The dependency fixture $1 is compiled against: <source in deps/> <crate name> <extern name>.
dependency() {
	case "$1" in
	298_import_from_sysroot_named_crate) echo "298_dep.rs compiler_builtins sysroot_named" ;;
	299_extern_c_fn_of_dependency) echo "299_dep.rs unwind_dep unwind_dep" ;;
	esac
}

mkdir -p "$root/build"
work=$(mktemp -d "$root/build/unwind-mode.XXXXXX")
echo "unwind mode tests ($panic): retained output at $work"
failures=0
for setting in defaults inline-z; do
	if [ "$setting" = inline-z ]; then
		export JSC_RUST_MIR_INLINE=on JSC_RUST_OPT_LEVEL=z
	else
		unset JSC_RUST_MIR_INLINE JSC_RUST_OPT_LEVEL
	fi
	for mode in off on; do
		export JS_EXTRA_ARGS="js-profile=release-balanced js-opt=$mode"
		SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$setting-$mode.log" 2>&1
		for source in "$root"/examples/unwind-tests/*.rs; do
			name=$(basename -- "$source" .rs)
			tag="$panic/$setting/$mode $name"
			program="$work/$name-$setting-$mode.js"
			externs=()
			compiled=1
			read -r dep_source dep_crate dep_name <<<"$(dependency "$name")" || true
			if [ -n "${dep_source:-}" ]; then
				dep_dir="$work/deps-$name-$setting-$mode"
				mkdir -p "$dep_dir"
				JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=jir" "$script_dir/compile_core.sh" \
					"$root/examples/unwind-tests/deps/$dep_source" "$dep_dir/lib$dep_crate.rlib" \
					--crate-type rlib --crate-name "$dep_crate" >"$program.dep.log" 2>&1 || {
					echo "$tag: FAILED to compile its dependency; see $program.dep.log" >&2
					failures=$((failures + 1))
					continue
				}
				externs=(--extern "$dep_name=$dep_dir/lib$dep_crate.rlib")
			fi
			JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=jir" "$script_dir/compile_core.sh" "$source" "$program" \
				${externs[@]+"${externs[@]}"} >"$program.compile.log" 2>&1 || compiled=0
			expected=$(refusal "$name" "$setting")
			if [ "$panic" = unwind ] && [ -n "$expected" ]; then
				missing=""
				while IFS= read -r fragment; do
					grep -qF -- "$fragment" "$program.compile.log" || missing="$missing [$fragment]"
				done <<<"$expected"
				if [ "$compiled" = 0 ] && [ ! -e "$program" ] && [ -z "$missing" ]; then
					echo "$tag: refused as expected"
				else
					echo "$tag: FAILED, expected the refusal naming extern \"C-unwind\"${missing:+ (missing:$missing)}; see $program.compile.log" >&2
					failures=$((failures + 1))
				fi
				continue
			fi
			if [ "$compiled" = 0 ]; then
				echo "$tag: FAILED to compile; see $program.compile.log" >&2
				failures=$((failures + 1))
				continue
			fi
			if node "$script_dir/unwind-mode-check.mjs" "$program" "$root/runtime/shim.js" "$name" \
				>"$program.result.json" 2>"$program.result.log"; then
				echo "$tag: normal and host-throw rows passed"
			else
				echo "$tag: FAILED; see $program.result.json" >&2
				failures=$((failures + 1))
			fi
		done
	done
done
test "$failures" = 0
