#!/usr/bin/env bash
# natall.sh <src.rs>... : native O0/O3 for each; prints failures with the first errors
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
for f in "$@"; do
  n=$(basename "$f" .rs)
  if ! out=$(bash "$V/tools/native.sh" "$f"); then
    echo "NATIVE FAIL $n: $out" | head -5
    grep -m4 -A8 "^error" "$V/native/$n/build-O0.log" | head -40
  fi
done
echo native-done
