#!/usr/bin/env bash
# matrix.sh <tag> <src.rs>... ; env COMPILERS="fix main" MODES="def jiroff rb rbz"
set -uo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
tag=$1; shift
out=$V/out/$tag
mkdir -p "$out"
modes=${MODES:-def jiroff rb rbz}
compilers=${COMPILERS:-fix}
for src in "$@"; do
  name=$(basename "$src" .rs)
  nat=$(bash "$V/tools/native.sh" "$src") || { echo "$name NATIVE-FAIL: $nat" | tee -a "$out/summary.txt"; continue; }
  for compiler in $compilers; do
    root=${ROOT:-$V/fixroot}
    [ "$compiler" = main ] && root=${MAINROOT:-$V/mainroot}
    for mode in $modes; do
      bash "$V/tools/mires.sh" "$root" "$src" "$mode" "$out/$compiler"
      base=$out/$compiler/$name.$mode
      if [ "$(cat "$base.exit")" = 0 ] && cmp -s "$nat" "$base.stdout"; then
        echo "$name $compiler $mode PASS" | tee -a "$out/summary.txt"
      else
        diff "$nat" "$base.stdout" > "$base.diff"
        echo "$name $compiler $mode FAIL exit=$(cat "$base.exit") $(grep -c '^[<>]' "$base.diff") diff-lines; $(grep -m1 -E '^error' "$base.log" | cut -c1-260) $(head -c 200 "$base.stderr" | tr '\n' ' ')" | tee -a "$out/summary.txt"
      fi
    done
  done
done
