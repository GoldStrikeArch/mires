#!/usr/bin/env bash
# Pins the formatting analysis: which format_args! sites are eligible and why the rest are not.
#
# The analysis is report-only, so this suite compiles with no executable pass selected and
# requires all three modes to be byte-identical. That is the invariant a future emission round
# has to keep for `off` while it changes `on`.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/51_formatting_shapes.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for mode in off report on; do
	args="js-opt=$mode js-opt-passes=none"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="$args js-opt-warnings=off js-opt-report=$work/$mode.json" \
		"$script_dir/compile_core.sh" "$fixture" "$work/$mode.js" >/dev/null
done
cmp "$work/off.js" "$work/report.js"
cmp "$work/off.js" "$work/on.js"

node - "$work" "$root" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const work = process.argv[2];
const root = process.argv[3];

const cases = [
  ["format_integers", [1920, -1080], "1920x-1080"],
  ["format_mixed", ["max-monochrome", 8], "(max-monochrome: 8)"],
  ["format_char_and_bool", [0x4e2d, true], "\u4e2dtrue!"],
  ["format_float", [0.5], "0.5px"],
  ["format_width_spec", [42], "      42"],
  ["format_to_string_chain", [7], "<7>"],
];

function load(mode) {
  const context = vm.createContext({ TextDecoder, TextEncoder });
  vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
  vm.runInContext(fs.readFileSync(path.join(work, `${mode}.js`), "utf8"), context);
  return context;
}

for (const mode of ["off", "report", "on"]) {
  const context = load(mode);
  for (const [symbol, args, expected] of cases) {
    const actual = context.format_as_str(context[symbol](...args));
    if (actual !== expected) {
      throw new Error(
        `${mode}: ${symbol}(${args}) produced ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`,
      );
    }
  }
  // `format_owned` takes a String, so it needs one the program itself built.
  const owned = context.format_as_str(context.format_owned(context.format_integers(3, 4)));
  if (owned !== "[3x4]") {
    throw new Error(`${mode}: format_owned produced ${JSON.stringify(owned)}, expected "[3x4]"`);
  }
}

const expected = new Map([
  ["format_integers", { selected: true, reason: null }],
  ["format_mixed", { selected: true, reason: null }],
  ["format_char_and_bool", { selected: true, reason: null }],
  ["format_owned", { selected: true, reason: null }],
  ["format_to_string_chain", { selected: true, reason: null }],
  ["format_float", { selected: true, reason: null }],
  [
    "format_width_spec",
    {
      selected: false,
      reason: "format template uses flags, width, precision, or explicit argument selection",
    },
  ],
]);

for (const mode of ["report", "on"]) {
  const report = JSON.parse(fs.readFileSync(path.join(work, `${mode}.json`), "utf8"));
  const seen = new Set();
  for (const item of report.items) {
    const wanted = expected.get(item.name);
    if (!wanted) continue;
    seen.add(item.name);
    const selections = (item.selections ?? [])
      .filter((selection) => selection.pass === "formatting-analysis")
      .map((selection) => selection.selected);
    const misses = (item.misses ?? [])
      .filter((miss) => miss.pass === "formatting-analysis")
      .map((miss) => miss.reason);
    const lowerable = selections.some((selected) =>
      selected.startsWith("native-default-display-builder-format-"));
    if (lowerable !== wanted.selected) {
      throw new Error(
        `${mode}: ${item.name} eligibility is ${lowerable}, expected ${wanted.selected} (${selections})`,
      );
    }
    if (wanted.reason !== null && !misses.includes(wanted.reason)) {
      throw new Error(
        `${mode}: ${item.name} did not record ${JSON.stringify(wanted.reason)}, recorded ${JSON.stringify(misses)}`,
      );
    }
  }
  for (const name of expected.keys()) {
    if (!seen.has(name)) throw new Error(`${mode}: ${name} is missing from the report`);
  }
}

const off = JSON.parse(fs.readFileSync(path.join(work, "off.json"), "utf8"));
for (const item of off.items) {
  if ((item.selections ?? []).some((selection) => selection.pass === "formatting-analysis")) {
    throw new Error(`off recorded a formatting selection on ${item.name}`);
  }
}
NODE

bytes=$(wc -c <"$work/off.js" | tr -d ' ')
echo "formatting analysis: PASS (6 eligible sites, 1 recorded decline, $bytes bytes in every mode)"
