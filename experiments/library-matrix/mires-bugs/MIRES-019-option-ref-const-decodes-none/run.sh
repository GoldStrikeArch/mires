#!/usr/bin/env bash
# Reproduces MIRES-019 through the matrix harness with a registry of its own.
# Expect: build PASS, check FAIL on `ascii` ("MISCOMPILED: Some(&b'.') decoded as None").
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-019-option-ref-const-decodes-none"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language option-ref-const
node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language option-ref-const || true
grep 'd="None",e=void 0' "$here/port/dist/index.js" | head -3
