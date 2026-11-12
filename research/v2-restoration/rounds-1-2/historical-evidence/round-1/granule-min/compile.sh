#!/usr/bin/env bash
# compile.sh <base|new> <outdir-name> <probe-name>...: compiles probes under release-balanced with
# the chosen jsc-link (base: the saved base binary; new: the worktree's) and censuses them.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
which=$1
out=$G/probe/$2
shift 2
mkdir -p "$out"
cd "$W" || exit 1
for n in "$@"; do
	if [ "$which" = base ]; then
		JSC_LINK_BIN=$G/base/jsc-link JS_EXTRA_ARGS="js-profile=release-balanced" bash scripts/compile_core.sh "$G/probe/$n.rs" "$out/$n.js" >"$out/$n.compile.log" 2>&1
	else
		JS_EXTRA_ARGS="js-profile=release-balanced" bash scripts/compile_core.sh "$G/probe/$n.rs" "$out/$n.js" >"$out/$n.compile.log" 2>&1
	fi
	echo "$n compile exit $?"
done
for n in "$@"; do node "$G/census.mjs" "$out/$n.js" | head -1; done
