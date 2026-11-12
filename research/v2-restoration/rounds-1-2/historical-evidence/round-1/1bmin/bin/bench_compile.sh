#!/usr/bin/env bash
# bench_compile.sh: compiles bench_put, bench2, bench3 with the base tree and the worktree, rb and default.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
B=$V/bench
for arm in base mine; do
  root=$W; [ $arm = base ] && root=$V/base
  for mode in rb def; do
    args=""; [ $mode = rb ] && args="js-profile=release-balanced"
    for b in bench_put bench2 bench3; do
      (cd $root && JS_EXTRA_ARGS="$args" SKIP_BUILD=1 bash scripts/compile_core.sh $B/$b.rs $B/out/$arm-$b-$mode.js > $B/out/$arm-$b-$mode.log 2>&1)
      echo "compile $arm $b $mode exit=$? $(wc -c < $B/out/$arm-$b-$mode.js) bytes"
    done
  done
done
