#!/usr/bin/env bash
# reshim_cmp.sh: re-runs every mine-compiled program with the final shim (arm mineshim4) and compares
# each stdout with the stdout the same program printed under the earlier shim.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
bash $V/bin/reshim.sh mine mineshim4 $W/runtime/shim.js > /dev/null 2>&1
python3 $V/bin/table.py base mineshim4 | tail -3
n=0; d=0
for m in rb rbopt0 rbco jir0 def; do
  for f in $V/out/mine/$m/*.js.out; do
    b=$(basename $f); n=$((n+1))
    if ! cmp -s $f $V/out/mineshim4/$m/$b; then d=$((d+1)); echo "STDOUT DIFF $m $b"; fi
  done
done
echo "compared $n program outputs, $d differ"
