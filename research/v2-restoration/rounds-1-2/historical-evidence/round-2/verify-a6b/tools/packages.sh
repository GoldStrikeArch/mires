#!/usr/bin/env bash
# packages.sh <base|fix>: builds both FingerprintJS packages as COMMON.md prescribes with the
# cf80163 linker (base, through JSC_LINK_BIN) or the linker under test (fix), same dylib and
# sysroot, checks the kernels, and records minified / gzip / brotli of fp.esm.min.js in
# pkg-<which>/sizes.txt, keeping a copy of each fp.esm.min.js for a byte compare.
set -uo pipefail
source "$(dirname "$0")/env.sh"
which=$1
if [ "$which" = base ]; then export JSC_LINK_BIN=$V/tools/base-logged.sh; else unset JSC_LINK_BIN; fi
tag=va6b-$which
P=$V/pkg-$which
rm -rf "$P"; mkdir -p "$P"
cd "$W" || exit 2
sizes() {
  node -e '
const fs = require("fs"), zlib = require("zlib");
const b = fs.readFileSync(process.argv[1]);
const gz = zlib.gzipSync(b, { level: 9 }).length;
const br = zlib.brotliCompressSync(b, { params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 } }).length;
console.log(`${b.length} / ${gz} / ${br}`);' "$1"
}
echo "linker ${JSC_LINK_BIN:-$FIX_LINK}" > "$P/sizes.txt"
node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output "experiments/fingerprintjs-rust-compact/dist/$tag" > "$P/compact-build.log" 2>&1
echo "compact build exit $?" >> "$P/sizes.txt"
node experiments/tools/check-oss-kernels.mjs "experiments/fingerprintjs-rust-compact/dist/$tag/fp.esm.min.js" > "$P/compact-kernels.log" 2>&1
echo "compact kernels exit $? $(tail -1 "$P/compact-kernels.log")" >> "$P/sizes.txt"
echo "compact sizes $(sizes "experiments/fingerprintjs-rust-compact/dist/$tag/fp.esm.min.js")" >> "$P/sizes.txt"
node experiments/fingerprintjs-rust/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output "experiments/fingerprintjs-rust/dist/$tag" > "$P/idiomatic-build.log" 2>&1
echo "idiomatic build exit $?" >> "$P/sizes.txt"
node experiments/tools/check-oss-kernels.mjs "experiments/fingerprintjs-rust/dist/$tag/boundary/fp.esm.min.js" > "$P/idiomatic-kernels.log" 2>&1
echo "idiomatic kernels exit $? $(tail -1 "$P/idiomatic-kernels.log")" >> "$P/sizes.txt"
echo "idiomatic sizes $(sizes "experiments/fingerprintjs-rust/dist/$tag/boundary/fp.esm.min.js")" >> "$P/sizes.txt"
cp "experiments/fingerprintjs-rust-compact/dist/$tag/fp.esm.min.js" "$P/compact.fp.esm.min.js"
cp "experiments/fingerprintjs-rust/dist/$tag/boundary/fp.esm.min.js" "$P/idiomatic.fp.esm.min.js"
echo done >> "$P/sizes.txt"
cat "$P/sizes.txt"
