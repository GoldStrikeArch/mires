#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
export JSC_JIR_SELFCHECK=1
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
work=$(mktemp -d "$root/build/suspension-facts.XXXXXX")
echo "suspension facts: retained output at $work"
for mode in off report on; do
  export JS_EXTRA_ARGS="js-opt=$mode js-minify=on js-ecma-target=2018"
  SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$root/examples/cross-crate-tests/suspension_provider.rs" "$work/libsuspension_provider.rlib" \
    --crate-type rlib --crate-name suspension_provider >"$work/provider-$mode.log" 2>&1
  grep -q '^source-mir-selfcheck: ordinary ' "$work/provider-$mode.log"
  grep -q '^source-mir-selfcheck: coroutine ' "$work/provider-$mode.log"
  cp "$work/libsuspension_provider.rlib" "$work/provider-$mode.rlib"
  for lane in baseline jir; do
    mkdir "$work/$lane-$mode"
    for fixture in local generic liveness initialization; do
      program="$work/$lane-$mode/$fixture.js"
      if [ "$fixture" = local ]; then
        source_file="$root/examples/core-tests/26_async.rs"
        extra=()
      elif [ "$fixture" = initialization ]; then
        source_file="$root/examples/core-tests/112_async_initialization.rs"
        extra=()
      elif [ "$fixture" = liveness ]; then
        source_file="$root/examples/core-tests/111_async_source_liveness.rs"
        extra=()
      else
        source_file="$root/examples/cross-crate-tests/suspension_client.rs"
        extra=(--extern "suspension_provider=$work/libsuspension_provider.rlib")
      fi
      JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane" "$script_dir/compile_core.sh" "$source_file" "$program" ${extra[@]+"${extra[@]}"} >"$program.compile.log" 2>&1
      grep -q '^source-mir-selfcheck: ordinary ' "$program.compile.log"
      if [ "$fixture" != generic ]; then
        grep -q '^source-mir-selfcheck: coroutine ' "$program.compile.log"
      fi
      cat "$root/runtime/shim.js" "$program" "$script_dir/entry.js" | node - >"$program.stdout"
      if [ "$fixture" != generic ]; then
        diff -u "${source_file%.rs}.expected" "$program.stdout"
      else
        diff -u <(printf '7\ngeneric\n9\n11\n') "$program.stdout"
      fi
      if [ "$lane" = jir ]; then
        node "$script_dir/suspension-facts-test.mjs" "$program.lane.json" "$fixture" >"$program.facts.json"
      fi
      echo "$lane/$mode/$fixture: passed"
    done
  done
done
for lane in baseline jir; do
  for fixture in local generic liveness initialization; do
    cmp "$work/$lane-off/$fixture.js" "$work/$lane-report/$fixture.js"
  done
done
sysroot=$(sed -n 's/^==> .*sysroot: //p' "$work/sysroot-off.log" | head -n 1)
python3 "$script_dir/suspension-facts-incremental.py" "$work/incremental" "$sysroot"
echo 'suspension facts: PASS (report only, generic library metadata, exact off/report JavaScript)'
