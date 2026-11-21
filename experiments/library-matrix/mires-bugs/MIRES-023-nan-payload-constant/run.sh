#!/usr/bin/env bash
# Reproduces MIRES-023 through the matrix harness with a registry of its own, then compiles the
# same crate once more with JSC_RUST_MIR_INLINE=on.
# Expect: build PASS, check FAIL on every case with "MISCOMPILED: NaN payload lost: const-f32
# const-f64" (the matrix profile, inlining off); with inlining on, the from_bits values are lost too.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-023-nan-payload-constant"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language nan-payload-const
if node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language nan-payload-const; then
  echo "Check passed. MIRES-023 did not reproduce."
  exit 1
fi
node --input-type=module -e "const { run } = await import('$here/port/dist/index.min.js'); console.log('matrix profile (inlining off):', run({ text: 'x' }))"
grep -o '[$A-Za-z_][$A-Za-z_0-9]*(NaN)' "$here/port/dist/index.js" | head -2

# The same program crate with MIR inlining on, against the mires_js rlib the build above compiled;
# every other setting is the matrix builder's.
compile="$receipts/work/floor/nan-payload-const/compile"
inline="$receipts/inline-on"
rm -rf "$inline"
mkdir -p "$inline"
args="js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off"
(cd "$repo" && env -u CARGO_INCREMENTAL -u JSC_PANIC JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on JSC_SYSROOT_ARGS="$args" \
  JS_EXTRA_ARGS="$args js-opt-warnings=off js-workspace-root=$repo js-modules=esm js-comments=off js-lane=jir js-entry-role=main" \
  scripts/compile_core.sh "$here/port/src/lib.rs" "$inline/program.js" --crate-name mires_bug_nan_payload_const \
  --extern "mires_js=$compile/libmires_js.rlib" --search "$compile/proc-macro" --search "$compile")
(cd "$inline" && node "$repo/scripts/minimal-shim.mjs" --module --manifest "$inline/program.js.artifacts.json" "$repo/runtime/shim.js" "$inline/shim.js")
node --input-type=module -e "const { matrix_run } = await import('$inline/program.js'); console.log('JSC_RUST_MIR_INLINE=on:', matrix_run({ text: 'x' }))"
