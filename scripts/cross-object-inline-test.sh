#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
work=$(mktemp -d "$root/build/cross-object-inline.XXXXXX")
echo "cross-object inline: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
mkdir "$work/proc-macro"
ln -s "$macro" "$work/proc-macro/$(basename -- "$macro")"
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"
for mode in off report on on-long; do
  mkdir "$work/$mode"
  export JS_EXTRA_ARGS="js-opt=${mode%-long} js-minify=on js-ecma-target=2018 js-source-map=on"
  SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$work/$mode/libmires_js.rlib" \
    --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/library-$mode.log" 2>&1
  provider_name=cross_inline_provider
  if [ "$mode" = on-long ]; then provider_name=cross_inline_provider_with_a_deliberately_long_crate_name; fi
  "$script_dir/compile_core.sh" "$root/examples/cross-crate-tests/inline_provider.rs" "$work/$mode/libcross_inline_provider.rlib" \
    --crate-type rlib --crate-name "$provider_name" \
    --extern "mires_js=$work/$mode/libmires_js.rlib" \
    --search "$work/proc-macro" >"$work/provider-$mode.log" 2>&1
  program="$work/$mode/program.mjs"
  JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=jir js-modules=esm js-shim-module=../shim.mjs js-source-map=on" \
    "$script_dir/compile_core.sh" "$root/examples/cross-crate-tests/inline_client.rs" "$program" \
    --extern "mires_js=$work/$mode/libmires_js.rlib" \
    --extern "cross_inline_provider=$work/$mode/libcross_inline_provider.rlib" \
    --search "$work/proc-macro" --search "$work/$mode" >"$work/client-$mode.log" 2>&1
  node "$script_dir/cross-object-inline-test.mjs" "$program" "${mode%-long}"
  node "$script_dir/check-sourcemap.js" "$program" >"$work/map-$mode.log"
  if [ "${mode%-long}" = on ]; then
    origin_line=$(rg -n 'pair.0 != pair.1' "$root/examples/cross-crate-tests/inline_provider.rs")
    node "$script_dir/check-sourcemap.js" "$program" inline_provider.rs '!==7' "${origin_line%%:*}"
    if node "$script_dir/check-sourcemap.js" "$program" inline_client.rs '!==7' >"$work/map-negative.log" 2>&1; then
      echo 'source-map negative control unexpectedly passed' >&2
      exit 1
    fi
  fi
done
node --input-type=module - "$work" <<'JS'
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const root = process.argv[2];
const inline = mode => JSON.parse(readFileSync(`${root}/${mode}/program.mjs.lane.json`)).inline;
assert.deepEqual(inline('on-long'), inline('on'), 'symbol length changed inlining decisions or measured bytes');
JS
cmp "$work/off/program.mjs" "$work/report/program.mjs"
echo 'cross-object inline: PASS (off/report byte identity)'
