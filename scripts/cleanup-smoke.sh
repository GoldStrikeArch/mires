#!/usr/bin/env bash
# Active host exceptions: exact drops and thrown-value identity, with optimization on and off.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
work="$root/build/cleanup-smoke"
mkdir -p "$work/proc-macro"
if [ "${SKIP_BUILD:-0}" != 1 ]; then bash scripts/build.sh --locked; fi
cargo build -p js-extern-macro --locked >"$work/macro.log" 2>&1
case "$(uname -s)" in Darwin) ext=dylib ;; *) ext=so ;; esac
macro="$root/target/debug/libjs_extern_macro.$ext"
ln -sf "$macro" "$work/proc-macro/libjs_extern_macro.$ext"
node scripts/make-esm-shim.mjs "$work/shim.mjs"
for mode in on off; do
  export JS_EXTRA_ARGS="js-profile=release-balanced js-cleanup=on js-opt=$mode"
  SKIP_BUILD=1 bash scripts/build_sysroot.sh >"$work/sysroot-$mode.log" 2>&1
  bash scripts/compile_core.sh mires-js/src/lib.rs "$work/libmires_js-$mode.rlib" --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/lib-$mode.log" 2>&1
  JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=jir js-modules=esm js-shim-module=./shim.mjs js-source-map=on" \
    bash scripts/compile_core.sh examples/active_poll_cleanup.rs "$work/program-$mode.mjs" \
      --extern "mires_js=$work/libmires_js-$mode.rlib" --search "$work/proc-macro" >"$work/compile-$mode.log" 2>&1
  node scripts/active-poll-cleanup-check.mjs "$work/program-$mode.mjs" >"$work/result-$mode.json"
  node scripts/check-sourcemap.js "$work/program-$mode.mjs" >"$work/map-$mode.log"
  echo "cleanup enabled / optimizer $mode: 20 active-call/drop cases passed"
  # Zero-sized array elements have no storage; a throwing host call still owes each one its drop.
  bash scripts/compile_core.sh examples/core-tests/114_zst_aggregate_drops.rs "$work/zst-$mode.js" >"$work/zst-compile-$mode.log" 2>&1
  node - "$work/zst-$mode.js" runtime/shim.js >"$work/zst-$mode.json" <<'NODE'
const fs = require('node:fs')
const vm = require('node:vm')
const [program, shim] = process.argv.slice(2)
const context = vm.createContext({ TextDecoder, TextEncoder })
vm.runInContext(fs.readFileSync(shim, 'utf8'), context)
const events = []
context.__rt.js_log_f64 = (value) => events.push(Number(value))
const marker = new Error('host marker')
context.__rt.host_throw = () => { throw marker }
vm.runInContext(fs.readFileSync(program, 'utf8'), context)
// `rows: Vec<[Z<7>; 2]>` and then `held: [Z<2>; 3]`, each element exactly once, on both paths.
const expected = JSON.stringify([7, 7, 2, 2, 2])
const result = context.zst_host_throw(false)
const normal = JSON.stringify(events)
if (result !== 12 || normal !== expected) {
  throw new Error(`normal path: returned ${result}, dropped ${normal}`)
}
events.length = 0
let caught
try {
  context.zst_host_throw(true)
} catch (error) {
  caught = error
}
const thrown = JSON.stringify(events)
if (caught !== marker || thrown !== expected) {
  throw new Error(`host throw: caught ${caught === marker ? 'the marker' : String(caught)}, dropped ${thrown}`)
}
console.log(JSON.stringify({ normal: JSON.parse(normal), thrown: JSON.parse(thrown), identity: true }))
NODE
  echo "cleanup enabled / optimizer $mode: zero-sized array drops on a host throw passed"
done
