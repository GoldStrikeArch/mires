#!/usr/bin/env bash
# ab.sh <rounds>: interleaved base/mine rounds (order alternates per round) of every compiled
# benchmark in both lanes, plus ptr_micro.mjs and moves_micro.mjs on the two shims.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
S=$V/..
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
rounds=${1:-7}
B=$V/bench; R=$B/results; mkdir -p $R
out=$R/bench.jsonl; micro=$R/micro.txt; up=$R/uptime.txt
: > $out; : > $micro; : > $up
names_put="bench_field bench_vec bench_swap bench_move bench_through"
names_2="b2_big_field b2_big_through b2_big_vec b2_local_small b2_local_big b2_with_array b2_enum b2_swap_big"
names_3="b3_value_moves b3_int_moves b3_bulk"
run_arm() {
  arm=$1; r=$2
  shim=$B/out/$arm-shim.js
  for mode in rb def; do
    BENCH_N=200000 node $V/bench/time_n.mjs $B/out/$arm-bench_put-$mode.js $shim "$arm|$mode|put|r$r" $names_put >> $out
    BENCH_N=200000 node $V/bench/time_n.mjs $B/out/$arm-bench2-$mode.js $shim "$arm|$mode|b2|r$r" $names_2 >> $out
    BENCH_N=200000 node $V/bench/time_n.mjs $B/out/$arm-bench3-$mode.js $shim "$arm|$mode|b3|r$r" $names_3 >> $out
  done
  node $S/verify1b/ptr_micro.mjs $shim $arm >> $micro
  node $V/bench/moves_micro.mjs $shim $arm >> $micro
}
for r in $(seq 1 $rounds); do
  uptime >> $up
  if [ $((r % 2)) = 1 ]; then run_arm base $r; run_arm mine $r; else run_arm mine $r; run_arm base $r; fi
done
uptime >> $up
node $V/bin/ab_report.mjs $out $micro
sed 's/.*load averages: /load /' $up | tr '\n' ';'; echo
