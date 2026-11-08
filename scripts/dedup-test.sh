#!/usr/bin/env bash
# Checks the link-time duplicate item merge: one body per distinct piece of JavaScript.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/48_link_dedup.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for mode in off report on; do
	args="js-opt=$mode js-opt-passes=link-dedup"
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

// The seeds cover the wrapping edge, so a merge that dropped an arithmetic difference shows up.
const seeds = [0, 1, 3, 7, 0xfffffff0];
const expected = [990, 1080, 1260, 1620, 196926];
for (const mode of ["off", "report", "on"]) {
  const context = vm.createContext({});
  vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
  vm.runInContext(fs.readFileSync(path.join(work, `${mode}.js`), "utf8"), context);
  seeds.forEach((seed, at) => {
    const actual = context.link_dedup(seed);
    if (actual !== expected[at]) {
      throw new Error(`${mode} link_dedup(${seed}) = ${actual}, expected ${expected[at]}`);
    }
  });
}

const read = (mode) => JSON.parse(fs.readFileSync(path.join(work, `${mode}.json`), "utf8"));
const baseline = read("off");
const report = read("report");
const merged = read("on");

for (const [mode, value] of [["off", baseline], ["report", report]]) {
  if (!Array.isArray(value.dedup) || value.dedup.length !== 0) {
    throw new Error(`${mode} merged duplicates: ${JSON.stringify(value.dedup)}`);
  }
  if (value.passes.applied.includes("link-dedup")) {
    throw new Error(`${mode} claims link-dedup changed an emitted item`);
  }
}
if (!merged.passes.applied.includes("link-dedup")) {
  throw new Error(`on did not apply link-dedup: ${JSON.stringify(merged.passes.applied)}`);
}

// The fixture instantiates one generic body under several names, and the closures two callers
// only differ in are what the second round needs to see merged.
const records = merged.dedup;
if (records.length === 0) {
  throw new Error("link-dedup merged nothing in a program built out of duplicate instantiations");
}
const emitted = new Set(merged.items.map((item) => item.name));
let mergedItems = 0;
let bytesSaved = 0;
for (const record of records) {
  if (!emitted.has(record.winner)) {
    throw new Error(`link-dedup kept a record for the unemitted item ${record.winner}`);
  }
  if (record.merged.length === 0 || record.bytes_saved <= 0) {
    throw new Error(`link-dedup recorded an empty merge for ${record.winner}`);
  }
  for (const name of record.merged) {
    if (emitted.has(name)) {
      throw new Error(`link-dedup emitted ${name} after merging it into ${record.winner}`);
    }
    if (name <= record.winner) {
      throw new Error(`link-dedup kept ${record.winner} over the smaller name ${name}`);
    }
  }
  mergedItems += record.merged.length;
  bytesSaved += record.bytes_saved;
}

// `fold` calls its closure by name, so the two instantiations are only duplicates once the two
// identical closures have been merged. Finding it proves the pass runs to a fixed point.
const paths = new Map(merged.items.map((item) => [item.name, item.path]));
const foldRecord = records.find((record) => (paths.get(record.winner) ?? "").includes("fold"));
if (foldRecord === undefined) {
  throw new Error("the second-round merge of the two `fold` instantiations did not happen");
}

if (merged.program_bytes >= baseline.program_bytes) {
  throw new Error(`link-dedup grew the program: ${baseline.program_bytes} -> ${merged.program_bytes}`);
}
const accounted =
  merged.reachable_item_bytes + merged.link_generated_item_bytes + merged.synthesized_bytes;
if (accounted !== merged.program_bytes) {
  throw new Error(`on: byte categories total ${accounted}, expected ${merged.program_bytes}`);
}

console.log(
  `link-dedup: ${records.length} groups, ${mergedItems} items merged, ${bytesSaved} pre-rename bytes`,
);
NODE

off_bytes=$(wc -c <"$work/off.js" | tr -d ' ')
on_bytes=$(wc -c <"$work/on.js" | tr -d ' ')
echo "link-time duplicate merge: PASS ($off_bytes -> $on_bytes bytes; off and report byte-identical)"
