#!/usr/bin/env bash
# Checks the private multi-lane result ABI and its report/on transaction.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixtures=(91_flat_result 93_flat_result_wrappers 98_product_lanes 99_recursive_adt_iter)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

# Pin the new recursive/tagged fixture under the most destructive printer too. Source maps are
# checked independently below because item/local renaming deliberately disables them. Building
# this keyed sysroot first lets the regular off/report/on loop leave the checkout on its usual
# optimizer-on cache when the script exits.
min_args="js-lane=jir js-opt=on js-minify=on"
JS_EXTRA_ARGS="$min_args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
JSC_RUST_MIR_INLINE=off JS_EXTRA_ARGS="$min_args" \
	"$script_dir/compile_core.sh" "$root/examples/core-tests/99_recursive_adt_iter.rs" \
	"$work/99_recursive_adt_iter-on-min.js" >/dev/null

for mode in off report on; do
	args="js-lane=jir js-opt=$mode"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	for fixture in "${fixtures[@]}"; do
		JSC_RUST_MIR_INLINE=off JS_EXTRA_ARGS="$args" \
			"$script_dir/compile_core.sh" "$root/examples/core-tests/$fixture.rs" \
			"$work/$fixture-$mode.js" >/dev/null
	done
done

map_args="js-lane=jir js-opt=on js-source-map=on"
for fixture in "${fixtures[@]}"; do
	JSC_RUST_MIR_INLINE=off JS_EXTRA_ARGS="$map_args" \
		"$script_dir/compile_core.sh" "$root/examples/core-tests/$fixture.rs" \
		"$work/$fixture-on-map.js" >/dev/null
done

# A chunked link is a hard whole-program boundary: the fixture must remain canonical and execute
# through the ESM chunk runner instead of borrowing the unsplit physical plan.
chunk_args="js-lane=jir js-opt=on js-modules=esm js-chunk=only:rust_entry"
JSC_RUST_MIR_INLINE=off JS_EXTRA_ARGS="$chunk_args" \
	"$script_dir/compile_core.sh" "$root/examples/core-tests/99_recursive_adt_iter.rs" \
	"$work/99_recursive_adt_iter-on-chunk.js" >/dev/null

for fixture in "${fixtures[@]}"; do
	cmp "$work/$fixture-off.js" "$work/$fixture-report.js"
	cat "$root/runtime/shim.js" "$work/$fixture-on.js" "$script_dir/entry.js" | node - | \
		diff -u "$root/examples/core-tests/$fixture.expected" -
	cat "$root/runtime/shim.js" "$work/$fixture-on-map.js" "$script_dir/entry.js" | node - | \
		diff -u "$root/examples/core-tests/$fixture.expected" -
	node "$script_dir/check-sourcemap.js" "$work/$fixture-on-map.js" "$fixture.rs" >/dev/null
done

cat "$root/runtime/shim.js" "$work/99_recursive_adt_iter-on-min.js" "$script_dir/entry.js" | node - | \
	diff -u "$root/examples/core-tests/99_recursive_adt_iter.expected" -
"$script_dir/run-chunks.sh" "$work/99_recursive_adt_iter-on-chunk.js" \
	"flat-result-99" only:rust_entry | \
	diff -u "$root/examples/core-tests/99_recursive_adt_iter.expected" -

jq -e '
  .abi.applied == true and
  .abi.registers >= 1 and
  any(.abi.flat[]; .result == 2)
' "$work/91_flat_result-on.js.lane.json" >/dev/null
jq -e '
  .abi.applied == false and
  any(.abi.flat[]; .result == 2)
' "$work/91_flat_result-report.js.lane.json" >/dev/null
jq -e 'any(.abi.flat[]; .result == 2)' "$work/91_flat_result-on-map.js.lane.json" >/dev/null
jq -e '
  .abi.applied == true and
  .abi.flat == [] and
  .physical.selected_calls == [] and
  .physical.planned.score > .physical.canonical.score and
  .physical.validated == .physical.canonical and
  .physical.final_emitted == .physical.canonical and
  ([.physical.decisions[] |
    select(
      (.item == "exported_pair" or
       .item == "exported_tuple" or
       .item == "fallible_pair") and
      .shape == "product-result(2)" and
      .selected == false and
      .reason == "cost.score-no-improvement" and
      .after.score > .before.score)] | length) == 3
