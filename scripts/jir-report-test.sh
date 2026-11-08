#!/usr/bin/env bash
#
# `js-opt=report` on the JIR lane: says what `js-opt=on` would do, prints what `js-opt=off` prints.
#
# The passes run over a clone of each function and the rewrite is dropped (`opt/pipeline.rs`), and
# the flat direct-call ABI plan is made, reported and then emptied before anything is rewritten
# (`link/lane.rs::run`). So the two claims here are one each: `report`'s program is byte for byte
# `off`'s, and its sidecar's `passes.would_apply` is nonzero while `passes.applied` is all zero.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi
for mode in off report; do
	JS_EXTRA_ARGS="js-opt=$mode" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
done

for fixture in 29_rc 60_virtual_struct_merge; do
	for mode in off report; do
		JS_EXTRA_ARGS="js-lane=jir js-opt=$mode js-opt-warnings=off" SKIP_BUILD=1 \
			"$script_dir/compile_core.sh" "$root/examples/core-tests/$fixture.rs" \
			"$work/$fixture-$mode.js" >/dev/null
	done
	cmp "$work/$fixture-off.js" "$work/$fixture-report.js"
	node - "$work/$fixture-report.js.lane.json" "$work/$fixture-off.js.lane.json" <<'NODE'
const fs = require("node:fs");
const [report, off] = process.argv.slice(2).map((p) => JSON.parse(fs.readFileSync(p, "utf8")));
const zero = (counts) => Object.values(counts).every((n) => n === 0);
if (!zero(report.passes.applied)) {
  throw new Error(`report applied a pass: ${JSON.stringify(report.passes.applied)}`);
}
if (zero(report.passes.would_apply)) {
  throw new Error("report measured nothing, so the byte identity proves nothing");
}
if (report.abi && report.abi.applied !== false) {
  throw new Error(`report applied the flat ABI plan: ${JSON.stringify(report.abi.applied)}`);
}
if (!zero(off.passes.would_apply) || off.abi !== null) {
  throw new Error(`off measured something: ${JSON.stringify(off.passes)}`);
}
console.log(`  would_apply ${JSON.stringify(report.passes.would_apply)}, applied none`);
NODE
	echo "js-opt=report on the jir lane ($fixture): PASS (off-identical bytes)"
done

# Re-reach after an accepted lowering changed an item's references: under `js-opt=on` fixture 69's
# `pick` folds the `None` arm, the only reference to `cold`, and `cold` must leave the program.
JS_EXTRA_ARGS="js-opt=on" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
JS_EXTRA_ARGS="js-lane=jir js-opt=on js-opt-warnings=off" SKIP_BUILD=1 \
	"$script_dir/compile_core.sh" "$root/examples/core-tests/69_dead_call_dropped.rs" \
	"$work/69-on.js" >/dev/null
if grep -q 'cold\$h' "$work/69-on.js"; then
	echo "jir-report-test: 69_dead_call_dropped still contains \`cold\` on the jir lane under js-opt=on" >&2
	exit 1
fi
echo "jir-report-test: ok"
