#!/usr/bin/env bash
# final2.sh: the correctness runs again with the final runtime. The four JIR-lane modes are
# recompiled (their output depends on the runtime through the representation planner's size
# model); the default profile is recompiled for a sample, checked byte-identical to the earlier
# compile, and otherwise re-run on the earlier compile.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
cd $W
shasum -a 256 target/release/librustc_codegen_mires.dylib runtime/shim.js
bash $V/bin/matrix2.sh $W final3 rb rbopt0 rbco jir0
# default profile: sample recompiles
mkdir -p $V/out/final3def/def
for n in r1/ref_moves fuse/r8_cell r3new/a_ptr_places r3new/k_ptrptr fuse/m1_moves r1/field_borrow_local; do
  set_=${n%%/*}; f=${n#*/}
  bash $V/bin/mx.sh $W final3def def $set_ $f > /dev/null
  if cmp -s $V/out/final3def/def/$set_-$f.js $V/out/mine/def/$set_-$f.js; then echo "def recompile $n identical to the earlier compile"; else echo "def recompile $n DIFFERS"; fi
done
mkdir -p $V/out/final3/def
for rc in $V/out/mine/def/*.js.rc; do js=${rc%.rc}; b=$(basename $js); cp $js $V/out/final3/def/$b 2>/dev/null; cp $rc $V/out/final3/def/$b.rc; done
bash $V/bin/reshim.sh mine final3 $W/runtime/shim.js def > /dev/null
python3 $V/bin/table.py base final3 > $V/logs/table-base-vs-final3.txt; tail -4 $V/logs/table-base-vs-final3.txt
EXPECTED=$W/examples/core-tests/122_pointer_element_moves.expected bash $V/bin/f122.sh $W final3 $W/examples/core-tests/122_pointer_element_moves.rs rb rbopt0 rbco jir0 def
bash $V/bin/f122mut3.sh
python3 scripts/check-known-gaps.py > $V/logs/known-gaps-final3.log 2>&1; echo "known gaps exit=$?"; tail -6 $V/logs/known-gaps-final3.log
node --test scripts/runtime-*.test.mjs scripts/native-values.test.mjs scripts/minimal-shim.test.mjs > $V/logs/gate-node-final3.log 2>&1; echo "node gate exit=$?"; grep -E "^ℹ (tests|pass|fail)" $V/logs/gate-node-final3.log
bash $V/bin/suites.sh $W final3
bash $V/bin/gates.sh $W final3
echo FINAL3-DONE
echo FINAL3-ALL-DONE
