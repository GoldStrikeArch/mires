#!/usr/bin/env bash
# packages.sh <label>: builds the fingerprint compact package and the browser workloads with the
# worktree's current compiler and shim, and copies both outputs to scratch/<label>/.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
label=$1
out=$G/$label
mkdir -p "$out"
cd "$W" || exit 1
echo "== fingerprint $(date +%T)"
node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on \
	--lane jir --cleanup on --scheduler boundary --lane-report \
	--output "experiments/fingerprintjs-rust-compact/dist/granule-min-$label" >"$out/fingerprint-build.log" 2>&1
echo "fingerprint exit $?"
rm -rf "$out/fp-dist"
cp -R "experiments/fingerprintjs-rust-compact/dist/granule-min-$label" "$out/fp-dist"
echo "== workloads $(date +%T)"
node experiments/browser-workloads/tools/build.mjs >"$out/workloads-build.log" 2>&1
echo "workloads exit $?"
rm -rf "$out/wl-dist" "$out/wl-build.json"
cp -R experiments/browser-workloads/dist "$out/wl-dist"
cp experiments/browser-workloads/results/build.json "$out/wl-build.json"
echo "== done $(date +%T)"
