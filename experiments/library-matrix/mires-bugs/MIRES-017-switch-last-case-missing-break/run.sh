#!/usr/bin/env bash
# Reproduces MIRES-017 through the matrix harness with a registry of its own.
# Expect: build PASS, check FAIL with "Error: rustc_codegen_js: bad block".
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-017-switch-last-case-missing-break"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language switch-missing-break
node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language switch-missing-break || true
grep -o 'case 2:.*bad block[^)]*)' "$here/port/dist/index.js" | head -3
