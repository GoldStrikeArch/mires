#!/usr/bin/env bash
#
# Closures created inside `for` loop bodies and handed to the host (examples/host_loop_callbacks.rs;
# CONTRACT.md, "A thunk built inside a loop snapshots its environment").
#
#   scripts/host-loop-callbacks-test.sh        # SKIP_BUILD=1 reuses the backend
#
# Two builds, each in optimizer modes off, report and on, and off and report must emit identical
# JavaScript in each:
#
#   release   js-profile=release-balanced (so js-cleanup=on, the JIR lane). `callback0` is not
#             inlined here, so the marker runs in the callee's own frame and each call snapshots by
#             being a call. scripts/host-loop-callbacks-check.mjs checks behaviour and that no
#             function created on a loop escapes over a binding the loop reuses.
#   inlined   JSC_RUST_MIR_INLINE=on, cleanup off, both lanes. rustc inlines `callback0` into the
#             loop body, so the marker sits on the loop itself: the case examples/dom-tests/
#             23_for_row_handlers pinned for the DOM `handler` marker. The check additionally
#             requires the contract's immediately invoked wrapper at every loop site (--snapshot).
#
# Every artifact is retained under build/host-loop-callbacks.XXXXXX.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/host-loop-callbacks.XXXXXX")
echo "host loop callbacks: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro --locked) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
mkdir -p "$work/proc-macro"
ln -s "$macro" "$work/proc-macro/$(basename -- "$macro")"
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"

# build <name> <mode> <emit args> <lanes> <check flag> [env assignment...]
build() {
	local name=$1 mode=$2 args=$3 lanes=$4 flag=$5
	shift 5
	local library="$work/$name-$mode-lib"
	mkdir -p "$library"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$library/sysroot.log" 2>&1
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$library/libmires_js.rlib" \
		--crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$library/compile.log" 2>&1
	local lane program
	for lane in $lanes; do
		mkdir -p "$work/$name-$lane-$mode"
		program="$work/$name-$lane-$mode/program.mjs"
		env "$@" JS_EXTRA_ARGS="$args js-lane=$lane js-modules=esm js-shim-module=../shim.mjs js-source-map=on" \
			"$script_dir/compile_core.sh" "$root/examples/host_loop_callbacks.rs" "$program" \
			--extern "mires_js=$library/libmires_js.rlib" --search "$work/proc-macro" >"$program.compile.log" 2>&1
		printf '%s/%s/%s: ' "$name" "$lane" "$mode"
		node "$script_dir/host-loop-callbacks-check.mjs" "$program" $flag
		node "$script_dir/check-sourcemap.js" "$program" >"$program.map.log"
	done
}

for mode in off report on; do
	build release "$mode" "js-profile=release-balanced js-opt=$mode" jir ""
	build inlined "$mode" "js-opt=$mode" "baseline jir" --snapshot JSC_RUST_MIR_INLINE=on
done
for pair in release-jir inlined-baseline inlined-jir; do
	cmp "$work/$pair-off/program.mjs" "$work/$pair-report/program.mjs"
done
echo 'host loop callbacks: PASS (release jir, inlined baseline and jir; off, report, on; off == report)'
