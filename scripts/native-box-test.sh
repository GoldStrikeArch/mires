#!/usr/bin/env bash
# Checks the ownership-only native Box carrier and its compiler drop-glue boundary.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixtures=(
	89_repr_box_drop
	90_repr_box_structural_drop
	94_repr_box_nested
	95_repr_box_mixed_callers
	96_repr_box_abrupt_exit
	107_box_pea_composition
)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for mode in off report on; do
	args="js-lane=jir js-opt=$mode js-opt-passes=native-representations"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	for fixture in "${fixtures[@]}"; do
		JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
			"$root/examples/core-tests/$fixture.rs" "$work/$fixture-$mode.js" >/dev/null
	done
done

node - "$work/96_repr_box_abrupt_exit-on.js" "$root/runtime/shim.js" <<'NODE'
const fs = require('node:fs')
const vm = require('node:vm')

const context = vm.createContext({ TextDecoder, TextEncoder })
vm.runInContext(fs.readFileSync(process.argv[3], 'utf8'), context)
const events = []
context.__rt.js_log_f64 = (value) => events.push(Number(value))
const marker = new Error('host marker')
context.__rt.host_throw = () => { throw marker }
vm.runInContext(fs.readFileSync(process.argv[2], 'utf8'), context)

events.length = 0
let panicCaught = false
try {
  context.box_exit(1)
} catch {
  panicCaught = true
}
if (!panicCaught || events.includes(10) || events.includes(20)) {
  throw new Error(`panic-abort ran Box cleanup or did not abort: ${JSON.stringify(events)}`)
}

events.length = 0
let hostCaught = false
try {
  context.box_exit(2)
} catch (error) {
  hostCaught = error === marker
}
if (!hostCaught || events.includes(10) || events.includes(20)) {
  throw new Error(`host throw changed abrupt-exit cleanup: ${JSON.stringify(events)}`)
}
NODE

for fixture in "${fixtures[@]}"; do
	cmp "$work/$fixture-off.js" "$work/$fixture-report.js"
	cat "$root/runtime/shim.js" "$work/$fixture-on.js" "$script_dir/entry.js" | node - | \
		diff -u "$root/examples/core-tests/$fixture.expected" -
	if [ "$fixture" = 94_repr_box_nested ]; then
		# The outer plain Box is eligible, while the user-drop and Vec-owned nested boxes must
		# stay canonical. This fixture is a conservative-boundary receipt, not a shrink receipt.
		jq -e '
      any(.repr.decisions[];
        (.alternatives | index("native-box")) and
        .chosen == "baseline" and
        (.decline | startswith("legality.")))
    ' "$work/$fixture-on.js.lane.json" >/dev/null
	else
		jq -e '
      .repr.validated.would_commit >= 1 and
      .repr.final_emitted.applied >= 1 and
      any(.repr.decisions[]; .chosen == "native-box")
    ' "$work/$fixture-on.js.lane.json" >/dev/null
	fi
done

off_bytes=$(wc -c <"$work/89_repr_box_drop-off.js" | tr -d ' ')
on_bytes=$(wc -c <"$work/89_repr_box_drop-on.js" | tr -d ' ')
if ((on_bytes >= off_bytes)); then
	echo "native Box lowering did not shrink: $off_bytes -> $on_bytes" >&2
	exit 1
fi

target=5767
high=$((target * 102 / 100))
if ((on_bytes > high)); then
	echo "native Box lowering regressed: $on_bytes > $high bytes (receipt $target +2%)" >&2
	exit 1
fi

echo "native Box lowering (plain and structural drop): $off_bytes -> $on_bytes bytes"
