#!/usr/bin/env bash
# chunkrun.sh <tag> <probe> <calls> <order>... : one probe split into chunks, each <order> being a
# comma-separated chunk:entry list as REQUESTED; every build is RUN calling <calls> (comma list, the
# order rust_entry calls them), in modes $CMODES, against native. EXTRA adds backend options.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
tag=$1; name=$2; calls=$(printf '%s' "$3" | tr ',' ' '); shift 3
out=$A/out/$tag
mkdir -p "$out"
cmodes=${CMODES:-def rb rbnc rbz}
src=$A/src/$name.rs
nat=$(bash "$A/tools/native.sh" "$src") || { echo "$name NATIVE-FAIL: $nat" | tee -a "$out/summary.txt"; exit 1; }
n=0
for order in "$@"; do
  n=$((n + 1))
  pairs=$(printf '%s' "$order" | tr ',' ' ')
  for mode in $cmodes; do
    # shellcheck disable=SC2086
    TAG=o$n CALLS="$calls" bash "$A/tools/mires.sh" "$src" "$mode" "$out" $pairs
    base=$out/$name.$mode.o$n
    if [ "$(cat "$base.exit")" = 0 ] && cmp -s "$nat" "$base.stdout"; then
      echo "$name[o$n $order] $mode PASS" | tee -a "$out/summary.txt"
    else
      diff "$nat" "$base.stdout" > "$base.diff"
      echo "$name[o$n $order] $mode FAIL exit=$(cat "$base.exit") $(grep -c '^[<>]' "$base.diff") diff-lines; $(grep -m1 -E '^error' "$base.log" | cut -c1-300) $(grep -m1 -E 'Error' "$base.stderr" | cut -c1-200)" | tee -a "$out/summary.txt"
    fi
  done
done
