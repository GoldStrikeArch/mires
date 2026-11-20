#!/usr/bin/env bash
# Builds the repro through the matrix harness with its own registry and receipts, then calls the
# artifact directly for each construction. Expect: build PASS, every call throws
# "rustc_codegen_js: this pointer does not name a live heap block, so it cannot be deallocated".
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-002-boxed-slice-dealloc"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language boxed-slice
node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language boxed-slice || true
node --input-type=module -e "
const { run } = await import('$here/port/dist/index.min.js')
for (const c of [0, 1, 2]) { try { run({ text: 'hello', case: c }); console.log('case', c, 'ok') } catch (e) { console.log('case', c, 'threw:', e.message) } }"
