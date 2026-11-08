#!/usr/bin/env bash
# Compiled NativeBox and PEA composition, including owned moves and structural cleanup.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for mode in off report on; do
	args="js-lane=jir js-opt=$mode js-opt-warnings=off"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	RCGJS_WITNESS_CHECK=1 JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
		"$root/examples/core-tests/107_box_pea_composition.rs" "$work/$mode.js" >/dev/null
done

args='js-lane=jir js-opt=on js-opt-warnings=off'
RCGJS_WITNESS_CHECK=1 RCGJS_JIR_PASSES='constprop,inline,devirt,range,dce,slice-iter,host-args' \
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/core-tests/107_box_pea_composition.rs" "$work/without-pea.js" >/dev/null
RCGJS_WITNESS_CHECK=1 JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/core-tests/107_box_pea_composition.rs" "$work/repeat.js" >/dev/null
RCGJS_WITNESS_CHECK=1 JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/core-tests/107_box_pea_composition.rs" "$work/replace.js" --cfg box_pea_replace >/dev/null
RCGJS_WITNESS_CHECK=1 RCGJS_JIR_PASSES='constprop,inline,devirt,range,dce,slice-iter,host-args' \
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/core-tests/107_box_pea_composition.rs" "$work/replace-without-pea.js" --cfg box_pea_replace >/dev/null

cmp "$work/off.js" "$work/report.js"
cmp "$work/on.js" "$work/repeat.js"
cmp "$work/on.js.lane.json" "$work/repeat.js.lane.json"

node "$script_dir/carrier-pea-test.mjs" "$work" "$root/runtime/shim.js"
cat "$root/runtime/shim.js" "$work/on.js" "$script_dir/entry.js" | node - |
	diff -u "$root/examples/core-tests/107_box_pea_composition.expected" -
