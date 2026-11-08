#!/usr/bin/env bash
# Shared DOM traces, handler captures, zero JIR fallback, and exact off/report output.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
cd "$root"
if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi
work=$(mktemp -d "$root/build/dom-matrix.XXXXXX")
echo "dom-matrix: artifacts at $work"
args='js-names=readable js-comments=on js-modules=esm js-dom-module=./topcoat-dom.js'
for mode in off report on; do
	extra=""
	if [ "$mode" != off ]; then extra=" js-opt=$mode"; fi
	JS_EXTRA_ARGS="$args$extra" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
	for lane in baseline jir; do
		for mir in off on; do
			out="$work/$lane/$mir/$mode"
			mkdir -p "$out"
			DOM_OPT_MODE="$mode" JSC_RUST_MIR_INLINE="$mir" JS_EXTRA_ARGS="js-lane=$lane" \
				SKIP_BUILD=1 "$script_dir/dom-test.sh" >"$out/test.log" 2>&1
			cp build/domtest/*.js build/domtest/*.trace "$out/"
			if [ "$lane" = jir ]; then
				cp build/domtest/*.lane.json "$out/"
				node --input-type=module - "$out" <<'JS'
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
const root = process.argv[2];
const reports = readdirSync(root).filter(name => name.endsWith('.lane.json'));
assert.equal(reports.length, 25);
for (const name of reports) {
  assert.equal(JSON.parse(readFileSync(`${root}/${name}`)).fallback_count, 0, name);
}
JS
			fi
			if [ "$mode" = report ]; then
				for program in "$out"/*.js; do
					cmp "$work/$lane/$mir/off/$(basename -- "$program")" "$program"
				done
			fi
			echo "PASS: $lane / MIR $mir / $mode"
		done
	done
done
echo 'dom-matrix: PASS (12 configurations, 25 fixtures each)'
