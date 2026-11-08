#!/usr/bin/env bash
# Checks idiomatic f64::to_string lowering into an opaque JS-native String.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/46_native_f64_string.rs"
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
const cases = [
  [NaN, "NaN"],
  [Infinity, "inf"],
  [-Infinity, "-inf"],
  [0, "0"],
  [-0, "-0"],
  [1 / 3, "0.3333333333333333"],
  [7.5e-11, "0.000000000075"],
  [1.9971e20, "199710000000000000000"],
  [1.00000762939453125, "1.0000076293945313"],
  [Number.MIN_VALUE, "0." + "0".repeat(323) + "5"],
  [Number.MAX_VALUE, "17976931348623157" + "0".repeat(292)],
];

function load(mode) {
  const context = vm.createContext({ TextDecoder, TextEncoder });
  vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
  vm.runInContext(fs.readFileSync(path.join(work, `${mode}.js`), "utf8"), context);
  return context;
}

for (const mode of ["off", "report", "on"]) {
  const context = load(mode);
  for (const [input, expected] of cases) {
    const value = context.native_f64_string(input);
    const actual = context.native_f64_string_as_str(value);
    if (actual !== expected) {
      throw new Error(`${mode}: ${input} formatted as ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
    if (context.native_f64_string_len(value) !== expected.length) {
      throw new Error(`${mode}: ${input} has the wrong UTF-8 byte length`);
    }
    if (mode === "on") {
      if (context.native_f64_string_capacity(value) < expected.length) {
        throw new Error(`${mode}: ${input} has insufficient native String capacity`);
      }
      if (context.native_f64_string_round_trip(value) !== value) {
        throw new Error(`${mode}: ${input} changed native String identity`);
      }
    }
  }
}

NODE

if [ "$lane" = baseline ]; then
	# The v1 text optimizer is deleted: `js-opt=on` on the baseline lane no longer rewrites
	# anything, so the receipt is byte-frozen -- `on` must equal `off` exactly.
	cmp "$work/off.js" "$work/on.js"
	echo "native f64 String lowering (baseline lane): frozen, on == off ($(wc -c <"$work/off.js" | tr -d ' ') bytes)"
else
	off_bytes=$(wc -c <"$work/off.js" | tr -d ' ')
	on_bytes=$(wc -c <"$work/on.js" | tr -d ' ')
	if ((on_bytes >= off_bytes)); then
		echo "native f64 String lowering (jir lane) did not reduce output: $off_bytes -> $on_bytes" >&2
		exit 1
	fi
	echo "native f64 String lowering (jir lane): PASS ($off_bytes -> $on_bytes bytes)"
fi
