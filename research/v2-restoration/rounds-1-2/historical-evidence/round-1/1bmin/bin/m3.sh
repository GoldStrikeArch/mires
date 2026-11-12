#!/usr/bin/env bash
# m3.sh: mutation M3 (item 3 reverted: `ref_to_raw` back to spelling-based slots), built in the
# worktree; then the worktree is restored and rebuilt.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
cd $W
git apply -R $V/item3-ptr.patch || { echo "revert failed"; exit 1; }
bash scripts/build.sh --locked > $V/logs/build-m3.log 2>&1; echo "m3 build exit=$?"
shasum -a 256 target/release/librustc_codegen_mires.dylib $V/base/target/release/librustc_codegen_mires.dylib
for m in "js-profile=release-balanced" ""; do
  JS_EXTRA_ARGS="$m" SKIP_BUILD=1 bash scripts/build_sysroot.sh > $V/logs/sysroot-m3-$(echo "x$m" | tr ' =' '__').log 2>&1; echo "m3 sysroot [$m] exit=$?"
done
bash $V/bin/mx.sh $W m3 def fuse r8_cell
bash $V/bin/mx.sh $W m3 rb r3new k_ptrptr b_mixed_struct
EXPECTED=$W/examples/core-tests/122_pointer_element_moves.expected bash $V/bin/f122.sh $W m3 $W/examples/core-tests/122_pointer_element_moves.rs def rb
git apply $V/item3-ptr.patch || { echo "restore failed"; exit 1; }
git diff --stat
bash scripts/build.sh --locked > $V/logs/build-restore.log 2>&1; echo "restore build exit=$?"
shasum -a 256 target/release/librustc_codegen_mires.dylib
echo M3-DONE
