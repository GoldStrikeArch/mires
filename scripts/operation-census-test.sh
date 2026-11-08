#!/usr/bin/env bash
# Captured operation identity and final dependencies are separate report contracts.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
work=$(mktemp -d "$root/build/operation-census.XXXXXX")
echo "operation census: retained output at $work"
census="$root/../../experiments-pro/compiler-v2/tools/summarize-jir-census.py"
for mode in off report on; do
  args="js-lane=jir js-opt=$mode js-opt-warnings=off js-source-map=on"
  JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  for fixture in 69_dead_call_dropped 75_repr_native_vec 107_box_pea_composition 108_standard_identity_owner; do
    output="$work/$fixture-$mode"
    mkdir "$output"
    JS_EXTRA_ARGS="$args js-opt-report=$output/report.json" "$script_dir/compile_core.sh" \
      "$root/examples/core-tests/$fixture.rs" "$output/program.js" >"$output/compile.log" 2>&1
    python3 "$census" --linked-report "$output/report.json" --output "$output/census"
    cat "$root/runtime/shim.js" "$output/program.js" "$script_dir/entry.js" | node - | \
      diff -u "$root/examples/core-tests/$fixture.expected" -
    node "$script_dir/check-sourcemap.js" "$output/program.js" >"$output/map.log"
  done
done

fixture=107_box_pea_composition
mkdir "$work/without-report" "$work/repeat"
JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
  "$root/examples/core-tests/$fixture.rs" "$work/without-report/program.js" >"$work/without-report/compile.log" 2>&1
JS_EXTRA_ARGS="$args js-opt-report=$work/repeat/report.json" "$script_dir/compile_core.sh" \
  "$root/examples/core-tests/$fixture.rs" "$work/repeat/program.js" >"$work/repeat/compile.log" 2>&1
cmp "$work/$fixture-on/program.js" "$work/without-report/program.js"
cmp "$work/$fixture-on/program.js" "$work/repeat/program.js"
cmp "$work/$fixture-on/report.json" "$work/repeat/report.json"
for fixture in 69_dead_call_dropped 75_repr_native_vec 107_box_pea_composition 108_standard_identity_owner; do
  cmp "$work/$fixture-off/program.js" "$work/$fixture-report/program.js"
done
python3 "$script_dir/operation-census-test.py" "$work"
python3 "$root/../../experiments-pro/compiler-v2/tools/test-summarize-jir-census.py"
echo 'operation census: PASS (captured identities, final closure, unchanged emission)'
