#!/usr/bin/env bash
# gates.sh: the remaining COMMON.md gates and the other suites that link, each run from the
# worktree with every link done by both linkers (tools/abrun.sh). One row per gate in
# gates/summary.tsv: name, the suite's exit status, the per-link comparison counts.
set -uo pipefail
source "$(dirname "$0")/env.sh"
G=$V/gates
mkdir -p "$G"
gate() { # gate <name> <command...>
  local name=$1; shift
  bash "$V/tools/abrun.sh" "gate-$name" "$@" > "$G/$name.line" 2>&1
  local links
  links=$(awk '{print $1}' "$V/ab2/gate-$name/ab.log" 2>/dev/null | sort | uniq -c | tr '\n' ' ')
  printf '%s\t%s\t%s\n' "$name" "$(tail -1 "$V/ab2/gate-$name/suite.log")" "${links:-no links}" | tee -a "$G/summary.tsv"
}
: > "$G/summary.tsv"
gate emit env SKIP_BUILD=1 bash scripts/emit-test.sh
gate chunks-jiropt env SKIP_BUILD=1 JS_EXTRA_ARGS='js-lane=jir js-opt=on' bash scripts/chunk-test.sh
gate chunks-def env SKIP_BUILD=1 bash scripts/chunk-test.sh
gate chunks-rb env SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced' bash scripts/chunk-test.sh
gate active-poll env SKIP_BUILD=1 bash scripts/active-poll-cleanup-test.sh
gate extern env SKIP_BUILD=1 bash scripts/extern-test.sh
gate artifact-manifest env SKIP_BUILD=1 bash scripts/artifact-manifest-test.sh
gate es2018 env SKIP_BUILD=1 bash scripts/es2018-test.sh
gate flat-result env SKIP_BUILD=1 bash scripts/flat-result-test.sh
gate standalone node scripts/standalone-check.mjs
gate known-gaps python3 scripts/check-known-gaps.py
gate round1-open python3 scripts/check-round1-open.py
gate round1-open-oz-inline env JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on python3 scripts/check-round1-open.py
gate pkg-compact node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust-compact/dist/va6b
gate pkg-idiomatic node experiments/fingerprintjs-rust/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust/dist/va6b
cd "$W" || exit 2
sizes() {
  node -e '
const fs = require("fs"), zlib = require("zlib");
const b = fs.readFileSync(process.argv[1]);
const gz = zlib.gzipSync(b, { level: 9 }).length;
const br = zlib.brotliCompressSync(b, { params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 } }).length;
console.log(`${b.length} / ${gz} / ${br}`);' "$1"
}
{
  node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust-compact/dist/va6b/fp.esm.min.js > "$G/compact-kernels.log" 2>&1
  echo "compact kernels exit $? $(tail -1 "$G/compact-kernels.log")"
  echo "compact sizes $(sizes experiments/fingerprintjs-rust-compact/dist/va6b/fp.esm.min.js)"
  node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust/dist/va6b/boundary/fp.esm.min.js > "$G/idiomatic-kernels.log" 2>&1
  echo "idiomatic kernels exit $? $(tail -1 "$G/idiomatic-kernels.log")"
  echo "idiomatic sizes $(sizes experiments/fingerprintjs-rust/dist/va6b/boundary/fp.esm.min.js)"
} > "$G/packages.txt" 2>&1
cat "$G/packages.txt"
echo gates-done >> "$G/summary.tsv"
