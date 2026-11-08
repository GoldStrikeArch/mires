#!/usr/bin/env bash
# Zero-sized and scalar-constant captures across suspension, error, and cancellation.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/async-action.XXXXXX")
echo "async-action: artifacts at $work"

for minify in off on; do
	for mode in off report on; do
		args="js-opt=$mode js-minify=$minify js-source-map=on js-opt-warnings=off"
		JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$minify-$mode.log" 2>&1
		for lane in baseline jir; do
			for mir in off on; do
				out="$work/$lane/$mir/$minify/$mode/program.js"
				mkdir -p "$(dirname -- "$out")"
				RCGJS_WITNESS_CHECK=1 JSC_RUST_MIR_INLINE="$mir" JS_EXTRA_ARGS="$args js-lane=$lane" \
					"$script_dir/compile_core.sh" "$root/examples/core-tests/109_async_zero_sized_action.rs" \
					"$out" >"$out.compile.log" 2>&1
				cat "$root/runtime/shim.js" "$out" "$script_dir/entry.js" | node - >"$out.stdout"
				diff -u "$root/examples/core-tests/109_async_zero_sized_action.expected" "$out.stdout"
				node "$script_dir/check-sourcemap.js" "$out" >"$out.map-check.log"
				if [ "$lane" = jir ]; then
					node --input-type=module - "$out.lane.json" <<'JS'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
const report = JSON.parse(readFileSync(process.argv[2], 'utf8'))
assert.equal(report.fallback_count, 0, process.argv[2])
assert.equal(report.witnesses.failed, 0, process.argv[2])
JS
				fi
				if [ "$mode" = report ]; then
					cmp "$work/$lane/$mir/$minify/off/program.js" "$out"
				fi
			done
		done
	done
done
echo "async-action: PASS (24 artifacts, exact traces, zero JIR fallback, maps, off/report bytes)"
