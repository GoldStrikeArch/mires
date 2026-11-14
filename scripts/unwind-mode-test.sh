#!/usr/bin/env bash
# The unwind build mode's own fixtures (examples/unwind-tests/), in the mode JSC_PANIC selects:
# run it once unset and once with JSC_PANIC=unwind. Each fixture compiles on the JIR lane under
# js-profile=release-balanced in optimizer modes off and on (and with whatever JSC_RUST_OPT_LEVEL /
# JSC_RUST_MIR_INLINE the caller sets), then runs normally and with the host throwing
# (scripts/unwind-mode-check.mjs): the guard must drop and the thrown value must leave unchanged.
#
#   290  a foreign import declared extern "C". Unwind mode: refused, and the message names
#        `extern "C-unwind"`; nothing is linked. Abort mode: compiles, and js-cleanup keeps the drop.
#   291  the same import declared extern "C-unwind": both modes keep the drop.
#   292  an exported `extern "C" fn` holding a guard, whose cleanup path ends in
#        UnwindTerminate(Abi) under -Cpanic=unwind: both modes keep the drop.
#
# Every result is retained. SKIP_BUILD=1 reuses the backend.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"
panic=$(jsc_panic) || exit 2
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/unwind-mode.XXXXXX")
echo "unwind mode tests ($panic): retained output at $work"
failures=0
for mode in off on; do
	export JS_EXTRA_ARGS="js-profile=release-balanced js-opt=$mode"
	SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
	for source in "$root"/examples/unwind-tests/*.rs; do
		name=$(basename -- "$source" .rs)
		program="$work/$name-$mode.js"
		compiled=1
		JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=jir" "$script_dir/compile_core.sh" "$source" "$program" \
			>"$program.compile.log" 2>&1 || compiled=0
		if [ "$panic" = unwind ] && [ "$name" = 290_extern_c_import_rejected ]; then
			if [ "$compiled" = 0 ] && [ ! -e "$program" ] &&
				grep -qF 'is a foreign import with the "C" ABI, which cannot unwind' "$program.compile.log" &&
				grep -qF 'Declare it `extern "C-unwind"`' "$program.compile.log"; then
				echo "$panic/$mode $name: refused as expected"
			else
				echo "$panic/$mode $name: FAILED, expected the refusal naming extern \"C-unwind\"; see $program.compile.log" >&2
				failures=$((failures + 1))
			fi
			continue
		fi
		if [ "$compiled" = 0 ]; then
			echo "$panic/$mode $name: FAILED to compile; see $program.compile.log" >&2
			failures=$((failures + 1))
			continue
		fi
		if node "$script_dir/unwind-mode-check.mjs" "$program" "$root/runtime/shim.js" "$name" \
			>"$program.result.json" 2>"$program.result.log"; then
			echo "$panic/$mode $name: normal and host-throw rows passed"
		else
			echo "$panic/$mode $name: FAILED; see $program.result.json" >&2
			failures=$((failures + 1))
		fi
	done
done
test "$failures" = 0
