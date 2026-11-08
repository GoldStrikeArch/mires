#!/usr/bin/env bash
# Checks nested native values, their complete structural drop closure, and own-Drop fallback.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/44_native_structural.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

# The suite runs on the jir lane (js-lane=jir) by default: W5h deleted the v1 text
# optimizer, so the jir lane is the only lowerer. NATIVE_LANE=baseline checks the
# baseline lane instead, where `js-opt=on` no longer rewrites anything: `on` must
# equal `off` byte for byte.
lane="${NATIVE_LANE:-jir}"

for mode in off report on; do
	args="js-opt=$mode js-opt-passes=native-representations"
	if [ "$lane" = jir ]; then
		args="js-lane=jir $args"
	fi
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="$args js-opt-warnings=off js-opt-report=$work/$mode.json" \
		"$script_dir/compile_core.sh" "$fixture" "$work/$mode.js" >/dev/null
done
cmp "$work/off.js" "$work/report.js"

node - "$work" "$root" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const work = process.argv[2];
const root = process.argv[3];

function exercise(mode) {
  const context = vm.createContext({ TextDecoder, TextEncoder });
  vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
  vm.runInContext(fs.readFileSync(path.join(work, `${mode}.js`), "utf8"), context);

  let store = context.native_structural_make();
  const original = store;
  store = context.native_structural_round_trip(store);
  if (store !== original) {
    throw new Error(`${mode} same-artifact aggregate round trip changed identity`);
  }

  const trace = [];
  const record = () => {
    const rows = [];
    for (let index = 0; index < context.native_structural_len(store); index += 1) {
      rows.push([
        context.native_structural_id(store, index),
        context.native_structural_label(store, index),
      ]);
    }
    trace.push(rows);
  };

  record();
  context.native_structural_push(store, 11, "\u03b2eta");
  record();
  context.native_structural_clear(store);
  record();
  context.native_structural_push(store, 13, "after");
  record();

  const dropped = context.native_structural_dispose(context.native_structural_make());
  const own = context.native_structural_own_drop_make(23);
  const ownValue = context.native_structural_own_drop_value(own);
  const ownConsumed = context.native_structural_own_drop_consume(own);
  return { trace, dropped, ownValue, ownConsumed };
}

const expected = {
  trace: [
    [[7, "alpha"], [9, "\u{1f980} rust"]],
    [[7, "alpha"], [9, "\u{1f980} rust"], [11, "\u03b2eta"]],
    [],
    [[13, "after"]],
  ],
  dropped: 2,
  ownValue: 23,
  ownConsumed: 23,
};
const results = Object.fromEntries(["off", "report", "on"].map((mode) => [mode, exercise(mode)]));
for (const [mode, result] of Object.entries(results)) {
  if (JSON.stringify(result) !== JSON.stringify(expected)) {
    throw new Error(`${mode} structural behavior differs: ${JSON.stringify(result)}`);
  }
}
if (JSON.stringify(results.off) !== JSON.stringify(results.on)) {
  throw new Error("optimized structural behavior differs from baseline");
}

NODE

if [ "$lane" = baseline ]; then
	# The v1 text optimizer is deleted: `js-opt=on` on the baseline lane no longer rewrites
	# anything, so the receipt is byte-frozen -- `on` must equal `off` exactly.
	cmp "$work/off.js" "$work/on.js"
	echo "native structural values (baseline lane): frozen, on == off ($(wc -c <"$work/off.js" | tr -d ' ') bytes)"
else
	off_bytes=$(wc -c <"$work/off.js" | tr -d ' ')
	on_bytes=$(wc -c <"$work/on.js" | tr -d ' ')
	# No shrink budget on the jir lane: the jir planner currently declines this region (the
	# own-Drop fallback shapes pin it), so the check here is the behavioural equality above.
	# v1's 25% receipt died with the text optimizer; a committed region would only shrink this.
	if ((on_bytes > off_bytes)); then
		echo "native structural lowering (jir lane) grew the program: $off_bytes -> $on_bytes" >&2
		exit 1
	fi
	echo "native structural values (jir lane): PASS ($off_bytes -> $on_bytes bytes)"
fi
