#!/usr/bin/env bash
# native.sh <src.rs>: native -O0 and -O3 with pinned toolchain; prints O3 stdout path
set -uo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
src=$1
name=$(basename "$src" .rs)
d=$V/native/$name
mkdir -p "$d"
python3 -I "$V/tools/nativize.py" "$src" "$d/main.rs" || exit 1
chan=nightly-2026-07-27
for opt in 0 3; do
  rustc +$chan --edition 2021 -C opt-level=$opt -A warnings -o "$d/main-O$opt" "$d/main.rs" 2> "$d/build-O$opt.log" \
    || { echo "NATIVE BUILD FAIL -O$opt"; head -30 "$d/build-O$opt.log"; exit 1; }
  "$d/main-O$opt" > "$d/stdout-O$opt" 2> "$d/stderr-O$opt"; echo "$?" > "$d/exit-O$opt"
done
if cmp -s "$d/stdout-O0" "$d/stdout-O3" && cmp -s "$d/exit-O0" "$d/exit-O3"; then
  echo "$d/stdout-O3"
else
  echo "NATIVE O0/O3 DISAGREE for $name"; diff "$d/stdout-O0" "$d/stdout-O3" | head; exit 1
fi
