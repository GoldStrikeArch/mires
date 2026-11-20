#!/usr/bin/env bash
# Reproduces MIRES-020 through the matrix harness with a registry of its own.
# Expect: build PASS and check PASS (fixed). A compiler with the bug fails the build with
# `reason: SpreadNotTuple`, and this script fails with it.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-020-closure-spread-cleanup"
cd "$repo/experiments/library-matrix"
if ! env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language closure-spread --clean; then
  grep -F 'reason: SpreadNotTuple' "$receipts/logs/floor--closure-spread.build.log" || true
  echo "Build failed. MIRES-020 reproduced."
  exit 1
fi
node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language closure-spread --readable
echo "Build PASS and check PASS: MIRES-020 is fixed."
