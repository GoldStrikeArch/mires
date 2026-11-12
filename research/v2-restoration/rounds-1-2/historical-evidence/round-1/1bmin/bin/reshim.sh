#!/usr/bin/env bash
# reshim.sh <from-arm> <to-arm> <shim.js> [modes...]: re-runs every program compiled for <from-arm> with another runtime shim.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
from=$1; to=$2; shim=$3; shift 3
modes=${*:-rb rbopt0 rbco jir0 def}
for m in $modes; do
  mkdir -p $V/out/$to/$m
  for rc in $V/out/$from/$m/*.js.rc; do
    js=${rc%.rc}
    b=$(basename $js .js); set=${b%%-*}; n=${b#*-}
    t=$V/out/$to/$m/$b.js
    rm -f $t.out $t.err $t.rc
    if [ "$(cat $js.rc 2>/dev/null)" = COMPILE-FAILED ]; then echo COMPILE-FAILED > $t.rc; continue; fi
    cp $js $t
    if grep -q 'fn host_throw' $V/corpus/$set/$n.rs; then node $V/../verify1b-r3/cleanup_check.mjs $t $shim > $t.out 2> $t.err; st=$?
    else cat $shim $t $W/scripts/entry.js | node - > $t.out 2> $t.err; st=$?; fi
    echo $st > $t.rc
  done
done
echo RESHIM-DONE
