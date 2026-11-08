#!/usr/bin/env bash
# Checks Phase 12D's report facts, hard declines, and explicit exact-storage opt-in.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/104_exact_typed_vec.rs"
expected="$root/examples/core-tests/104_exact_typed_vec.expected"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for mode in off report on; do
	args="js-lane=jir js-opt=$mode js-opt-warnings=off"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" "$fixture" "$work/$mode.js" >/dev/null
	cat "$root/runtime/shim.js" "$work/$mode.js" "$script_dir/entry.js" | node - |
		diff -u "$expected" -
	jq -e '
    .schema == 17 and
    .physical.storage_mode == "report-only" and
    (.physical.storage_decisions | length) == 4 and
    all(.physical.storage_decisions[]; .selected == false) and
    any(.physical.storage_decisions[];
      .element == "u32" and
      .storage == "typed-array" and
      .typed_array == "Uint32Array" and
      .constant_initial_capacity == 2 and
      .proven_maximum_length == 2 and
      .proven_exact_final_length == 2 and
      .selected == false and
      .reason == "economics.complete-workload-evidence-missing") and
    any(.physical.storage_decisions[];
      .constant_initial_capacity == 4 and
      .reason == "legality.capacity-or-layout-observed") and
    any(.physical.storage_decisions[];
      .constant_initial_capacity == 2 and
      .reason == "legality.retained-view")
  ' "$work/$mode.js.lane.json" >/dev/null
	if [ "$mode" != off ]; then
		jq -e '
      any(.physical.decisions[];
        (.item | contains("exact_typed_vec$sum")) and
        .shape == "slice(3)" and
        .selected == true and
        .reason == null) and
      ([.physical.storage_decisions[] |
        select(.reason == "economics.complete-workload-evidence-missing")] | length) == 2
    ' "$work/$mode.js.lane.json" >/dev/null
	fi
	if grep -Eq 'new Uint32Array|Uint32Array\(' "$work/$mode.js"; then
		echo "storage-plan-test: js-opt=$mode emitted a report-only TypedArray" >&2
		exit 1
	fi
done

cmp "$work/off.js" "$work/report.js"
jq -e '
  .physical.cleanup_mode == "existing-executable" and
  any(.physical.drop_decisions[];
    .plan == "direct" and .reason == "registry.native-vec-drop")
' "$work/on.js.lane.json" >/dev/null

typed_args="js-lane=jir js-opt=on js-opt-warnings=off js-jir-typed-storage=on"
JS_EXTRA_ARGS="$typed_args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
JS_EXTRA_ARGS="$typed_args" "$script_dir/compile_core.sh" "$fixture" "$work/typed.js" >/dev/null
cat "$root/runtime/shim.js" "$work/typed.js" "$script_dir/entry.js" | node - |
	diff -u "$expected" -
jq -e '
  .physical.storage_mode == "exact-opt-in" and
  ([.physical.storage_decisions[] | select(.selected)] | length) == 2 and
  any(.abi.flat[];
    (.item | contains("exact_typed_vec$sum")) and
    .params == [3] and
    .canonical_wrapper == false) and
  any(.physical.storage_decisions[];
    .constant_initial_capacity == 2 and
    .proven_maximum_length == 2 and
    .proven_exact_final_length == 2 and
    .selected == true and
    .reason == null) and
  any(.physical.storage_decisions[];
    .reason == "legality.capacity-or-layout-observed") and
  any(.physical.storage_decisions[];
    .reason == "legality.retained-view")
' "$work/typed.js.lane.json" >/dev/null
test "$(grep -Ec 'new Uint32Array\(2\)' "$work/typed.js")" = 2
test "$(grep -Ec '__rt\.typed_vec_push\(' "$work/typed.js")" = 4
if grep -Eq 'as_slice[^;]*\(laned\)' "$work/typed.js"; then
	echo "storage-plan-test: scalar-lane Vec slice was materialized" >&2
	exit 1
fi

echo "storage plan: PASS (exact facts, hard declines, scalar slice lanes, and TypedArray opt-in)"
