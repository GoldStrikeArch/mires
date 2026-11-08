#!/usr/bin/env bash
# Checks exact byte-slice Take/Skip cursor lowering, hard declines, and witness rollback.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/105_slice_iterator_cursor.rs"
expected="$root/examples/core-tests/105_slice_iterator_cursor.expected"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

args='js-lane=jir js-opt=on js-opt-warnings=off'
report_args='js-lane=jir js-opt=report js-opt-warnings=off'
JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null

RCGJS_JIR_PASSES='constprop,inline,devirt,pea,range,dce' \
	JSC_RUST_MIR_INLINE=on JS_EXTRA_ARGS="$args" \
	"$script_dir/compile_core.sh" "$fixture" "$work/baseline.js" >/dev/null
RCGJS_WITNESS_CHECK=1 JSC_RUST_MIR_INLINE=on JS_EXTRA_ARGS="$args" \
	"$script_dir/compile_core.sh" "$fixture" "$work/candidate.js" >/dev/null
JS_EXTRA_ARGS="$report_args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
RCGJS_WITNESS_CHECK=1 JSC_RUST_MIR_INLINE=on JS_EXTRA_ARGS="$report_args" \
	"$script_dir/compile_core.sh" "$fixture" "$work/report.js" >/dev/null
RCGJS_WITNESS_CHECK=1 RCGJS_WITNESS_FAULT=slice-iterator \
	JSC_RUST_MIR_INLINE=on JS_EXTRA_ARGS="$args" \
	"$script_dir/compile_core.sh" "$fixture" "$work/fault.js" >/dev/null

for build in baseline candidate report fault; do
	cat "$root/runtime/shim.js" "$work/$build.js" "$script_dir/entry.js" | node - |
		diff -u "$expected" -
done

jq -e '
  .schema == 17 and
  .fallback_count == 0 and
  .slice_iter.would_lower == 0 and
  .slice_iter.final_emitted == 0
' "$work/baseline.js.lane.json" >/dev/null

jq -e '
  .schema == 17 and
  .fallback_count == 0 and
  .slice_iter.would_lower == 3 and
  .slice_iter.final_emitted == 3 and
  .witnesses.failed == 0 and
  any(.items[];
    (.name | contains("slice_iterator_cursor$tail_checksum")) and
    .lane == "jir" and
    .decline == null and
    .slice_iter.would_lower == 2 and
    .slice_iter.final_emitted == 2 and
    .alloc.materialize == 0 and
    .alloc.copy == 0)
' "$work/candidate.js.lane.json" >/dev/null

jq -e '
  .schema == 17 and
  .fallback_count == 0 and
  .slice_iter.would_lower > 0 and
  .slice_iter.final_emitted == 0 and
  all(.items[]; .slice_iter.final_emitted == 0)
' "$work/report.js.lane.json" >/dev/null

baseline_bytes=$(jq -r '.items[] | select(.name | contains("slice_iterator_cursor$tail_checksum")) | .bytes_min' "$work/baseline.js.lane.json")
candidate_bytes=$(jq -r '.items[] | select(.name | contains("slice_iterator_cursor$tail_checksum")) | .bytes_min' "$work/candidate.js.lane.json")
if [ "$candidate_bytes" -ge "$baseline_bytes" ]; then
	echo "slice-iterator-test: cursor lowering did not shrink the fixture ($baseline_bytes -> $candidate_bytes)" >&2
	exit 1
fi

for control in wide_checksum signed_checksum other_order observed_state; do
	baseline_control=$(jq -r --arg name "slice_iterator_cursor\$$control" \
		'.items[] | select(.name | contains($name)) | .bytes_min' \
		"$work/baseline.js.lane.json")
	candidate_control=$(jq -r --arg name "slice_iterator_cursor\$$control" \
		'.items[] | select(.name | contains($name)) | .bytes_min' \
		"$work/candidate.js.lane.json")
	if [ -z "$baseline_control" ] || [ "$baseline_control" != "$candidate_control" ]; then
		echo "slice-iterator-test: hard control $control changed ($baseline_control -> $candidate_control)" >&2
		exit 1
	fi
	jq -e --arg name "slice_iterator_cursor\$$control" '
    any(.items[];
      (.name | contains($name)) and
      .slice_iter.would_lower == 0 and
      .slice_iter.final_emitted == 0)
  ' "$work/candidate.js.lane.json" >/dev/null
done

jq -e '
  .fallback_count > 0 and
  .slice_iter.would_lower > 0 and
  .slice_iter.final_emitted == 0 and
  .witnesses.failed > 0 and
  any(.items[];
    (.name | contains("slice_iterator_cursor$tail_checksum")) and
    .lane == "fallback" and
    .decline == "proof.witness")
' "$work/fault.js.lane.json" >/dev/null

echo "slice iterator cursor: PASS (3 exact loops, $baseline_bytes -> $candidate_bytes bytes, hard controls and witness rollback)"
