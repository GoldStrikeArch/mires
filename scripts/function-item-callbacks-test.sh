#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
work=$(mktemp -d "$root/build/function-item-callbacks.XXXXXX")
echo "function item callbacks: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
mkdir "$work/proc-macro"
ln -s "$macro" "$work/proc-macro/$(basename -- "$macro")"
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"
for mode in off report on; do
  export JS_EXTRA_ARGS="js-opt=$mode js-minify=on js-ecma-target=2018"
  SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$work/libmires_js.rlib" \
    --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/host-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$root/examples/function_item_callbacks/library.rs" "$work/libcallback_library.rlib" \
    --crate-type rlib --crate-name callback_library --extern "mires_js=$work/libmires_js.rlib" \
    --search "$work/proc-macro" >"$work/library-$mode.log" 2>&1
  for lane in baseline jir; do
    mkdir "$work/$lane-$mode"
    program="$work/$lane-$mode/program.mjs"
    JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane js-modules=esm js-shim-module=../shim.mjs js-source-map=on" \
      "$script_dir/compile_core.sh" "$root/examples/function_item_callbacks/main.rs" "$program" \
      --crate-name function_item_callbacks --extern "mires_js=$work/libmires_js.rlib" \
      --extern "callback_library=$work/libcallback_library.rlib" --search "$work" --search "$work/proc-macro" >"$program.compile.log" 2>&1
    node "$script_dir/function-item-callbacks-test.mjs" "$program"
    node "$script_dir/check-sourcemap.js" "$program" >"$program.map.log"
  done
done
cmp "$work/baseline-off/program.mjs" "$work/baseline-report/program.mjs"
cmp "$work/jir-off/program.mjs" "$work/jir-report/program.mjs"
echo 'function item callbacks: PASS'
