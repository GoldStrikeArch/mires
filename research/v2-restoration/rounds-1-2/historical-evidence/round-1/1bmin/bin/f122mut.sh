#!/usr/bin/env bash
# f122mut.sh: re-runs the mine-compiled fixture 122 (every mode) under each runtime mutant shim.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
exp=$W/examples/core-tests/122_pointer_element_moves.expected
for m in M1_never_rebind M2_rebind_boxes M2b_rebind_scaled M2c_rebind_always; do
  line="$m:"
  for mode in rb rbopt0 rbco jir0 def; do
    js=$V/f122/final-$mode.js
    cat $V/mutants/$m.shim.js $js $W/scripts/entry.js | node - > $V/f122/mut-$m-$mode.out 2>/dev/null; st=$?
    line="$line $mode=$(diff $exp $V/f122/mut-$m-$mode.out | grep -c '^<')wrong(rc$st)"
  done
  echo "$line"
done
