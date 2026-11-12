#!/usr/bin/env bash
# usage: run_host_number.sh <program.js>
{ cat /Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-aae8e384d9b94c2e6/runtime/shim.js "$1"; cat <<'JS'
const holders = {
  "number 12": { count: 12 },
  "bigint 12n": { count: 12n },
  "symbol": { count: Symbol("s") },
  "object with valueOf": { count: { valueOf() { trace.push("host valueOf ran"); throw new Error("valueOf threw"); } } },
};
let trace;
for (const use_bits of [false, true]) {
  for (const [label, holder] of Object.entries(holders)) {
    trace = [];
    let outcome;
    try { outcome = "returned " + run(holder, trace, use_bits); }
    catch (e) { outcome = "threw " + (__rt.is_abort(e) ? "ABORT " : "") + (e && e.constructor ? e.constructor.name : typeof e) + ": " + (e && e.message); }
    console.log((use_bits ? "bits " : "sum  ") + label.padEnd(20) + " trace=" + JSON.stringify(trace) + "  " + outcome);
  }
}
JS
} | node - 2>&1
