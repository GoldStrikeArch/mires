#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-020-closure-spread-cleanup"
cd "$repo/experiments/library-matrix"
if node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language closure-spread --clean; then
  echo "Compiled successfully. MIRES-020 did not reproduce."
  exit 1
fi
grep -F 'reason: SpreadNotTuple' "$receipts/logs/floor--closure-spread.build.log"
