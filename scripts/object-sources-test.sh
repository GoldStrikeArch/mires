#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
work=$(mktemp -d "$root/build/object-sources.XXXXXX")
echo "object sources: retained output at $work"
mkdir "$work/source"
cat > "$work/library.rs" <<'RS'
#![no_std]
#[inline(never)]
pub fn external(x: u32) -> u32 {
    x.wrapping_add(7)
}
#[inline(never)]
pub fn generic<T: Copy>(x: T) -> T {
    x
}
RS
cat > "$work/source/main.rs" <<'RS'
#![no_std]
#![no_main]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
#[unsafe(no_mangle)]
pub extern "C" fn calculate(x: u32) -> u32 {
    provider::external(x).wrapping_add(provider::generic(x))
}
RS
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"
export JSC_REMAP_ROOT="$work/source"
for mode in off report on; do
  export JS_EXTRA_ARGS="js-opt=$mode js-minify=on js-ecma-target=2018"
  SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  cp "$work/library.rs" "$work/source/library.rs"
  JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-source-map=on" "$script_dir/compile_core.sh" "$work/source/library.rs" "$work/libprovider.rlib" \
    --crate-type rlib --crate-name provider >"$work/provider-$mode.log" 2>&1
  mv "$work/source/library.rs" "$work/removed-$mode.rs"
  for lane in baseline jir; do
    mkdir "$work/$lane-$mode"
    program="$work/$lane-$mode/program.mjs"
    JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$lane js-modules=esm js-shim-module=../shim.mjs js-source-map=on" \
      "$script_dir/compile_core.sh" "$work/source/main.rs" "$program" --crate-name object_sources --extern "provider=$work/libprovider.rlib" \
      >"$program.compile.log" 2>&1
    node "$script_dir/object-sources-test.mjs" "$program" "$work/library.rs"
    node "$script_dir/check-sourcemap.js" "$program" >"$program.map.log"
  done
done
cmp "$work/baseline-off/program.mjs" "$work/baseline-report/program.mjs"
cmp "$work/jir-off/program.mjs" "$work/jir-report/program.mjs"
python3 "$script_dir/object-sources-incremental.py" "$work/incremental"
echo 'object sources: PASS'
