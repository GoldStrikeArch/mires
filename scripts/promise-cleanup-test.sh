#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/promise-cleanup.XXXXXX")
echo "promise cleanup: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
mkdir -p "$work/proc-macro"
ln -s "$macro" "$work/proc-macro/$(basename -- "$macro")"
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"
skipped=0
for mode in off report on; do
  export JS_EXTRA_ARGS="js-opt=$mode js-minify=on js-ecma-target=2018"
  SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$work/libmires_js.rlib" \
    --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/library-$mode.log" 2>&1
  for lane in baseline jir; do
    mkdir -p "$work/$lane-$mode"
    program="$work/$lane-$mode/program.mjs"
    compile_status=0
    JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane js-modules=esm js-shim-module=../shim.mjs js-source-map=on" \
      "$script_dir/compile_core.sh" "$root/examples/promise_cleanup.rs" "$program" \
      --extern "mires_js=$work/libmires_js.rlib" --search "$work/proc-macro" >"$program.compile.log" 2>&1 ||
      compile_status=$?
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
    node "$script_dir/promise-cleanup-check.mjs" "$program" >"$program.result.json"
    echo "$lane/$mode: Promise cleanup passed"
    node "$script_dir/check-sourcemap.js" "$program" >"$program.map.log"
  done
done
for lane in baseline jir; do
  # A lane whose rows were all skipped (above) has no programs to compare.
  [ -e "$work/$lane-off/program.mjs" ] || [ -e "$work/$lane-report/program.mjs" ] || continue
  cmp "$work/$lane-off/program.mjs" "$work/$lane-report/program.mjs"
done
[ "$skipped" = 0 ] || echo "promise cleanup: $skipped row(s) skipped (JSC_PANIC=unwind, baseline lane)"
