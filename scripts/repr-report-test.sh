#!/usr/bin/env bash
#
# The representation plan's report-mode contract (Phase 4, D12), extending jir-report-test.sh's
# byte identity to a program the planner works on:
#
#   * `js-opt=off` and `js-opt=report` print byte-identical programs — the solve runs fully in
#     report mode and commits nothing;
#   * `js-opt=on` prints a different program (the optimizer applied);
#   * all three programs print the same stdout under node;
#   * the sidecar carries the schema-17 staged `repr` block in `report` and `on`, `null` under
#     `off`, and only `on` reports final emitted applications.
#
# Until the solver (W4b) lands the block is all zero, and the `on` byte difference comes from the
# other passes; the claims above hold either way, which is what makes this script landable first.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi
for mode in off report on; do
	JS_EXTRA_ARGS="js-opt=$mode" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
done

shim="$root/runtime/shim.js"
entry="$script_dir/entry.js"

# 73_repr_commit_two_regions joins the list when W4b lands it: the committed-plan program is the
# scenario this script exists for, and 29_rc is the placeholder with a nonzero would_apply.
for fixture in 29_rc 73_repr_commit_two_regions; do
	src="$root/examples/core-tests/$fixture.rs"
	[ -f "$src" ] || continue
	for mode in off report on; do
		JS_EXTRA_ARGS="js-lane=jir js-opt=$mode js-opt-warnings=off" SKIP_BUILD=1 \
			"$script_dir/compile_core.sh" "$src" "$work/$fixture-$mode.js" >/dev/null
		cat "$shim" "$work/$fixture-$mode.js" "$entry" | node - >"$work/$fixture-$mode.out"
	done

	cmp "$work/$fixture-off.js" "$work/$fixture-report.js"
	if cmp -s "$work/$fixture-off.js" "$work/$fixture-on.js"; then
		echo "repr-report-test: $fixture prints identical bytes under js-opt=off and on, so the" \
			"report/on split proves nothing here; pick a fixture the optimizer changes" >&2
		exit 1
	fi
	cmp "$work/$fixture-off.out" "$work/$fixture-report.out"
	cmp "$work/$fixture-off.out" "$work/$fixture-on.out"

	node - "$work/$fixture-off.js.lane.json" "$work/$fixture-report.js.lane.json" \
		"$work/$fixture-on.js.lane.json" <<'NODE'
const fs = require("node:fs");
const [off, report, on] = process.argv.slice(2).map((p) => JSON.parse(fs.readFileSync(p, "utf8")));
if (off.schema !== 17) throw new Error(`sidecar schema is ${off.schema}, this script reads 17`);
if (off.repr !== null) {
  throw new Error(`js-opt=off ran the representation analysis: ${JSON.stringify(off.repr)}`);
}
for (const [mode, sidecar] of [["report", report], ["on", on]]) {
  const repr = sidecar.repr;
  if (repr === null || typeof repr !== "object") {
    throw new Error(`js-opt=${mode} carries no repr block`);
  }
  for (const stage of ["planned", "validated", "final_emitted"]) {
    if (!repr[stage] || typeof repr[stage] !== "object") {
      throw new Error(`repr.${stage} is not an object (${mode})`);
    }
  }
  if (!Array.isArray(repr.decisions)) throw new Error(`repr.decisions is not a list (${mode})`);
  if (repr.validated.would_commit > repr.validated.cost_positive ||
      repr.validated.cost_positive > repr.planned.legal ||
      repr.planned.legal > repr.planned.candidates) {
    throw new Error(`repr counts are not nested: ${JSON.stringify(repr)} (${mode})`);
  }
  // The v1 accounting artifact must be structurally impossible: a decline that says "no
  // improvement" while both sides measured equal is a cost-model bug, not a decision.
  for (const decision of repr.decisions) {
    const match = /cost\.no-improvement before=(\d+) after=(\d+)/.exec(decision.decline ?? "");
    if (match && match[1] === match[2]) {
      throw new Error(`no-improvement with before==after: ${JSON.stringify(decision)} (${mode})`);
    }
  }
}
// The would_apply pattern: report's repr block must say what on's says.
const strip = ({ final_emitted, ...plan }) => plan;
if (JSON.stringify(strip(report.repr)) !== JSON.stringify(strip(on.repr))) {
  throw new Error(`report and on disagree about the repr accounting:\n  report ${
    JSON.stringify(strip(report.repr))}\n  on     ${JSON.stringify(strip(on.repr))}`);
}
if (report.repr.final_emitted.applied !== 0) {
  throw new Error(`report mode claims applied regions: ${JSON.stringify(report.repr.final_emitted)}`);
}
if (on.repr.final_emitted.applied > on.repr.validated.would_commit) {
  throw new Error(`final reachability invented a representation region: ${JSON.stringify(on.repr)}`);
}
if (on.repr.validated.would_commit > 0 && on.repr.final_emitted.applied === 0) {
  throw new Error(`the committed-plan fixture emitted no representation region: ${JSON.stringify(on.repr)}`);
}
console.log(`  repr ${JSON.stringify(strip(on.repr))}`);
NODE
	echo "repr-report-test ($fixture): PASS (off == report bytes, on differs, stdout mode-identical)"
