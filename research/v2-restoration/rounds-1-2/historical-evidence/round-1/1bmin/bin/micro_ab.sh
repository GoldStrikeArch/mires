#!/usr/bin/env bash
# micro_ab.sh <rounds> <arms...>: rotates the arms' shims through moves_micro.mjs and ptr_micro.mjs.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
S=$V/..
rounds=$1; shift
arms="$*"
out=$V/bench/results/micro-ab.txt; : > $out
for r in $(seq 1 $rounds); do
  # rotate the order each round
  set -- $arms
  for k in $(seq 1 $(( (r - 1) % $# ))); do first=$1; shift; set -- "$@" $first; done
  for arm in "$@"; do
    node $V/bench/moves_micro.mjs $V/bench/out/$arm-shim.js $arm >> $out
    node $S/verify1b/ptr_micro.mjs $V/bench/out/$arm-shim.js $arm >> $out
  done
done
uptime >> $V/bench/results/micro-ab.uptime
echo MICRO-AB-DONE
