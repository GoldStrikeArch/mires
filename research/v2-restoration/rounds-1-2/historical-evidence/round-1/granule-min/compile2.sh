#!/usr/bin/env bash
# compile2.sh <base|r2> <srcdir> <outdir> <name>...: compiles <srcdir>/<name>.rs under
# release-balanced with the chosen jsc-link (base: the saved base binary; r2: the worktree's).
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
which=$1
src=$2
out=$3
shift 3
mkdir -p "$out"
cd "$W" || exit 1
for n in "$@"; do
	if [ "$which" = base ]; then
		JSC_LINK_BIN=$G/base/jsc-link JS_EXTRA_ARGS="js-profile=release-balanced" bash scripts/compile_core.sh "$src/$n.rs" "$out/$n.js" >"$out/$n.compile.log" 2>&1
	else
		JS_EXTRA_ARGS="js-profile=release-balanced" bash scripts/compile_core.sh "$src/$n.rs" "$out/$n.js" >"$out/$n.compile.log" 2>&1
	fi
	echo "$n compile exit $?"
done
