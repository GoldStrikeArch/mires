#!/usr/bin/env bash
# Checks scalar replacement of aggregates: no object is built to be taken apart.
#
# The fixture is compiled twice over, once with rustc's own MIR inliner off and once with it on
# ($JSC_RUST_MIR_INLINE). Inlining is what puts a `[u8; 4]` literal and the byte conversion that
# reads it in one body, so the two runs cover the pass with and without the shape its two
# expression rewrites need; the local split fires either way.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/52_sroa.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for inline in off on; do
	for mode in off report on; do
		args="js-opt=$mode js-opt-passes=sroa"
		JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
		JSC_RUST_MIR_INLINE="$inline" \
			JS_EXTRA_ARGS="$args js-opt-warnings=off js-opt-report=$work/$inline-$mode.json" \
			"$script_dir/compile_core.sh" "$fixture" "$work/$inline-$mode.js" >/dev/null
	done
	cmp "$work/$inline-off.js" "$work/$inline-report.js"
done

node - "$work" "$root" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const work = process.argv[2];
const root = process.argv[3];

// The seeds cover the wrapping edge, so a split that dropped an arithmetic difference shows up.
const seeds = [0, 1, 7, 0x0000ffff, 0xfffffff0];
const answers = new Map();
for (const inline of ["off", "on"]) {
  for (const mode of ["off", "report", "on"]) {
    const name = `${inline}-${mode}`;
    const context = vm.createContext({});
    vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
    vm.runInContext(fs.readFileSync(path.join(work, `${name}.js`), "utf8"), context);
    for (const seed of seeds) {
      const actual = context.sroa(seed);
      const key = String(seed);
      if (!answers.has(key)) answers.set(key, actual);
      if (answers.get(key) !== actual) {
        throw new Error(`${name} sroa(${seed}) = ${actual}, expected ${answers.get(key)}`);
      }
    }
  }
}
// A fixture whose answer is zero everywhere would pass the agreement check without computing.
if ([...answers.values()].every((value) => value === 0)) {
  throw new Error("the fixture computed nothing");
}

const read = (name) => JSON.parse(fs.readFileSync(path.join(work, `${name}.json`), "utf8"));
const selected = (report) =>
  report.items.flatMap((item) => item.selections).filter((s) => s.pass === "sroa");

for (const inline of ["off", "on"]) {
  for (const mode of ["off", "report"]) {
    if (read(`${inline}-${mode}`).passes.applied.includes("sroa")) {
      throw new Error(`${inline}-${mode} claims sroa changed an emitted item`);
    }
  }
  if (selected(read(`${inline}-report`)).length === 0) {
    throw new Error(`${inline}-report recorded no sroa selection`);
  }
  const applied = read(`${inline}-on`);
  if (!applied.passes.applied.includes("sroa")) {
    throw new Error(`${inline}-on did not apply sroa: ${JSON.stringify(applied.passes.applied)}`);
  }
}

// `spread` keeps the whole `Pair`, so the local feeding it must survive the split.
const emitted = fs.readFileSync(path.join(work, "on-on.js"), "utf8");
if (!/spread[^(]*\(\{ high:/.test(emitted)) {
  throw new Error("the aggregate a callee keeps was split anyway");
}
// The byte conversion reads its operand's elements where they are built, with no array left.
if (/\$b\[0\]/.test(emitted)) {
  throw new Error("a fresh byte array still reaches the conversion through a parameter");
}

// The pass trades syntax for allocation: what has to fall is the number of aggregates built, not
// the byte count, which grows a little before minification renames the new locals and shrinks
// again once the object literal syntax is gone.
const aggregates = (name) =>
  (fs.readFileSync(path.join(work, `${name}.js`), "utf8").match(/\{ high: /g) ?? []).length;
for (const inline of ["off", "on"]) {
  const before = aggregates(`${inline}-off`);
  const after = aggregates(`${inline}-on`);
  if (after >= before) {
    throw new Error(`sroa built as many aggregates in ${inline}: ${before} -> ${after}`);
  }
  const report = read(`${inline}-on`);
  const accounted =
    report.reachable_item_bytes + report.link_generated_item_bytes + report.synthesized_bytes;
  if (accounted !== report.program_bytes) {
    throw new Error(
      `${inline}-on: byte categories total ${accounted}, expected ${report.program_bytes}`,
    );
  }
}
NODE

for inline in off on; do
	off_built=$(grep -c '{ high: ' "$work/$inline-off.js" || true)
	on_built=$(grep -c '{ high: ' "$work/$inline-on.js" || true)
	echo "scalar replacement of aggregates (mir-inline=$inline): PASS ($off_built -> $on_built aggregates built; off and report byte-identical)"
done
