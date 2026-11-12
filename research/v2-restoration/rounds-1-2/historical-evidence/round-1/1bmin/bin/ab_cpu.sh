#!/usr/bin/env bash
# ab_cpu.sh <rounds> <armA> <armB>: interleaved rounds (order alternating) of every compiled benchmark
# in both lanes and the runtime micro-benchmarks, timed in process CPU time.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
rounds=${1:-9}; A=${2:-base}; Bm=${3:-final}
B=$V/bench; R=$B/results-cpu; mkdir -p $R
out=$R/bench.jsonl; micro=$R/micro.txt; up=$R/uptime.txt
: > $out; : > $micro; : > $up
names_put="bench_field bench_vec bench_swap bench_move bench_through"
names_2="b2_big_field b2_big_through b2_big_vec b2_local_small b2_local_big b2_with_array b2_enum b2_swap_big"
names_3="b3_value_moves b3_int_moves b3_bulk"
run_arm() {
  arm=$1; r=$2; label=$3
  shim=$B/out/$arm-shim.js
  for mode in rb def; do
    node $B/time_cpu.mjs $B/out/$arm-bench_put-$mode.js $shim "$label|$mode|put|r$r" $names_put >> $out
    node $B/time_cpu.mjs $B/out/$arm-bench2-$mode.js $shim "$label|$mode|b2|r$r" $names_2 >> $out
    node $B/time_cpu.mjs $B/out/$arm-bench3-$mode.js $shim "$label|$mode|b3|r$r" $names_3 >> $out
  done
  node $B/micro_cpu.mjs $shim $label >> $micro
}
for r in $(seq 1 $rounds); do
  uptime >> $up
  if [ $((r % 2)) = 1 ]; then run_arm $A $r base; run_arm $Bm $r mine; else run_arm $Bm $r mine; run_arm $A $r base; fi
done
uptime >> $up
node $V/bin/ab_report.mjs $out $micro
sed 's/.*load averages: /load /' $up | tr '\n' ';'; echo
