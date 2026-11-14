#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
work=$(mktemp -d "$root/build/task-cleanup.XXXXXX")
echo "task cleanup: retained output at $work"
library_root=${TASK_CLEANUP_LIBRARY_ROOT:-$root}
(cd "$root" && cargo build -p js-extern-macro) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) suffix=dylib ;;
*) suffix=so ;;
esac
macro="$root/target/debug/libjs_extern_macro.$suffix"
mkdir "$work/proc-macro"
ln -s "$macro" "$work/proc-macro/$(basename -- "$macro")"
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"
cp "$root/runtime/dom/dist/topcoat-dom.js" "$work/topcoat-dom.mjs"
skipped=0
for mode in off report on; do
  export JS_EXTRA_ARGS="js-opt=$mode js-minify=on js-ecma-target=2018 js-dom-module=../topcoat-dom.mjs"
  SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$library_root/mires-js/src/lib.rs" "$work/libmires_js.rlib" \
    --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/topcoat-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$library_root/view-abi/src/lib.rs" "$work/libview_abi.rlib" \
    --crate-type rlib --crate-name view_abi >"$work/abi-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$library_root/view-async/src/lib.rs" "$work/libview_async.rlib" \
    --crate-type rlib --crate-name view_async --extern "view_abi=$work/libview_abi.rlib" >"$work/async-$mode.log" 2>&1
  mkdir "$work/library-$mode"
  cp "$work/libmires_js.rlib" "$work/libview_abi.rlib" "$work/libview_async.rlib" "$work/library-$mode/"
  for lane in baseline jir; do
    mkdir "$work/$lane-$mode"
    program="$work/$lane-$mode/program.mjs"
    compile_status=0
    JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane js-modules=esm js-shim-module=../shim.mjs js-source-map=on" \
      "$script_dir/compile_core.sh" "$root/examples/task_cleanup.rs" "$program" \
      --extern "js_extern_macro=$macro" --extern "mires_js=$work/libmires_js.rlib" \
      --extern "view_abi=$work/libview_abi.rlib" --extern "view_async=$work/libview_async.rlib" \
      --search "$work" --search "$work/proc-macro" >"$program.compile.log" 2>&1 || compile_status=$?
    if [ "$compile_status" != 0 ]; then
      # JSC_PANIC=unwind: rustc's optimized MIR keeps cleanup blocks, which only the JIR lane lowers,
      # so the baseline lane refuses such a program (CONTRACT.md). That refusal is a skip, printed
      # and counted; any other failure fails the suite as before.
      if [ "${JSC_PANIC:-}" = unwind ] && [ "$lane" = baseline ] && [ ! -e "$program" ] &&
        grep -q 'requires semantic cleanup lowering' "$program.compile.log"; then
        echo "$lane/$mode: SKIP (JSC_PANIC=unwind: the baseline lane cannot build a program with cleanup; refused with 'requires semantic cleanup lowering')"
        skipped=$((skipped + 1))
        continue
      fi
      cat "$program.compile.log" >&2
      exit "$compile_status"
    fi
    status=0
    node "$script_dir/task-cleanup-check.mjs" "$program" "$work/topcoat-dom.mjs" >"$program.result.json" || status=$?
    if [ "${TASK_CLEANUP_CONTROL:-0}" = 1 ] && [ "$mode" != on ]; then
      [ "$status" = 1 ] || { echo "original Rust driver unexpectedly passed"; exit 1; }
    else
      [ "$status" = 0 ] || { cat "$program.result.json"; exit 1; }
    fi
    node "$script_dir/check-sourcemap.js" "$program" >"$program.map.log"
    if [ "$lane" = jir ]; then
      node -e 'const r=require(process.argv[1]); if(r.fallback_count!==0) throw Error("JIR fallback")' "$program.lane.json"
    fi
    echo "$lane/$mode: task cleanup checked"
  done
done
for lane in baseline jir; do
  # A lane whose rows were all skipped (above) has no programs to compare.
  [ -e "$work/$lane-off/program.mjs" ] || [ -e "$work/$lane-report/program.mjs" ] || continue
  cmp "$work/$lane-off/program.mjs" "$work/$lane-report/program.mjs"
done
[ "$skipped" = 0 ] || echo "task cleanup: $skipped row(s) skipped (JSC_PANIC=unwind, baseline lane)"
echo 'task cleanup: PASS (compiled Rust and optimized drivers, terminal errors, owners and cancellation)'
