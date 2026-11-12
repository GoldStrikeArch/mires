#!/usr/bin/env bash
# build-rust-mutants.sh: for each linker-side mutant, apply it, build jsc-link into a scratch
# target directory (never the worktree's target/), keep the binary, and revert the source.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
cd "$W" || exit 1
for m in r1-call-arg-at-drop r2-drop-cut-not-released m2c-from-utf8-unchecked-releases m2d-str-from-raw-parts-releases; do
	echo "== $m $(date +%T)"
	python3 "$G/rust_mutant.py" apply "$m" || exit 1
	cargo build --release --locked -p jsc-link --target-dir "$G/mt" >"$G/mt-$m.log" 2>&1
	status=$?
	python3 "$G/rust_mutant.py" revert "$m"
	echo "build exit $status"
	[ "$status" = 0 ] && cp "$G/mt/release/jsc-link" "$G/mt/jsc-link.$m"
done
git -C "$W" diff --stat
echo "== done $(date +%T)"
