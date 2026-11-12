#!/usr/bin/env bash
# m3b.sh: mutation M3 (item 3 reverted) against the final fixture 122 and r8_cell, default profile and
# release-balanced; then the worktree is restored and rebuilt.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
cd $W
git apply -R $V/item3-ptr.patch || { echo "revert failed"; exit 1; }
bash scripts/build.sh --locked > $V/logs/build-m3b.log 2>&1; echo "m3b build exit=$?"
shasum -a 256 target/release/librustc_codegen_mires.dylib
for m in "js-profile=release-balanced" ""; do
  JS_EXTRA_ARGS="$m" SKIP_BUILD=1 bash scripts/build_sysroot.sh > $V/logs/sysroot-m3b-$(echo "x$m" | tr ' =' '__').log 2>&1; echo "m3b sysroot [$m] exit=$?"
done
bash $V/bin/mx.sh $W m3b def fuse r8_cell
EXPECTED=$W/examples/core-tests/122_pointer_element_moves.expected bash $V/bin/f122.sh $W m3b $W/examples/core-tests/122_pointer_element_moves.rs def rb
git apply $V/item3-ptr.patch || { echo "restore failed"; exit 1; }
git diff --stat
bash scripts/build.sh --locked > $V/logs/build-restore3.log 2>&1; echo "restore build exit=$?"
shasum -a 256 target/release/librustc_codegen_mires.dylib
echo M3B-DONE
