#!/usr/bin/env bash
# final.sh: every correctness run again with the final runtime, sequentially.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
cd $W
shasum -a 256 target/release/librustc_codegen_mires.dylib runtime/shim.js
bash $V/bin/matrix.sh $W final
python3 $V/bin/table.py base final > $V/logs/table-base-vs-final.txt; tail -4 $V/logs/table-base-vs-final.txt
EXPECTED=$W/examples/core-tests/122_pointer_element_moves.expected bash $V/bin/f122.sh $W final $W/examples/core-tests/122_pointer_element_moves.rs rb rbopt0 rbco jir0 def
sed -i '' 's|\$V/f122/mine-\$mode.js|$V/f122/final-$mode.js|' $V/bin/f122mut.sh
bash $V/bin/f122mut.sh
python3 scripts/check-known-gaps.py > $V/logs/known-gaps-final.log 2>&1; echo "known gaps exit=$?"; cat $V/logs/known-gaps-final.log | tail -6
node --test scripts/runtime-*.test.mjs scripts/native-values.test.mjs scripts/minimal-shim.test.mjs > $V/logs/gate-node-final.log 2>&1; echo "node gate exit=$?"; grep -E "^ℹ (tests|pass|fail)" $V/logs/gate-node-final.log
bash $V/bin/suites.sh $W final
bash $V/bin/gates.sh $W final
echo FINAL-DONE
