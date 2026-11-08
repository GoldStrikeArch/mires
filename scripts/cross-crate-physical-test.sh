#!/usr/bin/env bash
# Checks that private physical-call evidence and selection survive an rlib boundary.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixtures="$root/examples/cross-crate-tests"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

args='js-profile=release-speed js-minify=off js-opt-passes=sroa js-source-map=on'
JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
host=$(rustc -vV | sed -n 's/^host: //p')
llvm_ar="$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-ar"

JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$fixtures/physical_provider.rs" "$work/libcross_crate_physical_provider.rlib" \
	--crate-type rlib --crate-name cross_crate_physical_provider >/dev/null

archive_dir="$work/archive"
reversed_dir="$work/reversed"
mkdir -p "$archive_dir"
mkdir -p "$reversed_dir"
members=()
while IFS= read -r member; do
	members+=("$member")
done < <("$llvm_ar" -t "$work/libcross_crate_physical_provider.rlib")
(
	cd "$archive_dir"
	"$llvm_ar" -x "$work/libcross_crate_physical_provider.rlib"
	# rustc requires lib.rmeta to remain the first rlib member. Reverse every
	# backend-owned member after it so reader order still varies legitimately.
	reversed=("${members[0]}")
	for ((index = ${#members[@]} - 1; index >= 1; index--)); do
		reversed+=("${members[index]}")
	done
	"$llvm_ar" -crs "$reversed_dir/libcross_crate_physical_provider.rlib" "${reversed[@]}"
)
JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$fixtures/physical_single.rs" "$work/single.js" >/dev/null
JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$fixtures/physical_client.rs" "$work/split.js" \
	--extern "cross_crate_physical_provider=$work/libcross_crate_physical_provider.rlib" \
	--search "$work" >/dev/null
JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$fixtures/physical_client.rs" "$reversed_dir/split.js" \
	--extern "cross_crate_physical_provider=$reversed_dir/libcross_crate_physical_provider.rlib" \
	--search "$reversed_dir" >/dev/null
JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$fixtures/physical_client_alt.rs" "$work/split-alt.js" \
	--extern "cross_crate_physical_provider=$work/libcross_crate_physical_provider.rlib" \
	--search "$work" >/dev/null

for program in single split; do
	value=$(cat "$root/runtime/shim.js" "$work/$program.js" - <<'NODE' | node -
console.log(cross_crate_physical_checksum());
NODE
	)
	if [ "$value" != 942031591 ]; then
		echo "$program build returned $value, expected 942031591" >&2
		exit 1
	fi

	jq -e --arg program "$program" '
      . as $report |
      $report.schema == 17 and
      $report.fallback_count == 0 and
      $report.plan_retries == 0 and
      $report.physical.profile == "release-speed" and
      $report.physical.cache_hits > 0 and
      $report.physical.planned == $report.physical.final_emitted and
      $report.physical.planned.loop_allocations < $report.physical.canonical.loop_allocations and
      ($report.physical.selected_calls | length) == 3 and
      any($report.physical.selected_calls[];
        (.item | contains("step")) and .wrapper == "canonical") and
      ([$report.physical.selected_calls[] |
        select(.params[0].product == ["buffer", "u32", "u32"])] | length) == 1 and
      ([$report.physical.selected_calls[] |
        select(.result.product == ["u32", "u32"])] | length) == 1 and
      all(["fold_slice", "result_pair", "step"][]; . as $needle |
        any($report.physical.decisions[];
          (.item | contains($needle)) and
          .selected == true and
          .reason == null and
          .hotness_source == "final-js-loop-allocation" and
          .budget_consumed_bps <= 500 and
          .cross_crate.summary_available == true and
          .cross_crate.summary_validated == true and
          .cross_crate.body_decoded == true and
          .cross_crate.caller_count > 0 and
          .cross_crate.crosses_crate == ($program == "split") and
          .cross_crate.selected == true and
          .cross_crate.reason == null))
    ' "$work/$program.js.lane.json" >/dev/null
done

for suffix in '' .lane.json .map; do
	if ! cmp -s "$work/split.js$suffix" "$reversed_dir/split.js$suffix"; then
		echo "reversing provider archive members changed split.js$suffix" >&2
		exit 1
	fi
done

alt_value=$(cat "$root/runtime/shim.js" "$work/split-alt.js" - <<'NODE' | node -
console.log(cross_crate_physical_alt_checksum());
NODE
)
if [ "$alt_value" != 607745230 ]; then
	echo "alternate client returned $alt_value, expected 607745230" >&2
	exit 1
fi
jq -e '
  .schema == 17 and
  .fallback_count == 0 and
  .plan_retries == 0 and
  .physical.profile == "release-speed" and
  .physical.cache_hits > 0 and
  .physical.planned.loop_allocations < .physical.canonical.loop_allocations and
  any(.physical.decisions[];
    .selected and .cross_crate.summary_validated and .cross_crate.crosses_crate)
' "$work/split-alt.js.lane.json" >/dev/null

jq -e '
  any(.sources[]; endswith("physical_single.rs"))
' "$work/single.js.map" >/dev/null
jq -e '
  any(.sources[]; endswith("physical_provider.rs")) and
  any(.sources[]; endswith("physical_client.rs"))
' "$work/split.js.map" >/dev/null
jq -e '
  any(.sources[]; endswith("physical_provider.rs")) and
  any(.sources[]; endswith("physical_client_alt.rs"))
' "$work/split-alt.js.map" >/dev/null

echo 'cross-crate physical calls: PASS (provider reused by two clients, validated summaries and source maps)'
