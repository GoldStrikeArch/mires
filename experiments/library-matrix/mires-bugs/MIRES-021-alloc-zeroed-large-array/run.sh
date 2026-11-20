#!/usr/bin/env bash
# Reproduces MIRES-021 through the matrix harness with a registry of its own.
# Expect: build PASS, check FAIL on every case with "RangeError: Invalid array length" (each call
# spends a few seconds and a few GB of memory inside `new Array(n).fill(0)` before V8 refuses it).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-021-alloc-zeroed-large-array"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language alloc-zeroed-large
if node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language alloc-zeroed-large; then
  echo "Check passed. MIRES-021 did not reproduce."
  exit 1
fi
grep -o 'Invalid array length' "$receipts/checks/floor--alloc-zeroed-large--node.json" | head -1