' "$work/93_flat_result_wrappers-on.js.lane.json" >/dev/null
jq -e '[.items[] | select(.name | contains("make_pair")) | .js_alloc.object] | add > 0' \
	"$work/91_flat_result-off.js.lane.json" >/dev/null
jq -e '[.items[] | select(.name | contains("make_pair")) | .js_alloc.object] | add == 0' \
	"$work/91_flat_result-on.js.lane.json" >/dev/null
jq -e '
  .fallback_count == 0 and
  any(.abi.flat[]; (.item | contains("bounce_even")) and .params[0] == 2) and
  any(.abi.flat[]; (.item | contains("bounce_odd")) and .params[0] == 2) and
  any(.abi.flat[]; (.item | contains("loop_product")) and .result == 2) and
  .physical.final_emitted.min < .physical.canonical.min and
  .physical.final_emitted.gzip < .physical.canonical.gzip and
  .physical.final_emitted.brotli < .physical.canonical.brotli and
  .physical.final_emitted.loop_allocations == 0 and
  ([.physical.decisions[] |
    select(
      ((.item | contains("make_option")) or
       (.item | contains("consume_option")) or
       (.item | contains("make_choice")) or
       (.item | contains("consume_choice")) or
       .item == "exported_tagged") and
      .shape != null and .selected == false and
      .reason == "cost.score-no-improvement")] | length) == 5 and
  ([.physical.decisions[] |
    select(
      ((.item | contains("make_forward_pair")) or
       (.item | contains("consume_forward_pair")) or
       (.item | contains("make_result")) or
       (.item | contains("consume_result"))) and
      .shape != null and .selected == false and
      .reason == "cost.compressed-growth-budget")] | length) == 4 and
  ([.items[] |
    select(
      (.name | contains("loop_product")) or
      (.name | contains("tagged_option")) or
      (.name | contains("tagged_result"))) |
    .js_alloc.in_loop] | length) == 3 and
  ([.items[] |
    select(
      (.name | contains("loop_product")) or
      (.name | contains("tagged_option")) or
      (.name | contains("tagged_result"))) |
    .js_alloc.in_loop] | add) == 0
' "$work/98_product_lanes-on.js.lane.json" >/dev/null
jq -e '
  ([.items[] |
    select(
      (.name | contains("make_option")) or
      (.name | contains("make_result")) or
      (.name | contains("make_choice"))) |
    .js_alloc.object] | add) > 0 and
  ([.items[] | select(.name == "exported_tagged") | .js_alloc.object] | add) > 0
' "$work/98_product_lanes-on.js.lane.json" >/dev/null
jq -e '
  .fallback_count == 0 and
  .abi.flat == [] and
  .physical.selected_calls == [] and
  .physical.planned.score > .physical.canonical.score and
  .physical.final_emitted == .physical.canonical and
  ([.physical.decisions[] |
    select(
      ((.item | contains("make_pair_option")) or
       (.item | contains("pair_option_helper_one")) or
       (.item | contains("pair_option_helper_two"))) and
      .shape != null and .selected == false and
      .reason == "cost.score-no-improvement")] | length) == 3 and
  any(.physical.decisions[];
    (.item | contains("make_option")) and
    .shape == "tagged-result(2)" and
    .selected == false and .reason == "cost.score-no-improvement") and
  any(.physical.decisions[];
    (.item | contains("make_nested")) and
    .shape == "recursive-product-result(3)" and
    .selected == false and .reason == "cost.compressed-growth-budget") and
  any(.physical.decisions[];
    (.item | contains("make_chunks")) and
    .shape == "recursive-product-result(7)" and
    .selected == false and .reason == "cost.score-no-improvement") and
  any(.physical.decisions[];
    (.item | contains("consume_chunks")) and
    .shape == "recursive-product(7)" and
    .selected == false and .reason == "cost.score-no-improvement") and
  any(.physical.decisions[];
    (.item | contains("iterator_control_flow")) and
    .shape == "tagged-result(2)" and
    .selected == false and .reason == "cost.score-no-improvement") and
  any(.physical.decisions[];
    .item == "exported_or_panic" and
    .selected == false and .reason == "boundary.exported") and
  any(.physical.decisions[];
    (.item | contains("impl_0_score")) and
    .selected == false and .reason == "boundary.non-call-reference") and
  any(.physical.decisions[];
    (.item | contains("mutate_tagged")) and
    .selected == false and .reason == "boundary.param-escapes")
