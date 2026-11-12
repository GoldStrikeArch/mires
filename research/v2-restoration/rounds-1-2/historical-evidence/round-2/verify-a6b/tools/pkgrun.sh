#!/usr/bin/env bash
# pkgrun.sh: both FingerprintJS packages, built as COMMON.md prescribes from the worktree, with
# every link done by both linkers (tools/abrun.sh), then the kernel checks and the sizes of
# fp.esm.min.js (minified / gzip -9 / brotli 11). Summary in gates/packages.txt.
# The worktree has no node_modules of its own; a symbolic link to the main checkout's (read only)
# stands in for `npm ci` and is removed again at the end.
set -uo pipefail
source "$(dirname "$0")/env.sh"
G=$V/gates
cd "$W" || exit 2
[ -e node_modules ] || ln -s /Users/mikhail.pertsev/Desktop/OSS/mires/node_modules node_modules
# Likewise the upstream FingerprintJS build the kernels are compared against (its dist/ is a build output).
[ -e experiments/fingerprintjs/dist ] || ln -s /Users/mikhail.pertsev/Desktop/OSS/mires/experiments/fingerprintjs/dist experiments/fingerprintjs/dist
bash "$V/tools/abrun.sh" gate-pkg-compact node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust-compact/dist/va6b > "$G/pkg-compact.line" 2>&1
bash "$V/tools/abrun.sh" gate-pkg-idiomatic node experiments/fingerprintjs-rust/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust/dist/va6b > "$G/pkg-idiomatic.line" 2>&1
sizes() {
  node -e '
const fs = require("fs"), zlib = require("zlib");
const b = fs.readFileSync(process.argv[1]);
const gz = zlib.gzipSync(b, { level: 9 }).length;
const br = zlib.brotliCompressSync(b, { params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 } }).length;
console.log(`${b.length} / ${gz} / ${br}`);' "$1"
}
{
  cat "$G/pkg-compact.line" "$G/pkg-idiomatic.line"
  node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust-compact/dist/va6b/fp.esm.min.js > "$G/compact-kernels.log" 2>&1
  echo "compact kernels exit $? $(tail -1 "$G/compact-kernels.log")"
  echo "compact sizes $(sizes experiments/fingerprintjs-rust-compact/dist/va6b/fp.esm.min.js 2>&1 | tail -1)"
  node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust/dist/va6b/boundary/fp.esm.min.js > "$G/idiomatic-kernels.log" 2>&1
  echo "idiomatic kernels exit $? $(tail -1 "$G/idiomatic-kernels.log")"
  echo "idiomatic sizes $(sizes experiments/fingerprintjs-rust/dist/va6b/boundary/fp.esm.min.js 2>&1 | tail -1)"
} > "$G/packages.txt" 2>&1
[ -L node_modules ] && rm node_modules
[ -L experiments/fingerprintjs/dist ] && rm experiments/fingerprintjs/dist
cat "$G/packages.txt"
