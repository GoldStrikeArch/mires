#!/usr/bin/env bash
# gates2.sh: the remaining repository gates, serially, each into its own log.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
cd "$W" || exit 1
echo "== extern-test $(date +%T)"
SKIP_BUILD=1 bash scripts/extern-test.sh >"$G/extern-test.log" 2>&1
echo "extern-test exit $?"
tail -3 "$G/extern-test.log"
echo "== cleanup-smoke $(date +%T)"
SKIP_BUILD=1 bash scripts/cleanup-smoke.sh >"$G/cleanup-smoke.log" 2>&1
echo "cleanup-smoke exit $?"
tail -3 "$G/cleanup-smoke.log"
echo "== known-gaps $(date +%T)"
python3 scripts/check-known-gaps.py >"$G/known-gaps.log" 2>&1
echo "known-gaps exit $?"
tail -12 "$G/known-gaps.log"
echo "== es2018 $(date +%T)"
SKIP_BUILD=1 ES2018_PARSER=/Users/mikhail.pertsev/Desktop/OSS/mires/node_modules/acorn/dist/acorn.mjs bash scripts/es2018-test.sh >"$G/es2018-test.log" 2>&1
echo "es2018 exit $?"
tail -3 "$G/es2018-test.log"
echo "== done $(date +%T)"
