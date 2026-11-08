#!/usr/bin/env bash
#
# The custom-allocator guard (Phase 5, D10f): `guard_custom_allocator` builds a
# `Vec<i32, Arena>` whose allocator type is not `Global`, and a custom allocator can observe
# every allocation, so the value may never leave the baseline representation.
#
# What is asserted:
#   * the fixture prints its `.expected` on the JIR lane in every optimizer mode — a
#     representation decline is a planning decision, never a behavior change;
#   * under `js-opt=report` and `js-opt=on` the sidecar records the region as a candidate and
#     declines it `legality.custom-allocator` (`InfReason::CustomAllocator` through
#     `plan/representation.rs`'s decline path), committing nothing for it;
#   * the three programs' stdout is byte-identical across the modes.
#
# Under `js-opt=off` no representation planning runs and the sidecar's `repr` is `null`, which
# is why the decline is asserted on the two modes that plan.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture=guard_custom_allocator
out="$root/build/core/$fixture.js"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for opt in off report on; do
	JS_EXTRA_ARGS="js-opt=$opt" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="js-lane=jir js-opt=$opt" SKIP_BUILD=1 "$script_dir/test.sh" "$fixture" \
		>"$work/$opt.log" 2>&1 || { cat "$work/$opt.log" >&2 && exit 1; }
	cat "$root/runtime/shim.js" "$out" "$script_dir/entry.js" | node - >"$work/$opt.out"
	if [ "$opt" = off ]; then
		continue
	fi
	node - "$out.lane.json" "$opt" <<'NODE'
const sidecar = JSON.parse(require("node:fs").readFileSync(process.argv[2], "utf8"));
const mode = process.argv[3];
const repr = sidecar.repr;
if (!repr || repr.planned.candidates < 1) {
  throw new Error(`${mode}: the custom-allocator vec is not even a candidate: ${JSON.stringify(repr)}`);
}
const declined = (repr.decisions ?? []).filter((d) => d.decline === "legality.custom-allocator");
if (declined.length === 0) {
  throw new Error(`${mode}: no decision declines legality.custom-allocator: ${JSON.stringify(repr.decisions)}`);
}
if (declined.some((d) => d.chosen !== "baseline")) {
  throw new Error(`${mode}: a custom-allocator value left the baseline: ${JSON.stringify(declined)}`);
}
console.log(`  ${mode}: ${declined.length} value(s) declined legality.custom-allocator, all baseline`);
NODE
done

cmp "$work/off.out" "$work/report.out"
cmp "$work/off.out" "$work/on.out"
cmp "$work/off.out" "$root/examples/core-tests/$fixture.expected"

echo "allocator guard: PASS ($fixture stays baseline with legality.custom-allocator in every planning mode, stdout identical)"
