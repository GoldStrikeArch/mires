#!/usr/bin/env bash
#
# The W5f raw-provenance whitelist receipt (Phase 5): `85_raw_view_slice` builds one box from a
# fresh allocation the whitelist admits and two boxes whose addresses are compared, which it
# must refuse.
#
# What is asserted:
#   * the fixture prints its `.expected` on the JIR lane in every optimizer mode;
#   * under `js-opt=report` and `js-opt=on` the sidecar shows the whitelisted candidate free of
#     `legality.raw-provenance` (it declines `legality.user-defined-drop`, its true class), and
#     the guarded candidates still declining exactly `legality.raw-provenance`;
#   * the three programs' stdout is byte-identical across the modes.
#
# Under `js-opt=off` no representation planning runs and the sidecar's `repr` is `null`, which
# is why the declines are asserted on the two modes that plan.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture=85_raw_view_slice
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
const decisions = sidecar.repr?.decisions ?? [];
const of = (name) => decisions.filter((d) => d.subject.includes(name));

const whitelisted = of("whitelisted_box_view");
if (whitelisted.length === 0) {
  throw new Error(`${mode}: the whitelisted box is not a candidate: ${JSON.stringify(decisions)}`);
}
if (whitelisted.some((d) => d.decline === "legality.raw-provenance")) {
  throw new Error(`${mode}: the whitelisted box still declines raw-provenance: ${JSON.stringify(whitelisted)}`);
}
if (!whitelisted.some((d) => d.decline === "legality.user-defined-drop")) {
  throw new Error(`${mode}: the whitelisted box does not decline its true class: ${JSON.stringify(whitelisted)}`);
}

const guarded = of("guarded_box_views").filter((d) => d.decline === "legality.raw-provenance");
if (guarded.length < 2) {
  throw new Error(`${mode}: the compared allocations lost their raw-provenance pin: ${JSON.stringify(of("guarded_box_views"))}`);
}
if (guarded.some((d) => d.chosen !== "baseline")) {
  throw new Error(`${mode}: a raw-provenance value left the baseline: ${JSON.stringify(guarded)}`);
}
console.log(`  ${mode}: whitelisted box relabeled, ${guarded.length} guarded value(s) still legality.raw-provenance`);
NODE
done

cmp "$work/off.out" "$work/report.out"
cmp "$work/off.out" "$work/on.out"
cmp "$work/off.out" "$root/examples/core-tests/$fixture.expected"

echo "raw-view-test: OK"
