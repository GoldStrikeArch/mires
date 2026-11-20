#!/usr/bin/env bash
# Builds the repro through the matrix harness with its own registry and receipts (nothing in the
# real registry or receipts changes). Expect: the build FAILS; the rustc_codegen_js errors are in
# receipts/logs/floor--in-place-collect.build.log (copied to error.txt by the last run).
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-003-in-place-collect"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language in-place-collect
grep -A12 '^error' "$here/receipts/logs/floor--in-place-collect.build.log" | grep -v '^    at ' > "$here/error.txt"
cat "$here/error.txt"