done

# 70_host_contract_slots: the V3 host-contract slot facts (F2/F3). It names `mires_js`, so it
# is compiled with the same externs scripts/test.sh offers. The sidecar must show the declared
# `reads, accepts(host_string)` slot leaving its edge a sink — the flow it reads still commits
# `host-string` — while the `retains, mutates` pin stops nothing else: an unrelated builder
# region still commits, and stdout is identical in all three modes.
fixture=70_host_contract_slots
src="$root/examples/core-tests/$fixture.rs"
if [ -f "$src" ]; then
	(cd "$root" && cargo build -p js-extern-macro) >/dev/null
	case "$(uname -s)" in
	Darwin) proc_macro="$root/target/debug/libjs_extern_macro.dylib" ;;
	*) proc_macro="$root/target/debug/libjs_extern_macro.so" ;;
	esac
	proc_macro_dir="$work/proc-macro"
	mkdir -p "$proc_macro_dir"
	ln -sf "$proc_macro" "$proc_macro_dir/$(basename -- "$proc_macro")"
	for mode in off report on; do
		args="js-lane=jir js-opt=$mode"
		JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
		JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
			"$root/mires-js/src/lib.rs" "$work/libmires_js.rlib" \
			--crate-type rlib --crate-name mires_js \
			--extern "js_extern_macro=$proc_macro" >/dev/null
		JS_EXTRA_ARGS="$args js-opt-warnings=off" "$script_dir/compile_core.sh" \
			"$src" "$work/$fixture-$mode.js" \
			--extern "mires_js=$work/libmires_js.rlib" \
			--search "$proc_macro_dir" >/dev/null
		cat "$shim" "$work/$fixture-$mode.js" "$entry" | node - >"$work/$fixture-$mode.out"
	done
	cmp "$work/$fixture-off.js" "$work/$fixture-report.js"
	cmp "$work/$fixture-off.out" "$work/$fixture-report.out"
	cmp "$work/$fixture-off.out" "$work/$fixture-on.out"
	node - "$work/$fixture-on.js.lane.json" <<'NODE'
const fs = require("node:fs");
const on = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const repr = on.repr;
if (!repr || repr.validated.would_commit < 2 || repr.final_emitted.applied < 2) {
  throw new Error(`the host-contract fixture no longer commits: ${JSON.stringify(repr)}`);
}
const chosen = repr.decisions.map((d) => d.chosen);
if (!chosen.includes("host-string")) {
  throw new Error(`the accepts(host_string) sink flow lost its host-string commit: ${chosen}`);
}
if (!chosen.includes("string-builder")) {
  throw new Error(`the retains,mutates pin poisoned the unrelated builder region: ${chosen}`);
}
console.log(`  repr applied=${repr.final_emitted.applied} chosen=[${chosen.filter((c) => c !== "baseline")}]`);
NODE
	echo "repr-report-test ($fixture): PASS (host-contract slots: sink commits, pin stays an edge)"
fi

echo "repr-report-test: ok"
