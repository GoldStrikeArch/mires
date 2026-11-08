#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root="${JSC_SPIKE_ROOT:-$(cd -- "$script_dir/.." && pwd)}"
source "$root/scripts/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$root/scripts/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/host-callback4.XXXXXX")
echo "host callback4: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro --locked) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
for optimizer in off report on; do
  export JS_EXTRA_ARGS="js-profile=release-balanced js-opt=$optimizer js-modules=esm js-opt-warnings=off"
  export JSC_SYSROOT_ARGS="js-profile=release-balanced js-opt=$optimizer"
  library="$work/library-$optimizer"
  mkdir "$library"
  "$root/scripts/build_sysroot.sh" >"$library/sysroot.log" 2>&1
  "$root/scripts/compile_core.sh" "$root/mires-js/src/lib.rs" "$library/libmires_js.rlib" \
    --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$library/compile.log" 2>&1
  for lane in baseline jir; do
    directory="$work/$lane-$optimizer"
    mkdir "$directory"
    program="$directory/program.js"
    JS_EXTRA_ARGS="js-profile=release-balanced js-opt=$optimizer js-modules=esm js-opt-warnings=off js-lane=$lane js-lane-report=$directory/lane.json" \
      "$root/scripts/compile_core.sh" "$root/examples/host_callback4.rs" "$program" \
      --crate-name host_callback4 --extern "mires_js=$library/libmires_js.rlib" \
      --search "$library" --search "$root/target/debug" >"$directory/compile.log" 2>&1
    node "$root/scripts/minimal-shim.mjs" --manifest "$program.artifacts.json" \
      --module "$root/runtime/shim.js" "$directory/shim.js" >"$directory/shim.log" 2>&1
    node "$script_dir/host-callback4-check.mjs" "$program" "$directory/receipt.json" "$root"
  done
done
for lane in baseline jir; do
  cmp "$work/$lane-off/program.js" "$work/$lane-report/program.js"
done
