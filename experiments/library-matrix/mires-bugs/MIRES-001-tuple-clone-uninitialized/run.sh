#!/usr/bin/env bash
# Reproduces the bug through the matrix harness with a registry of its own (nothing in the real
# registry or receipts changes). Expect: build PASS, check FAIL with
# "TypeError: Cannot set properties of undefined (setting '0')".
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-001-tuple-clone-uninitialized"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language tuple-clone
node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language tuple-clone || true
grep -o 'function [^{]*{[^}]*b\[0\]=[^}]*}' "$here/port/dist/index.js" | head -3
