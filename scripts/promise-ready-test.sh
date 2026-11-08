#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
work=$(mktemp -d "$root/build/promise-ready.XXXXXX")
echo "promise ready: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
mkdir "$work/proc-macro"
ln -s "$macro" "$work/proc-macro/$(basename -- "$macro")"
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"
compile() {
  local directory=$1 kind=$2 lane=$3 ready=$4
  mkdir -p "$work/$directory"
  local extra=()
  if [ "$kind" != ready ]; then extra=(--cfg "$kind"); fi
  JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane js-modules=esm js-shim-module=../shim.mjs js-source-map=on js-jir-ready-futures=$ready" \
    "$script_dir/compile_core.sh" "$root/examples/promise_ready.rs" "$work/$directory/program.mjs" \
    --extern "mires_js=$work/libmires_js.rlib" --search "$work/proc-macro" ${extra[@]+"${extra[@]}"} >"$work/$directory/compile.log" 2>&1
  node "$script_dir/promise-ready-check.mjs" "$work/$directory/program.mjs" "$kind" >"$work/$directory/result.json"
  node "$script_dir/check-sourcemap.js" "$work/$directory/program.mjs" >"$work/$directory/map.log"
}
for mode in off report on; do
  export JS_EXTRA_ARGS="js-opt=$mode js-minify=on js-ecma-target=2018"
  SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$work/libmires_js.rlib" \
    --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/library-$mode.log" 2>&1
  for lane in baseline jir; do
    compile "$lane-$mode" ready "$lane" on
    echo "$lane/$mode: ready boundary passed"
  done
done
compile jir-control ready jir off
JSC_JIR_ABI_FAULT=promise_ready compile jir-retry ready jir on
for kind in custom suspending local_drop captured_drop helper_drop borrowed mixed; do
  compile "$kind" "$kind" jir on
done
JSC_RUST_MIR_INLINE=on compile inlined_helper inlined_helper jir on
for lane in baseline jir; do
  cmp "$work/$lane-off/program.mjs" "$work/$lane-report/program.mjs"
done
node "$script_dir/promise-ready-results.mjs" "$work"
echo 'promise ready: PASS (compiled driver, decline controls, exact off/report JavaScript)'
