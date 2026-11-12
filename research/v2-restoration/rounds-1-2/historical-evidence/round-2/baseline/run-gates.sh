#!/usr/bin/env bash
# Runs the round-2 gate set once in a given checkout and records one log and exit code per gate.
#   run-gates.sh <repo-root> <out-dir>
set -uo pipefail
root=${1:?repo root}
out=${2:?output dir}
mkdir -p "$out"
cd "$root" || exit 2
summary="$out/summary.tsv"
: >"$summary"
{
	echo "head $(git rev-parse HEAD)"
	echo "dirty $(git status --short | wc -l | tr -d ' ')"
	echo "start $(date -u +%FT%TZ)"
	echo "load $(uptime)"
} >"$out/identity.txt"

gate() {
	local name=$1
	shift
	local start=$SECONDS
	"$@" >"$out/$name.log" 2>&1
	local code=$?
	printf '%s\t%s\t%ss\n' "$name" "$code" "$((SECONDS - start))" >>"$summary"
}

gate build bash scripts/build.sh --locked
gate check bash scripts/check.sh
gate fixtures env SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced js-minify=on' bash scripts/test.sh --require-core
gate active-poll env SKIP_BUILD=1 bash scripts/active-poll-cleanup-test.sh
gate cleanup-transport env SKIP_BUILD=1 python3 scripts/cleanup-transport-test.py
gate cleanup-admission env SKIP_BUILD=1 bash scripts/cleanup-admission-test.sh
gate extern env SKIP_BUILD=1 bash scripts/extern-test.sh
gate standalone node scripts/standalone-check.mjs
gate known-gaps python3 scripts/check-known-gaps.py
cp build/known-gaps/receipt.json "$out/known-gaps.receipt.json" 2>/dev/null
gate round1-open python3 scripts/check-round1-open.py
cp build/round1-open/receipt.json "$out/round1-open.receipt.json" 2>/dev/null
gate round1-open-oz-inline env JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on python3 scripts/check-round1-open.py
cp build/round1-open/receipt.json "$out/round1-open-oz-inline.receipt.json" 2>/dev/null
echo "end $(date -u +%FT%TZ)" >>"$out/identity.txt"
echo "load-end $(uptime)" >>"$out/identity.txt"
