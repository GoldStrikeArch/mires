#!/usr/bin/env bash
# Reproduces MIRES-024 through the matrix harness with a registry of its own.
# Expect: build FAIL with "constants of type `fn(u32) -> u32 {twice}` cannot be read out of an
# allocation" (error.txt).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-024-promoted-fn-item-ref"
cd "$repo/experiments/library-matrix"
if env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language fn-item-ref --clean; then
  echo "Compiled successfully. MIRES-024 did not reproduce."
  exit 1
fi
grep -F 'cannot be read out of an allocation' "$receipts/logs/floor--fn-item-ref.build.log"
