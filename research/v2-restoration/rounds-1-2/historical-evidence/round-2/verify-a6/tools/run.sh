#!/usr/bin/env bash
# run.sh <tag> <probe>... : each probe (src/<probe>.rs) in every mode against native (O0 == O3).
# A probe spec may carry chunk pairs: name|a:chunk_a,b:chunk_b  (then it is ALSO run chunked).
# LINK=base uses main's linker. MODES / CMODES override the mode lists.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
tag=$1; shift
out=$A/out/$tag
mkdir -p "$out"
modes=${MODES:-def jiroff rb rbz}
cmodes=${CMODES:-def rb rbnc rbz}
row() {
  local name=$1 mode=$2 base=$3 nat=$4
  if [ "$(cat "$base.exit")" = 0 ] && cmp -s "$nat" "$base.stdout"; then
    echo "$name $mode PASS" | tee -a "$out/summary.txt"
  else
    diff "$nat" "$base.stdout" > "$base.diff"
    echo "$name $mode FAIL exit=$(cat "$base.exit") $(grep -c '^[<>]' "$base.diff") diff-lines; $(grep -m1 -E '^error' "$base.log" | cut -c1-260) $(grep -m1 -E 'Error' "$base.stderr" | cut -c1-200)" | tee -a "$out/summary.txt"
  fi
}
for spec in "$@"; do
  name=${spec%%|*}; rest=""
  case "$spec" in *"|"*) rest=${spec#*|};; esac
  src=$A/src/$name.rs
  nat=$(bash "$A/tools/native.sh" "$src") || { echo "$name NATIVE-FAIL: $nat" | tee -a "$out/summary.txt"; continue; }
  if [ -z "${ONLY_CHUNKS:-}" ]; then
    for mode in $modes; do
      bash "$A/tools/mires.sh" "$src" "$mode" "$out"
      row "$name" "$mode" "$out/$name.$mode" "$nat"
    done
  fi
  if [ -n "$rest" ]; then
    pairs=$(printf '%s' "$rest" | tr ',' ' ')
    for mode in $cmodes; do
      # shellcheck disable=SC2086
      TAG=chunks bash "$A/tools/mires.sh" "$src" "$mode" "$out" $pairs
      row "$name[chunks]" "$mode" "$out/$name.$mode.chunks" "$nat"
    done
  fi
done
