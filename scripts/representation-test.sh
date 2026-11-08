#!/usr/bin/env bash
# Verifies report-only heap analysis without changing emitted JavaScript.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/33_heap_representation.rs"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	"$script_dir/build.sh"
fi

"$script_dir/compile_core.sh" "$fixture" "$work/off.js" >/dev/null

# `js-opt=report` is emit-affecting, so it keys its own sysroot. Build it rather than assuming one
# is cached: every backend edit moves the key, and a fresh checkout has neither (`sroa-test.sh`
# does the same for the same reason).
JS_EXTRA_ARGS="js-opt=report" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null

JS_EXTRA_ARGS="js-opt=report js-opt-warnings=off js-opt-report=$work/report.json js-workspace-root=$root" \
	"$script_dir/compile_core.sh" "$fixture" "$work/report.js" >/dev/null

cmp "$work/off.js" "$work/report.js"
grep -q '"pass": "heap-representation"' "$work/report.json"
grep -q 'scalar-fields-local-' "$work/report.json"
grep -q 'raw pointer or byte provenance' "$work/report.json"
grep -q 'length, capacity, allocator, and raw-parts' "$work/report.json"
grep -q 'drop behavior is observable' "$work/report.json"

echo "representation report: PASS"
