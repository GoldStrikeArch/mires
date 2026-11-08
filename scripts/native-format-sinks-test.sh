#!/usr/bin/env bash
# Checks the format shapes the native format_args! lowering must not claim: a producer call
# feeding a template, a user function consuming an Arguments value, and write! sinks including
# two sites in one function. Every shape must behave identically in off, report, and on.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/57_native_format_sinks.rs"
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

function load(mode) {
  const context = vm.createContext({ TextDecoder, TextEncoder });
  vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
  vm.runInContext(fs.readFileSync(path.join(work, `${mode}.js`), "utf8"), context);
  return context;
}

const stringCases = [
  ["format_chain", [7], "<7>"],
  ["format_chain", [4294967295], "<4294967295>"],
  ["format_write", [3], "x=3"],
  ["format_write", [0], "x=0"],
  ["format_write_twice", [3, 0.5], "x=3,y=0.5"],
  ["format_write_twice", [0, -0], "x=0,y=-0"],
];

for (const mode of ["off", "report", "on"]) {
  const context = load(mode);
  for (const [symbol, args, expected] of stringCases) {
    const actual = context.format_as_str(context[symbol](...args));
    if (actual !== expected) {
      throw new Error(
        `${mode}: ${symbol}(${args}) produced ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`,
      );
    }
  }
  // The facade must run as a real call: a runtime template has no as_str fast path.
  const facade = context.format_facade(7);
  if (facade !== 2) {
    throw new Error(`${mode}: format_facade(7) produced ${JSON.stringify(facade)}, expected 2`);
  }
}
NODE

if [ "$lane" = baseline ]; then
	# The v1 text optimizer is deleted: `js-opt=on` on the baseline lane no longer rewrites
	# anything, so the modes are byte-frozen -- `on` must equal `off` exactly.
	cmp "$work/off.js" "$work/on.js"
fi
echo "native format sinks ($lane lane): PASS (off/report/on agree on producer, facade, and write! shapes)"
