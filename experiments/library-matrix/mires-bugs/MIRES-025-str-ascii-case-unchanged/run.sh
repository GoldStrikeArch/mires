#!/usr/bin/env bash
# Reproduces MIRES-025 through the matrix harness with a registry of its own.
# Expect: build PASS, check FAIL on every case ("MISCOMPILED: str::to_ascii_lowercase
# returned the text unchanged"; `empty` fails on its rewritten call, whose text has letters).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
receipts="$repo/build/library-matrix/mires-bug-repros/MIRES-025-str-ascii-case-unchanged"
cd "$repo/experiments/library-matrix"
env -u CARGO_INCREMENTAL node tools/build.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language str-ascii-case
if node tools/check.mjs --matrix "$here/matrix.json" --receipts "$receipts" --library floor --language str-ascii-case; then
  echo "Check passed. MIRES-025 did not reproduce."
  exit 1
fi
node --input-type=module -e "const { run } = await import('$here/port/dist/index.min.js'); console.log(run({ text: 'The Quick Brown Fox' }))"
