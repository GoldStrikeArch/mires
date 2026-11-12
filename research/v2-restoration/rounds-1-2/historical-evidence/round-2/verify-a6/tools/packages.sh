#!/usr/bin/env bash
# chunk-test in four configurations, then both packages (fix linker, then main's linker), in the
# verifier's worktree. Logs under verify-a6/gates and verify-a6/pkg-{fix,base}.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-aa8b8ec6c06479cb8
cd "$W" || exit 2
sizes() {
  node -e '
const fs = require("fs"), zlib = require("zlib");
const b = fs.readFileSync(process.argv[1]);
const gz = zlib.gzipSync(b, { level: 9 }).length;
const br = zlib.brotliCompressSync(b, { params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 } }).length;
console.log(`${b.length} / ${gz} / ${br}`);' "$1"
}
for linker in fix base; do
  if [ "$linker" = base ]; then export JSC_LINK_BIN=$A/main-src/target/release/jsc-link; else unset JSC_LINK_BIN; fi
  P=$A/pkg-$linker; rm -rf "$P"; mkdir -p "$P"
  tag=va6$linker
  node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust-compact/dist/$tag > "$P/compact-build.log" 2>&1
  echo "compact build exit $?" >> "$P/sizes.txt"
  node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust-compact/dist/$tag/fp.esm.min.js > "$P/compact-kernels.log" 2>&1
  echo "compact kernels exit $? $(tail -1 "$P/compact-kernels.log")" >> "$P/sizes.txt"
  echo "compact sizes $(sizes experiments/fingerprintjs-rust-compact/dist/$tag/fp.esm.min.js)" >> "$P/sizes.txt"
  node experiments/fingerprintjs-rust/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust/dist/$tag > "$P/idiomatic-build.log" 2>&1
  echo "idiomatic build exit $?" >> "$P/sizes.txt"
  node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust/dist/$tag/boundary/fp.esm.min.js > "$P/idiomatic-kernels.log" 2>&1
  echo "idiomatic kernels exit $? $(tail -1 "$P/idiomatic-kernels.log")" >> "$P/sizes.txt"
  echo "idiomatic sizes $(sizes experiments/fingerprintjs-rust/dist/$tag/boundary/fp.esm.min.js)" >> "$P/sizes.txt"
  cp -R experiments/fingerprintjs-rust-compact/dist/$tag "$P/compact-dist"
  cp -R experiments/fingerprintjs-rust/dist/$tag "$P/idiomatic-dist"
  echo "pkg $linker done"; cat "$P/sizes.txt"
done
