#!/usr/bin/env bash
# packages.sh: both packages + kernels from the verifier worktree; sizes into pkg/sizes.txt
set -uo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-a3724c72be1271b05
P=$V/pkg
mkdir -p "$P"
: > "$P/sizes.txt"
cd "$W"
sizes() {
  node -e '
const fs = require("fs"), zlib = require("zlib"), crypto = require("crypto");
const b = fs.readFileSync(process.argv[1]);
const gz = zlib.gzipSync(b, { level: 9 }).length;
const br = zlib.brotliCompressSync(b, { params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 } }).length;
console.log(`${b.length} / ${gz} / ${br} sha256=${crypto.createHash("sha256").update(b).digest("hex")}`);' "$1"
}
node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust-compact/dist/va4r2 > "$P/compact-build.log" 2>&1
echo "compact build exit $?" >> "$P/sizes.txt"
node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust-compact/dist/va4r2/fp.esm.min.js > "$P/compact-kernels.log" 2>&1
echo "compact kernels exit $? $(tail -1 "$P/compact-kernels.log")" >> "$P/sizes.txt"
echo "compact sizes $(sizes experiments/fingerprintjs-rust-compact/dist/va4r2/fp.esm.min.js)" >> "$P/sizes.txt"
node experiments/fingerprintjs-rust/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust/dist/va4r2 > "$P/idiomatic-build.log" 2>&1
echo "idiomatic build exit $?" >> "$P/sizes.txt"
node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust/dist/va4r2/boundary/fp.esm.min.js > "$P/idiomatic-kernels.log" 2>&1
echo "idiomatic kernels exit $? $(tail -1 "$P/idiomatic-kernels.log")" >> "$P/sizes.txt"
echo "idiomatic sizes $(sizes experiments/fingerprintjs-rust/dist/va4r2/boundary/fp.esm.min.js)" >> "$P/sizes.txt"
cp experiments/fingerprintjs-rust-compact/dist/va4r2/fp.esm.min.js "$P/compact.fp.esm.min.js"
cp experiments/fingerprintjs-rust/dist/va4r2/boundary/fp.esm.min.js "$P/idiomatic.fp.esm.min.js"
echo done >> "$P/sizes.txt"
