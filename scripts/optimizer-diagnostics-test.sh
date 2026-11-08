#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source_file="$root/examples/optimizer-tests/workspace/diagnostics.rs"
workspace_root="examples/optimizer-tests/workspace"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

compile() {
	local mode=$1
	local warnings=$2
	local name=$3
	local passes=${4:-all}
	JS_EXTRA_ARGS="js-opt=$mode js-opt-passes=$passes js-opt-warnings=$warnings js-opt-report=$work/$name.json js-workspace-root=$workspace_root" \
		"$script_dir/compile.sh" "$source_file" "$work/$name.js" \
		>"$work/$name.stdout" 2>"$work/$name.stderr"
}

compile off on off
compile report on report
compile report on report-repeat
compile on on on
compile on on on-none none
compile report on report-ast-dce ast-dce
compile report on report-link-dedup link-dedup
compile report on report-sroa sroa
compile report off warnings-off
# The fixture instantiates one generic twice, so `on` merges the two identical bodies and their
# rejected candidates with them. This variant keeps the two apart, which is what proves one
# warning is emitted for a span several reachable items reject at.
compile on on on-kept-duplicates sroa,ast-dce,future-poller,native-representations

cmp "$work/off.js" "$work/report.js"
cmp "$work/off.js" "$work/on-none.js"
cmp "$work/off.js" "$work/report-ast-dce.js"
cmp "$work/off.js" "$work/report-link-dedup.js"
cmp "$work/off.js" "$work/report-sroa.js"
cmp "$work/report.json" "$work/report-repeat.json"

node - "$work" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");

const work = process.argv[2];

function fail(message) {
  console.error(`optimizer-diagnostics-test: ${message}`);
  process.exit(1);
}

function read(name, extension) {
  return fs.readFileSync(path.join(work, `${name}.${extension}`), "utf8");
}

function checkAccounting(name) {
  const report = JSON.parse(read(name, "json"));
  const emittedBytes = fs.statSync(path.join(work, `${name}.js`)).size;
  const accounted =
    report.reachable_item_bytes + report.link_generated_item_bytes + report.synthesized_bytes;
  const itemBytes = report.items.reduce((total, item) => total + item.bytes, 0);
  const crateBytes = report.crates.reduce((total, crate) => total + crate.bytes, 0);
  const crateItems = report.crates.reduce((total, crate) => total + crate.items, 0);

  if (report.program_bytes !== emittedBytes) {
    fail(`${name}: report says ${report.program_bytes} bytes, file has ${emittedBytes}`);
  }
  if (accounted !== report.program_bytes) {
    fail(`${name}: byte categories total ${accounted}, expected ${report.program_bytes}`);
  }
  if (itemBytes !== report.reachable_item_bytes || crateBytes !== report.reachable_item_bytes) {
    fail(`${name}: reachable item/crate byte totals disagree`);
  }
  if (crateItems !== report.items.length) {
    fail(`${name}: crate item count does not cover the report item manifest`);
  }
  return report;
}

function checkPasses(name, requested, analyzed, emission_enabled, applied) {
  const report = checkAccounting(name);
  const registered = ["sroa", "ast-dce", "future-poller", "native-representations", "link-dedup"];
  for (const [field, expected] of Object.entries({
    registered, requested, analyzed, emission_enabled, applied,
  })) {
    const actual = report.passes[field];
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      fail(`${name}: ${field} passes are ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
}

function optimizerWarnings(name) {
  return read(name, "stderr")
    .split("\n")
    .filter((line) => line.includes("rustc_codegen_js optimizations were not applied"));
}

function checkWarningMode(name) {
  const report = checkAccounting(name);
  const misses = report.items.flatMap((item) => item.misses);
  const workspaceMisses = misses.filter((miss) => miss.workspace && miss.location !== null);
  const dependencyMisses = misses.filter(
    (miss) => !miss.workspace && miss.location?.includes("dependency.rs"),
  );
  const locationCounts = new Map();
  for (const miss of workspaceMisses) {
    locationCounts.set(miss.location, (locationCounts.get(miss.location) ?? 0) + 1);
  }
  const locations = [...locationCounts.keys()];
  const warnings = optimizerWarnings(name);

  if (![...locationCounts.values()].some((count) => count > 1)) {
    fail(`${name}: fixture did not produce duplicate reachable misses at one workspace span`);
  }
  if (dependencyMisses.length === 0) {
    fail(`${name}: outside dependency miss was not retained in JSON`);
  }
  if (warnings.length !== locations.length) {
    fail(`${name}: got ${warnings.length} warnings for ${locations.length} workspace spans`);
  }
  for (const location of locations) {
    if (!warnings.some((warning) => warning.includes(location))) {
      fail(`${name}: warning does not name rejected workspace span ${location}`);
    }
  }
  if (warnings.some((warning) => warning.includes("dependency.rs"))) {
    fail(`${name}: outside dependency miss escaped the JSON-only boundary`);
  }
}

// The fixture instantiates one generic at two types, which prints one body under two names.
function checkDuplicateMerge() {
  const kept = JSON.parse(read("on-kept-duplicates", "json"));
  const merged = JSON.parse(read("on", "json"));
  if (!Array.isArray(kept.dedup) || kept.dedup.length !== 0) {
    fail("a build without link-dedup reported merged duplicates");
  }
  if (!Array.isArray(merged.dedup) || merged.dedup.length === 0) {
    fail("link-dedup merged nothing in a program with two instantiations of one generic");
  }
  const names = new Set(merged.items.map((item) => item.name));
  for (const record of merged.dedup) {
    if (!names.has(record.winner)) {
      fail(`link-dedup kept a record for the unemitted item ${record.winner}`);
    }
    if (record.merged.length === 0 || record.bytes_saved <= 0) {
      fail(`link-dedup recorded an empty merge for ${record.winner}`);
    }
    for (const name of record.merged) {
      if (names.has(name)) {
        fail(`link-dedup emitted ${name} after merging it into ${record.winner}`);
      }
    }
  }
  if (merged.program_bytes >= kept.program_bytes) {
    fail(`link-dedup grew the program: ${kept.program_bytes} -> ${merged.program_bytes}`);
  }
}

const all = ["sroa", "ast-dce", "future-poller", "native-representations", "link-dedup"];
const keptDuplicates = ["sroa", "ast-dce", "future-poller", "native-representations"];
checkPasses("off", all, [], [], []);
checkWarningMode("report");
checkWarningMode("on-kept-duplicates");
checkPasses("report", all, all, [], []);
checkPasses("on", all, all, all, ["link-dedup"]);
checkPasses("on-none", [], [], [], []);
checkPasses("report-ast-dce", ["ast-dce"], ["ast-dce"], [], []);
checkPasses("report-link-dedup", ["link-dedup"], ["link-dedup"], [], []);
checkPasses("report-sroa", ["sroa"], ["sroa"], [], []);
checkPasses("on-kept-duplicates", keptDuplicates, keptDuplicates, keptDuplicates, []);
checkDuplicateMerge();
const disabled = checkAccounting("warnings-off");
if (optimizerWarnings("warnings-off").length !== 0) {
  fail("js-opt-warnings=off still emitted an optimizer warning");
}
if (!disabled.items.some((item) => item.misses.length > 0)) {
  fail("disabling diagnostics also removed misses from the JSON report");
}

console.log("optimizer diagnostics: ok");
NODE
