#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root="${JSC_SPIKE_ROOT:-$(cd -- "$script_dir/.." && pwd)}"
source "$root/scripts/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$root/scripts/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/host-function-values.XXXXXX")
echo "host function values: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro --locked) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
export JS_EXTRA_ARGS='js-profile=release-balanced js-opt=on js-modules=esm js-opt-warnings=off'
export JSC_SYSROOT_ARGS='js-profile=release-balanced js-opt=on'
"$root/scripts/build_sysroot.sh" >"$work/sysroot.log" 2>&1
"$root/scripts/compile_core.sh" "$root/mires-js/src/lib.rs" "$work/libmires_js.rlib" \
  --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/library.log" 2>&1
for lane in baseline jir; do
  mkdir "$work/$lane"
  program="$work/$lane/program.js"
  JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane js-lane-report=$work/$lane/lane.json" \
    "$root/scripts/compile_core.sh" "$root/examples/host_function_values.rs" "$program" \
    --crate-name host_function_values --extern "mires_js=$work/libmires_js.rlib" \
    --search "$work" --search "$root/target/debug" >"$work/$lane/compile.log" 2>&1
  node "$root/scripts/minimal-shim.mjs" --manifest "$program.artifacts.json" \
    --module "$root/runtime/shim.js" "$work/$lane/shim.js" >"$work/$lane/shim.log" 2>&1
  node "$script_dir/host-function-values-check.mjs" "$program" "$work/$lane/receipt.json" "$root"
done
