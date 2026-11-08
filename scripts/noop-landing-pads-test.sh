#!/usr/bin/env bash
# No-op landing pad removal: exact drops and thrown-value identity with MIR inlining and the
# optimizer each on and off.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
work="$root/build/noop-landing-pads"
mkdir -p "$work/proc-macro"
if [ "${SKIP_BUILD:-0}" != 1 ]; then bash scripts/build.sh --locked >/dev/null; fi
cargo build -p js-extern-macro --locked >"$work/macro.log" 2>&1
case "$(uname -s)" in Darwin) ext=dylib ;; *) ext=so ;; esac
macro="$root/target/debug/libjs_extern_macro.$ext"
ln -sf "$macro" "$work/proc-macro/libjs_extern_macro.$ext"
node scripts/make-esm-shim.mjs "$work/shim.mjs"
for mode in on off; do
  export JS_EXTRA_ARGS="js-profile=release-balanced js-cleanup=on js-opt=$mode"
  SKIP_BUILD=1 bash scripts/build_sysroot.sh >"$work/sysroot-$mode.log" 2>&1
  bash scripts/compile_core.sh mires-js/src/lib.rs "$work/libmires_js-$mode.rlib" --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/lib-$mode.log" 2>&1
  for inline in off on; do
    program="$work/program-$mode-inline-$inline.mjs"
    JSC_RUST_MIR_INLINE=$inline JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=jir js-modules=esm js-shim-module=./shim.mjs" \
      bash scripts/compile_core.sh examples/noop_landing_pads.rs "$program" \
        --extern "mires_js=$work/libmires_js-$mode.rlib" --search "$work/proc-macro" >"$program.log" 2>&1
    node scripts/noop-landing-pads-check.mjs "$program" >"$program.json"
    echo "optimizer $mode / MIR inlining $inline: no-op landing pad cases passed"
  done
done
