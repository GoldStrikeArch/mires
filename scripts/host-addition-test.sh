#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/host-addition.XXXXXX")
echo "host addition: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro --locked) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
for names in readable mangled; do
  for minify in on off; do
    variant="$work/$names-$minify"
    mkdir "$variant"
    export JSC_SYSROOT_ARGS="js-profile=release-balanced js-opt=on js-names=$names js-minify=$minify"
    export JS_EXTRA_ARGS="$JSC_SYSROOT_ARGS js-modules=esm js-opt-warnings=off"
    "$script_dir/build_sysroot.sh" >"$variant/sysroot.log" 2>&1
    "$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$variant/libmires_js.rlib" \
      --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$variant/library.log" 2>&1
    for lane in baseline jir; do
      output="$variant/$lane"
      mkdir "$output"
      JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane js-source-map=on js-lane-report=$output/lane.json" \
        "$script_dir/compile_core.sh" "$root/examples/host_addition.rs" "$output/program.js" \
        --crate-name host_addition --extern "mires_js=$variant/libmires_js.rlib" \
        --search "$variant" --search "$root/target/debug" >"$output/compile.log" 2>&1
      node "$script_dir/minimal-shim.mjs" --manifest "$output/program.js.artifacts.json" \
        --module "$root/runtime/shim.js" "$output/shim.js" >"$output/shim.log" 2>&1
      node "$script_dir/check-sourcemap.js" "$output/program.js" >"$output/sourcemap.log" 2>&1
      node "$script_dir/host-addition-check.mjs" "$output/program.js" "$output/receipt.json" "$root"
    done
  done
done
