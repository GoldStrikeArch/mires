#!/usr/bin/env bash
# acc-run.sh: the 3.1-million-round acceptance probes, new build with the new shim and base build
# with the base shim, with how far the base got before it aborted.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
for n in acc_vec acc_str; do
	echo "== $n new $(date +%T)"
	/usr/bin/time -p node "$G/runp.mjs" "$W/runtime/shim.js" "$G/probe/js-new/$n.js" 2>&1
	echo "== $n base $(date +%T)"
	/usr/bin/time -p node "$G/runp.mjs" "$G/base/shim.js" "$G/probe/js-base/$n.js" 2>&1
done
echo "== done $(date +%T)"
