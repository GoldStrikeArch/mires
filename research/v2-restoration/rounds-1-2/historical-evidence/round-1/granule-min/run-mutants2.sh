#!/usr/bin/env bash
# run-mutants2.sh <compiled-dir>: every round-2 shim mutant against the committed
# scripts/runtime-addr.test.mjs (a mirror holding the exact committed test beside the mutated shim)
# and against fixtures 122, 123 and 124 compiled by test.sh (<compiled-dir>/<name>.js), each
# compared with its .expected; the unmutated shim first, as the control.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
compiled=$1
for m in control $(cd "$G/mut2" && ls *.js | sed 's/\.js$//'); do
	dir=$G/tt/mut2-$m
	rm -rf "$dir"
	mkdir -p "$dir/scripts" "$dir/runtime"
	cp "$W/scripts/runtime-addr.test.mjs" "$dir/scripts/"
	if [ "$m" = control ]; then cp "$W/runtime/shim.js" "$dir/runtime/shim.js"; else cp "$G/mut2/$m.js" "$dir/runtime/shim.js"; fi
	echo "== $m $(date +%T)"
	(cd "$dir" && node --test scripts/runtime-addr.test.mjs 2>&1) | grep -E "^✖ |^ℹ (pass|fail)" | grep -v "failing tests" | sort -u
	for f in 122_consumed_block_addresses 123_moved_block_addresses 124_read_block_addresses; do
		if cat "$dir/runtime/shim.js" "$compiled/$f.js" "$W/scripts/entry.js" | node - >"$dir/$f.out" 2>&1 &&
			cmp -s "$dir/$f.out" "$W/examples/core-tests/$f.expected"; then
			echo "fixture $f: pass"
		else
			echo "fixture $f: FAIL ($(tr '\n' ' ' <"$dir/$f.out" | cut -c1-120))"
		fi
	done
done
echo "== done $(date +%T)"
