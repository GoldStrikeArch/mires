#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/active-poll-cleanup.XXXXXX")
echo "active poll cleanup: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro --locked) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
mkdir -p "$work/proc-macro"
ln -s "$macro" "$work/proc-macro/$(basename -- "$macro")"
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"
failures=0
for mode in off report on; do
  export JS_EXTRA_ARGS="js-opt=$mode js-minify=on js-ecma-target=2018"
  SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$work/libmires_js.rlib" \
    --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/library-$mode.log" 2>&1
  for lane in baseline jir; do
    mkdir -p "$work/$lane-$mode"
    program="$work/$lane-$mode/program.mjs"
    JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane js-modules=esm js-shim-module=../shim.mjs js-source-map=on" \
      "$script_dir/compile_core.sh" "$root/examples/active_poll_cleanup.rs" "$program" \
      --extern "mires_js=$work/libmires_js.rlib" --search "$work/proc-macro" >"$program.compile.log" 2>&1
    if node "$script_dir/active-poll-cleanup-check.mjs" "$program" >"$program.result.json" 2>"$program.result.log"; then
      echo "$lane/$mode: active poll cleanup passed"
    else
      echo "$lane/$mode: active poll cleanup FAILED" >&2
      failures=$((failures + 1))
    fi
    node "$script_dir/check-sourcemap.js" "$program" >"$program.map.log"
  done
done
for lane in baseline jir; do
  cmp "$work/$lane-off/program.mjs" "$work/$lane-report/program.mjs"
done
test "$failures" = 0