' "$work/99_recursive_adt_iter-on.js.lane.json" >/dev/null
jq -e '
  .fallback_count == 0 and
  .abi.flat == [] and
  .physical.selected_calls == [] and
  .physical.planned.score > .physical.canonical.score and
  .physical.final_emitted == .physical.canonical and
  ([.physical.decisions[] |
    select(
      ((.item | contains("make_pair_option")) or
       (.item | contains("pair_option_helper_one")) or
       (.item | contains("pair_option_helper_two"))) and
      .selected == false and .reason == "cost.score-no-improvement")] | length) == 3
' "$work/99_recursive_adt_iter-on-min.js.lane.json" >/dev/null
jq -e '
  .abi.registers == 0 and
  .abi.flat == [] and
  .physical.canonical == null and
  .physical.selected_calls == [] and
  any(.physical.decisions[];
    (.item | contains("make_pair_option")) and .reason == "boundary.chunks") and
  any(.physical.decisions[];
    (.item | contains("pair_option_helper_one")) and .reason == "boundary.chunks") and
  any(.physical.decisions[];
    (.item | contains("pair_option_helper_two")) and .reason == "boundary.chunks")
' "$work/99_recursive_adt_iter-on-chunk.js.lane.json" >/dev/null

node - "$work/93_flat_result_wrappers-off.js" \
	"$work/93_flat_result_wrappers-on.js" "$root/runtime/shim.js" <<'NODE'
const fs = require('node:fs')
const vm = require('node:vm')

for (const program of process.argv.slice(2, 4)) {
  const context = vm.createContext({ TextDecoder, TextEncoder })
  vm.runInContext(fs.readFileSync(process.argv[4], 'utf8'), context)
  vm.runInContext(fs.readFileSync(program, 'utf8'), context)
  const exported = context.exported_pair(9)
  if (exported.left !== 10 || exported.right !== 27) {
    throw new Error(`exported canonical result changed shape: ${JSON.stringify(exported)}`)
  }
  const callback = context.indirect_pair(3)
  if (callback.left !== 5 || callback.right !== 7) {
    throw new Error(`function-pointer canonical result changed shape: ${JSON.stringify(callback)}`)
  }
  const tuple = context.exported_tuple(9)
  if (tuple[0] !== 10 || tuple[1] !== 18) {
    throw new Error(`exported canonical tuple changed shape: ${JSON.stringify(tuple)}`)
  }
  const fallible = context.fallible_pair(4, 0)
  if (fallible.left !== 11 || fallible.right !== 20 || context.direct_fallible_score(4, 0) !== 31) {
    throw new Error(`fallible product normal path changed: ${JSON.stringify(fallible)}`)
  }

  const marker = new Error('host marker')
  context.__rt.host_throw = () => { throw marker }
  for (const call of [
    (mode) => context.fallible_pair(4, mode),
    (mode) => context.direct_fallible_score(4, mode),
  ]) {
    let panicCaught = false
    try { call(1) } catch { panicCaught = true }
    if (!panicCaught) throw new Error(`product panic returned normally in ${program}`)

    let hostCaught = false
    try { call(2) } catch (error) { hostCaught = error === marker }
    if (!hostCaught) throw new Error(`product host throw changed in ${program}`)
  }
}
NODE

node - "$work/98_product_lanes-off.js" \
	"$work/98_product_lanes-on.js" "$root/runtime/shim.js" <<'NODE'
const fs = require('node:fs')
const vm = require('node:vm')

for (const program of process.argv.slice(2, 4)) {
  const context = vm.createContext({ TextDecoder, TextEncoder })
  vm.runInContext(fs.readFileSync(process.argv[4], 'utf8'), context)
  vm.runInContext(fs.readFileSync(program, 'utf8'), context)
  const some = context.exported_tagged(8)
  const none = context.exported_tagged(9)
  if (some.TAG !== 'Some' || !Object.values(some).includes(15) || none.TAG !== 'None') {
    throw new Error(`exported canonical tag changed shape: ${JSON.stringify({ some, none })}`)
  }
}
NODE

off_objects=$(grep -c '{ left: ' "$work/91_flat_result-off.js" || true)
on_objects=$(grep -c '{ left: ' "$work/91_flat_result-on.js" || true)
if ((on_objects >= off_objects)); then
	echo "flat result ABI did not remove a Pair materialization: $off_objects -> $on_objects" >&2
	exit 1
fi

echo "flat aggregate results (jir lane): PASS ($off_objects -> $on_objects Pair objects)"
