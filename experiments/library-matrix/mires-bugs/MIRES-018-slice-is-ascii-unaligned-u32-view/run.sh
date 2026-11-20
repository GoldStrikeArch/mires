#!/usr/bin/env bash
# Reproduces MIRES-018 through the matrix harness with a registry of its own.
# Expect: build PASS, check FAIL with "rustc_codegen_js: byte offset 39 cannot form a u32 scalar view".
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-018-slice-is-ascii-unaligned-u32-view"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language slice-is-ascii
node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language slice-is-ascii || true
grep -o 'byte offset [0-9]* cannot form a u32 scalar view' "$receipts/checks/floor--slice-is-ascii--node.json" | head -3
