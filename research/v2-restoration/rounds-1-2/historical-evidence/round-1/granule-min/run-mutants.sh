#!/usr/bin/env bash
# run-mutants.sh: for each mutant shim, runs the committed scripts/runtime-addr.test.mjs against it
# (a mirror holding the exact committed test beside the mutated shim) and fixture 122's compiled
# release-balanced program; then the same for the unmutated shim as the control.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
for m in control a b c d; do
	dir=$G/tt/mut-$m
	rm -rf "$dir"
	mkdir -p "$dir/scripts" "$dir/runtime"
	cp "$W/scripts/runtime-addr.test.mjs" "$dir/scripts/"
	if [ "$m" = control ]; then cp "$W/runtime/shim.js" "$dir/runtime/shim.js"; else cp "$G/mut/$m.js" "$dir/runtime/shim.js"; fi
	echo "== mutant $m $(date +%T)"
	(cd "$dir" && node --test scripts/runtime-addr.test.mjs 2>&1) | grep -E "^✖ |^ℹ (pass|fail)" | sort -u
	echo "fixture 122: $(node "$G/runp.mjs" "$dir/runtime/shim.js" "$G/probe/fix/122-rb.js")"
done
echo "== done $(date +%T)"
