#!/usr/bin/env bash
# micro_main_ab.sh <rounds> <out> <armA> <armB>: alternating rounds of the main-context element-move
# micro (moves_micro_main.mjs) and the vm-context pointer micro (ptr_micro.mjs).
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
S=$V/..
rounds=$1; out=$2; A=$3; Bm=$4
B=$V/bench/out
: > $out
for r in $(seq 1 $rounds); do
  if [ $((r % 2)) = 1 ]; then order="$A $Bm"; else order="$Bm $A"; fi
  for a in $order; do
    node $V/bench/moves_micro_main.mjs $B/$a-shim.js $a >> $out
    node $S/verify1b/ptr_micro.mjs $B/$a-shim.js $a >> $out
  done
done
uptime
