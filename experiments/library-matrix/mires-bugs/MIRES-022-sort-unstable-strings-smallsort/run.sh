#!/usr/bin/env bash
# Reproduces MIRES-022 through the matrix harness with a registry of its own.
# Expect: build PASS, check FAIL on every case with "`copy_nonoverlapping` over 1 elements at N
# reaches past the end of a buffer of 1".
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-022-sort-unstable-strings-smallsort"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language sort-unstable-strings
if node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language sort-unstable-strings; then
  echo "Check passed. MIRES-022 did not reproduce."
  exit 1
fi
grep -o 'copy_nonoverlapping[^"]*' "$receipts/checks/floor--sort-unstable-strings--node.json" | head -1
