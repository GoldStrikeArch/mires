#!/usr/bin/env bash
# Checks the report-only async graph and the already executable Promise-driver semantics.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/26_async.rs"
expected="$root/examples/core-tests/26_async.expected"
ceiling="$root/bench/async-poller/out/promise-ceiling.json"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for mode in off report; do
	args="js-lane=jir js-opt=$mode js-opt-warnings=off"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" "$fixture" "$work/$mode.js" >/dev/null
	cat "$root/runtime/shim.js" "$work/$mode.js" "$script_dir/entry.js" | node - |
		diff -u "$expected" -
	jq -e '
    .schema == 17 and
    .async.schema == 8 and
    .async.mode == "report-only" and
    (.async.graphs | length) > 0 and
    all(.async.graphs[];
      .state_count >= 3 and
      .estimated_suspension_state_count == (.state_count - 3) and
      .suspension_state_count_derivation == "state-count-minus-three" and
      .await_relationships_proven == false and
      .wake_sources_proven == false and
      .selected == false and
      (.reason == "legality.raw-pointer-across-suspension" or
       .reason == "legality.borrow-across-suspension" or
       .reason == "legality.cleanup-state-unavailable" or
       .reason == "graph.await-relationship-unavailable"))
  ' "$work/$mode.js.lane.json" >/dev/null
done

cmp "$work/off.js" "$work/report.js"
node --test "$script_dir/host-future-driver.test.mjs"

jq -e '
  .gate.representativeWorkloadMeasured == false and
  .gate.loweringAuthorized == false
' "$ceiling" >/dev/null

echo "async graph: PASS (coroutine facts reported, native lowering withheld, Promise driver exact)"
