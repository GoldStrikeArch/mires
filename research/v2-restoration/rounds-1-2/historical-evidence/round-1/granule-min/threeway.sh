#!/usr/bin/env bash
# threeway.sh <fixture.rs> <name>: compiles one source under release-balanced with the base,
# round-1 and round-2 linkers, runs each under its own shim, and compares with the native output
# (probe/nat/<name>.3.out). Also lists every consuming-conversion site in each program.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
src=$1
name=$2
out=$G/probe/three
mkdir -p "$out"
cd "$W" || exit 1
for which in base round1 round2; do
	case $which in
	base) bin=$G/base/jsc-link shim=$G/base/shim.js ;;
	round1) bin=$G/new/jsc-link.before-doc shim=$G/new/shim.js ;;
	round2) bin=$W/target/release/jsc-link shim=$W/runtime/shim.js ;;
	esac
	JSC_LINK_BIN=$bin JS_EXTRA_ARGS="js-profile=release-balanced" bash scripts/compile_core.sh "$src" \
		"$out/$name.$which.js" >"$out/$name.$which.log" 2>&1 || { echo "$which: compile failed"; continue; }
	got=$(node "$G/runp.mjs" "$shim" "$out/$name.$which.js" | sed 's/^[^:]*: //')
	want=$(tr '\n' ' ' <"$G/probe/nat/$name.3.out" | sed 's/ $//')
	if [ "$got" = "$want" ]; then verdict=ok; else verdict=BAD; fi
	echo "$which $verdict ($(wc -c <"$out/$name.$which.js" | tr -d ' ') bytes): $got"
	node "$G/sites.mjs" "$out/$name.$which.js" | sed 's/^/    /'
done
echo "native: $(tr '\n' ' ' <"$G/probe/nat/$name.3.out")"
