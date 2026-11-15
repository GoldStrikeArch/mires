#!/usr/bin/env bash
# Checks private `buf, off, len` slice lanes, range proofs, and canonical wrappers.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture=97_slice_lanes
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for mode in off report on; do
	args="js-lane=jir js-opt=$mode"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JSC_RUST_MIR_INLINE=off JS_EXTRA_ARGS="$args" \
		"$script_dir/compile_core.sh" "$root/examples/core-tests/$fixture.rs" \
		"$work/$fixture-$mode.js" >/dev/null
done

map_args='js-lane=jir js-opt=on js-source-map=on'
JSC_RUST_MIR_INLINE=off JS_EXTRA_ARGS="$map_args" \
	"$script_dir/compile_core.sh" "$root/examples/core-tests/$fixture.rs" \
	"$work/$fixture-on-map.js" >/dev/null

cmp "$work/$fixture-off.js" "$work/$fixture-report.js"
for mode in on on-map; do
	cat "$root/runtime/shim.js" "$work/$fixture-$mode.js" "$script_dir/entry.js" | node - | \
		diff -u "$root/examples/core-tests/$fixture.expected" -
done
node "$script_dir/check-sourcemap.js" "$work/$fixture-on-map.js" "$fixture.rs" >/dev/null

jq -e '
  .abi.applied == true and
  (.abi.flat | length) == 0 and
  (.physical.selected_calls | length) == 0 and
  any(.physical.decisions[];
    .item == "slice_fold" and
    .shape == "slice(3)" and
    .selected == false and
    .reason == "cost.score-no-improvement" and
    .after.score > .before.score) and
  .range.asserts_removed >= 6 and
  .witnesses.failed == 0
' "$work/$fixture-on.js.lane.json" >/dev/null
jq -e '
  .abi.applied == false and
  (.abi.flat | length) == 0 and
  (.physical.selected_calls | length) == 0 and
  any(.physical.decisions[];
    .item == "slice_fold" and
    .shape == "slice(3)" and
    .selected == false and
    .reason == "cost.score-no-improvement" and
    .after.score > .before.score)
' "$work/$fixture-report.js.lane.json" >/dev/null

# The new subslice, aligned-prefix, and loop-length facts do more than fold the local compares:
# together they make this exact panic/location/rem-by-zero subgraph unreachable. The generic
# bounds/panic formatter remains live for the deliberately unproved accesses elsewhere in the
# fixture, so gate the subgraph that this slice actually owns instead of claiming the entire
# panic runtime is gone.
dead_cluster='[
  "$panic$location$impl_6_file$",
  "$panic$location$impl_6_line$",
  "$panic$panic_info$impl_0_location$",
  "$panic_const$panic_const_rem_by_zero$"
]'
jq -e --argjson dead "$dead_cluster" '
  . as $report |
  all($dead[]; . as $needle | any($report.items[]; .name | contains($needle)))
' "$work/$fixture-off.js.lane.json" >/dev/null
jq -e --argjson dead "$dead_cluster" '
  . as $report |
  all($dead[]; . as $needle | all($report.items[]; .name | contains($needle) | not))
' "$work/$fixture-on.js.lane.json" >/dev/null

node - "$work/$fixture-on.js" "$root/runtime/shim.js" <<'NODE'
const fs = require('node:fs')
const vm = require('node:vm')

const context = vm.createContext({ TextDecoder, TextEncoder })
vm.runInContext(fs.readFileSync(process.argv[3], 'utf8'), context)
vm.runInContext(fs.readFileSync(process.argv[2], 'utf8'), context)
const value = context.slice_fold({
  buf: [1, 0x7f, 0x80, 0xff, 0x10, 0x20, 0x30, 0x40, 0xaa, 0xbb, 0xcc],
  off: 0,
  len: 11,
})
if (value !== 2504373805) {
  throw new Error(`canonical exported slice ABI returned ${value}`)
}
NODE

echo "slice planning + range proofs: PASS (regressive physical plan declined; six checks removed)"
