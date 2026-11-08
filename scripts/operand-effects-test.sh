#!/usr/bin/env bash
# Exact operand values, ordered host effects, and throw identity across both compiler lanes.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/operand-effects.XXXXXX")
echo "operand-effects: artifacts and observations at $work"

for minify in off on; do
	for mode in off report on; do
		export JS_EXTRA_ARGS="js-opt=$mode js-minify=$minify js-source-map=on js-opt-warnings=off"
		SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$minify-$mode.log" 2>&1
		for lane in baseline jir; do
			for mir in off on; do
				out="$work/$lane/$mir/$minify/$mode/fixture.js"
				mkdir -p "$(dirname -- "$out")"
				JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane" JSC_RUST_MIR_INLINE="$mir" \
					"$script_dir/compile_core.sh" "$root/examples/core-tests/106_operand_effects.rs" \
					"$out" >"$out.compile.log" 2>&1
				RCGJS_EXPECT_LANE="$lane" node "$script_dir/operand-effects.mjs" "$out" | tee "$out.effects.json"
				if [ "$mode" = report ]; then
					cmp "$work/$lane/$mir/$minify/off/fixture.js" "$out"
				fi
			done
		done
	done
done
echo "operand-effects: PASS (24 artifacts, exact off/report bytes)"
